//! Aggregation of the public JSON format. No replay is needed for saved traces.
use anyhow::{Context, Result, ensure};
use clap::Args;
use frame_trace_decode::{AnnotatedBlock, AnnotatedOp};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct ReportArgs {
    /// Saved block --json files.
    #[arg(value_name="BLOCK.json", required_unless_present_any=["rpc","from_json"], conflicts_with="rpc")]
    files: Vec<PathBuf>,
    /// Additional saved block --json files.
    #[arg(long, value_name = "BLOCK.json", conflicts_with = "rpc")]
    from_json: Vec<PathBuf>,
    /// Replay finalized blocks using shared proof/runtime/metadata caches.
    #[arg(long, conflicts_with_all=["files", "from_json"])]
    rpc: Option<String>,
    #[arg(long, requires = "rpc", conflicts_with = "last")]
    from: Option<u64>,
    #[arg(long, requires = "from", conflicts_with = "last")]
    to: Option<u64>,
    /// Most recent K blocks anchored to one finalized head.
    #[arg(long, requires="rpc", conflicts_with_all=["from", "to"], value_parser=clap::value_parser!(u32).range(1..=10000))]
    last: Option<u32>,
    /// Concurrent block replays (proof request concurrency is separate).
    #[arg(long, default_value_t=2, value_parser=clap::value_parser!(u8).range(1..=4))]
    concurrency: u8,
    #[arg(long, default_value_t=8, value_parser=clap::value_parser!(u8).range(1..=64))]
    proof_concurrency: u8,
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 1073741824)]
    cache_max_bytes: u64,
    #[arg(long, default_value_t = 15)]
    top: usize,
    /// Aggregated JSON (- sends tables to stderr).
    #[arg(long)]
    json: Option<PathBuf>,
}

pub fn read_trace(path: &Path) -> Result<AnnotatedBlock> {
    let block: AnnotatedBlock = serde_json::from_reader(
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing block JSON {}", path.display()))?;
    ensure!(
        block.call.is_none(),
        "{} is a call trace; expected block --json",
        path.display()
    );
    Ok(block)
}

pub fn run(args: ReportArgs) -> Result<()> {
    let mut blocks = Vec::new();
    if let Some(url) = &args.rpc {
        use frame_trace_replay::{ReplaySession, rpc::Rpc};
        let rpc = Rpc::with_options(
            url,
            args.proof_concurrency as usize,
            std::time::Duration::from_secs(30),
            Default::default(),
        )?;
        let session =
            ReplaySession::with_cache_max_bytes(args.cache_dir.as_deref(), args.cache_max_bytes)?;
        let head = rpc.call("chain_getFinalizedHead", serde_json::json!([]))?;
        let header = rpc.call("chain_getHeader", serde_json::json!([head]))?;
        let finalized = u64::from_str_radix(
            header["number"]
                .as_str()
                .context("header number")?
                .trim_start_matches("0x"),
            16,
        )?;
        let (from, to) = if let Some(last) = args.last {
            (
                finalized.saturating_sub(u64::from(last) - 1).max(1),
                finalized,
            )
        } else {
            (
                args.from.context("--rpc needs --from/--to or --last")?,
                args.to.context("--from needs --to")?,
            )
        };
        ensure!(
            from >= 1 && from <= to && to <= finalized,
            "range must be ordered, non-genesis and finalized (head #{finalized})"
        );
        ensure!(to - from < 10000, "range limit is 10000 blocks");
        // Finalized number lookups are immutable. Pin each hash before scheduling replay.
        let hashes: Vec<_> = (from..=to)
            .map(|n| {
                let hash = rpc.call("chain_getBlockHash", serde_json::json!([n]))?;
                Ok(hash.as_str().context("block hash unavailable")?.to_owned())
            })
            .collect::<Result<_>>()?;
        for chunk in hashes.chunks(args.concurrency as usize) {
            let results = std::thread::scope(|scope| {
                let jobs: Vec<_> = chunk
                    .iter()
                    .map(|hash| {
                        // Share transport/pool limits, isolate acquisition counters from
                        // the other worker's strict zero-RPC measured execution.
                        let rpc = rpc.fork();
                        let session = &session;
                        scope.spawn(move || {
                            frame_trace_replay::online_with_session(&rpc, hash, session)
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .map(|j| -> Result<_> {
                        Ok(j.join()
                            .map_err(|_| anyhow::anyhow!("report replay worker panicked"))??)
                    })
                    .collect::<Result<Vec<_>>>()
            })?;
            for replay in results {
                let loaded = crate::source::replay_loaded(replay, None)?;
                let mut block =
                    frame_trace_decode::annotate(&loaded.traced, loaded.metadata.as_ref());
                block.block_number = loaded.block_number;
                block.execution = loaded.execution;
                blocks.push(block);
            }
        }
    } else {
        for path in args.files.iter().chain(&args.from_json) {
            blocks.push(read_trace(path)?);
        }
    }
    ensure!(!blocks.is_empty(), "provide at least one block JSON file");
    let mut seen = BTreeSet::new();
    ensure!(
        blocks.iter().all(|b| seen.insert(&b.block_hash)),
        "duplicate block hash in report input"
    );
    let report = aggregate(&blocks);
    if let Some(path) = &args.json {
        let json = serde_json::to_string_pretty(&report)?;
        if path.as_os_str() == "-" {
            println!("{json}");
        } else {
            std::fs::write(path, json)?;
        }
    }
    let tables = render(&report, args.top);
    if args.json.is_some_and(|p| p.as_os_str() == "-") {
        eprint!("{tables}");
    } else {
        print!("{tables}");
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Tally {
    pub ops: usize,
    pub reads: usize,
    pub overlay: usize,
    pub backend_warm: usize,
    pub backend_cold: usize,
    pub origin_unknown: usize,
    pub writes: usize,
    pub iterations: usize,
    pub bytes_read: usize,
    pub bytes_written: usize,
    pub proof_bytes: usize,
    pub proof_ops_observed: usize,
    pub duplicates: usize,
    pub duplicate_overlay: usize,
    pub duplicate_backend_warm: usize,
    pub duplicate_backend_cold: usize,
    pub duplicate_origin_unknown: usize,
    pub storage_ns: u64,
}
impl Tally {
    pub fn of<'a>(ops: impl IntoIterator<Item = &'a AnnotatedOp>) -> Self {
        let mut tally = Self::default();
        let mut intervals = Vec::new();
        for op in ops {
            tally.ops += 1;
            if let Some(bytes) = op.proof_bytes_added {
                tally.proof_bytes += bytes;
                tally.proof_ops_observed += 1;
            }
            if let Some(t) = op.timing {
                intervals.push((t.start_ns, t.start_ns.saturating_add(t.duration_ns)));
            }
            if is_read(op) {
                tally.reads += 1;
                tally.bytes_read += op.value_len.unwrap_or(0);
                match op.origin.as_deref() {
                    Some("overlay") => tally.overlay += 1,
                    Some("backend_warm") => tally.backend_warm += 1,
                    Some("backend_cold") => tally.backend_cold += 1,
                    _ => tally.origin_unknown += 1,
                }
                if op.duplicate_read {
                    tally.duplicates += 1;
                    match op.origin.as_deref() {
                        Some("overlay") => tally.duplicate_overlay += 1,
                        Some("backend_warm") => tally.duplicate_backend_warm += 1,
                        Some("backend_cold") => tally.duplicate_backend_cold += 1,
                        _ => tally.duplicate_origin_unknown += 1,
                    }
                }
            } else if matches!(
                op.op.as_str(),
                "put" | "clear" | "append" | "clear_prefix" | "kill_child"
            ) {
                tally.writes += 1;
                tally.bytes_written += op.value_len.unwrap_or(0);
            } else if matches!(op.op.as_str(), "next_key" | "iteration") {
                tally.iterations += 1;
            }
        }
        tally.storage_ns = union_ns(intervals);
        tally
    }
    fn add(&mut self, other: &Self) {
        macro_rules! add { ($($field:ident),*) => { $(self.$field += other.$field;)* }; }
        add!(
            ops,
            reads,
            overlay,
            backend_warm,
            backend_cold,
            origin_unknown,
            writes,
            iterations,
            bytes_read,
            bytes_written,
            proof_bytes,
            proof_ops_observed,
            duplicates,
            duplicate_overlay,
            duplicate_backend_warm,
            duplicate_backend_cold,
            duplicate_origin_unknown,
            storage_ns
        );
    }
}
pub fn is_read(op: &AnnotatedOp) -> bool {
    matches!(op.op.as_str(), "get" | "exists" | "hash")
}
pub fn union_ns(mut intervals: Vec<(u64, u64)>) -> u64 {
    intervals.sort_unstable();
    let mut end = 0;
    let mut total = 0;
    for (lo, hi) in intervals {
        total += hi.saturating_sub(lo.max(end));
        end = end.max(hi);
    }
    total
}
#[derive(Serialize, Default)]
pub struct PhaseAggregate {
    pub tally: Tally,
    pub ops_share: f64,
    pub storage_time_share: f64,
    pub proof_bytes_share: f64,
}
#[derive(Serialize, Default)]
pub struct HookAggregate {
    pub count: usize,
    pub duration_ns: u64,
    pub tally: Tally,
}
#[derive(Serialize)]
pub struct Distribution {
    pub median: f64,
    pub p95: f64,
}
#[derive(Serialize)]
pub struct ExtrinsicAggregate {
    pub count: usize,
    pub ops: Distribution,
    pub proof_bytes: Distribution,
    pub storage_ns: Distribution,
    pub wall_ns: Option<Distribution>,
}
#[derive(Serialize, Default)]
pub struct Report {
    pub blocks: usize,
    pub block_hashes: Vec<String>,
    pub totals: Tally,
    pub execution_proof_bytes: usize,
    pub unattributed_proof_bytes: usize,
    pub phases: BTreeMap<String, PhaseAggregate>,
    pub items: BTreeMap<String, Tally>,
    pub hooks: BTreeMap<String, HookAggregate>,
    pub extrinsics: BTreeMap<String, ExtrinsicAggregate>,
}
fn distribution(mut values: Vec<u64>) -> Distribution {
    values.sort_unstable();
    let n = values.len();
    Distribution {
        median: if n.is_multiple_of(2) {
            (values[n / 2 - 1] as f64 + values[n / 2] as f64) / 2.0
        } else {
            values[n / 2] as f64
        },
        p95: values[(95 * n).div_ceil(100) - 1] as f64,
    }
}
pub fn aggregate(blocks: &[AnnotatedBlock]) -> Report {
    let mut report = Report {
        blocks: blocks.len(),
        ..Default::default()
    };
    let mut samples = BTreeMap::<String, Vec<(Tally, Option<u64>)>>::new();
    for block in blocks {
        report.block_hashes.push(block.block_hash.clone());
        let total = Tally::of(block.all_phases().flat_map(|p| &p.ops));
        report.totals.add(&total);
        if let Some(info) = &block.execution {
            report.execution_proof_bytes += info.proof_bytes;
            report.unattributed_proof_bytes += info.proof_bytes.saturating_sub(total.proof_bytes);
        }
        let spans: BTreeMap<_, _> = block.runtime_spans.iter().map(|s| (s.id, s)).collect();
        let mut hook_ops = BTreeMap::<u64, Vec<&AnnotatedOp>>::new();
        for span in &block.runtime_spans {
            if span.name.starts_with("on_") {
                let hook = report.hooks.entry(span.name.clone()).or_default();
                hook.count += 1;
                hook.duration_ns += span.duration_ns;
            }
        }
        for phase in block.all_phases() {
            let tally = Tally::of(&phase.ops);
            let phase_name = if phase.kind == "apply_extrinsic" {
                match phase.signed {
                    Some(true) => "signed_extrinsics",
                    Some(false) => "inherents",
                    None => "extrinsics_unknown",
                }
            } else {
                &phase.kind
            };
            report
                .phases
                .entry(phase_name.to_owned())
                .or_default()
                .tally
                .add(&tally);
            if phase.kind == "apply_extrinsic" {
                samples
                    .entry(phase.call.clone().unwrap_or_else(|| "<unknown>".into()))
                    .or_default()
                    .push((
                        tally,
                        phase
                            .wall_time
                            .as_ref()
                            .filter(|t| t.source == "runtime_span")
                            .map(|t| t.duration_ns),
                    ));
            }
            let mut item_ops = BTreeMap::<&str, Vec<&AnnotatedOp>>::new();
            for op in &phase.ops {
                item_ops.entry(&op.item).or_default().push(op);
                let mut parent = op.parent_span;
                while let Some(span) = parent.and_then(|id| spans.get(&id)) {
                    if span.name.starts_with("on_") {
                        hook_ops.entry(span.id).or_default().push(op);
                        break;
                    }
                    parent = span.parent_id;
                }
            }
            for (item, ops) in item_ops {
                report
                    .items
                    .entry(item.into())
                    .or_default()
                    .add(&Tally::of(ops));
            }
        }
        for (id, ops) in hook_ops {
            report
                .hooks
                .entry(spans[&id].name.clone())
                .or_default()
                .tally
                .add(&Tally::of(ops));
        }
    }
    for phase in report.phases.values_mut() {
        let share = |n, d| if d == 0 { 0.0 } else { n as f64 / d as f64 };
        phase.ops_share = share(phase.tally.ops, report.totals.ops);
        phase.storage_time_share = share(
            phase.tally.storage_ns as usize,
            report.totals.storage_ns as usize,
        );
        phase.proof_bytes_share = share(phase.tally.proof_bytes, report.totals.proof_bytes);
    }
    for (name, values) in samples {
        report.extrinsics.insert(
            name,
            ExtrinsicAggregate {
                count: values.len(),
                ops: distribution(values.iter().map(|(t, _)| t.ops as u64).collect()),
                proof_bytes: distribution(
                    values.iter().map(|(t, _)| t.proof_bytes as u64).collect(),
                ),
                storage_ns: distribution(values.iter().map(|(t, _)| t.storage_ns).collect()),
                wall_ns: values
                    .iter()
                    .map(|(_, t)| *t)
                    .collect::<Option<Vec<_>>>()
                    .map(distribution),
            },
        );
    }
    report
}
pub fn render(r: &Report, top: usize) -> String {
    let t = &r.totals;
    let mut out = format!(
        "{} block(s); {} ops; reads {} = overlay {} + backend {} (warm {}, cold {}) + unknown {}\nDuplicates {} = overlay {}, warm {}, cold {}, unknown {}; proof {} B ({} ops observed; {} B unattributed)\n\n",
        r.blocks,
        t.ops,
        t.reads,
        t.overlay,
        t.backend_warm + t.backend_cold,
        t.backend_warm,
        t.backend_cold,
        t.origin_unknown,
        t.duplicates,
        t.duplicate_overlay,
        t.duplicate_backend_warm,
        t.duplicate_backend_cold,
        t.duplicate_origin_unknown,
        t.proof_bytes,
        t.proof_ops_observed,
        r.unattributed_proof_bytes
    );
    let mut phases =
        crate::summary::table(1, &["phase", "ops %", "storage %", "proof %", "proof B"]);
    for (name, p) in &r.phases {
        phases.add_row(vec![
            name.clone(),
            format!("{:.1}", p.ops_share * 100.0),
            format!("{:.1}", p.storage_time_share * 100.0),
            format!("{:.1}", p.proof_bytes_share * 100.0),
            p.tally.proof_bytes.to_string(),
        ]);
    }
    out += &format!(
        "Phase shares\n{phases}\n\nTop items by proof bytes\n{}\n",
        item_table(&r.items, top)
    );
    let mut exts = crate::summary::table(
        1,
        &[
            "pallet.call",
            "n",
            "ops med/p95",
            "proof B med/p95",
            "storage µs med/p95",
        ],
    );
    for (name, e) in r.extrinsics.iter().take(top) {
        exts.add_row(vec![
            name.clone(),
            e.count.to_string(),
            pair(&e.ops, 1.0),
            pair(&e.proof_bytes, 1.0),
            pair(&e.storage_ns, 1000.0),
        ]);
    }
    out += &format!("\nExtrinsic distributions\n{exts}\n");
    if !r.hooks.is_empty() {
        let mut hooks =
            crate::summary::table(1, &["hook: pallet", "n", "ops", "proof B", "span µs"]);
        let mut rows: Vec<_> = r.hooks.iter().collect();
        rows.sort_by_key(|(_, h)| std::cmp::Reverse(h.tally.ops));
        for (name, h) in rows.into_iter().take(top) {
            hooks.add_row(vec![
                name.clone(),
                h.count.to_string(),
                h.tally.ops.to_string(),
                h.tally.proof_bytes.to_string(),
                format!("{:.1}", h.duration_ns as f64 / 1000.0),
            ]);
        }
        out += &format!("\nHooks (observed runtime spans)\n{hooks}\n");
    }
    out
}
fn pair(d: &Distribution, scale: f64) -> String {
    format!("{:.1}/{:.1}", d.median / scale, d.p95 / scale)
}
pub fn item_table(items: &BTreeMap<String, Tally>, top: usize) -> comfy_table::Table {
    let mut table = crate::summary::table(
        1,
        &[
            "item",
            "R overlay/warm/cold/?",
            "W",
            "iter",
            "read B",
            "written B",
            "dup",
            "proof B",
        ],
    );
    let mut rows: Vec<_> = items.iter().collect();
    rows.sort_by(|a, b| {
        b.1.proof_bytes
            .cmp(&a.1.proof_bytes)
            .then((b.1.reads + b.1.writes).cmp(&(a.1.reads + a.1.writes)))
            .then(a.0.cmp(b.0))
    });
    for (item, t) in rows.into_iter().take(top) {
        table.add_row(vec![
            item.clone(),
            format!(
                "{}/{}/{}/{}",
                t.overlay, t.backend_warm, t.backend_cold, t.origin_unknown
            ),
            t.writes.to_string(),
            t.iterations.to_string(),
            t.bytes_read.to_string(),
            t.bytes_written.to_string(),
            t.duplicates.to_string(),
            t.proof_bytes.to_string(),
        ]);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    fn trace() -> AnnotatedBlock {
        let json = include_str!("../../../fixtures/model/two-extrinsics.json");
        let raw = frame_trace_model::rpc::parse_trace_block(json).expect("fixture");
        let traced = frame_trace_model::TracedBlock::from_trace(&raw).expect("trace");
        frame_trace_decode::annotate(&traced, None)
    }
    #[test]
    fn nested_intervals_are_unioned_and_percentiles_use_nearest_rank() {
        assert_eq!(union_ns(vec![(0, 10), (2, 4), (8, 12), (20, 23)]), 15);
        let d = distribution(vec![1, 2, 3, 4, 100]);
        assert_eq!((d.median, d.p95), (3.0, 100.0));
        assert_eq!(distribution(vec![2, 4]).median, 3.0);
    }
    #[test]
    fn aggregation_keeps_iterations_out_of_writes_and_splits_duplicate_origins() {
        let mut block = trace();
        let mut op = block.phases[0].ops[0].clone();
        op.op = "iteration".into();
        op.proof_bytes_added = Some(20);
        op.item = "iterate".into();
        block.phases[0].ops = vec![op.clone()];
        for origin in ["overlay", "backend_warm", "backend_cold"] {
            let mut read = op.clone();
            read.op = "get".into();
            read.origin = Some(origin.into());
            read.item = "reads".into();
            read.duplicate_read = true;
            read.proof_bytes_added = Some(if origin == "backend_cold" { 10 } else { 0 });
            block.phases[0].ops.push(read);
        }
        let r = aggregate(&[block.clone(), block]);
        assert_eq!(
            (
                r.items["iterate"].writes,
                r.items["iterate"].iterations,
                r.items["iterate"].proof_bytes
            ),
            (0, 2, 40)
        );
        let reads = &r.items["reads"];
        assert_eq!(
            (reads.overlay, reads.backend_warm, reads.backend_cold),
            (2, 2, 2)
        );
        assert_eq!(
            (
                reads.duplicate_overlay,
                reads.duplicate_backend_warm,
                reads.duplicate_backend_cold
            ),
            (2, 2, 2)
        );
        assert_eq!(
            r.phases
                .values()
                .map(|p| p.tally.proof_bytes)
                .sum::<usize>(),
            r.totals.proof_bytes
        );
        assert_eq!(
            r.items.values().map(|p| p.proof_bytes).sum::<usize>(),
            r.totals.proof_bytes
        );
    }
    #[test]
    fn signed_and_nested_post_inherents_are_counted_once_with_extrinsic_distributions() {
        let mut block = trace();
        let phase = block
            .phases
            .iter_mut()
            .find(|p| p.extrinsic_index == Some(1))
            .expect("extrinsic");
        phase.signed = Some(true);
        phase.call = Some("Balances.transfer".into());
        let expected = phase.ops.len();
        let mut nested = phase.clone();
        nested.post_inherents = None;
        nested.kind = "post_inherents".into();
        nested.extrinsic_index = None;
        nested.ops.truncate(1);
        phase.post_inherents = Some(Box::new(nested));
        let r = aggregate(&[block.clone(), block]);
        assert_eq!(r.extrinsics["Balances.transfer"].count, 2);
        assert_eq!(
            r.extrinsics["Balances.transfer"].ops.median,
            expected as f64
        );
        assert_eq!(r.phases["post_inherents"].tally.ops, 2);
        assert_eq!(r.phases["signed_extrinsics"].tally.ops, expected * 2);
    }
}
