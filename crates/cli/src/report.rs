//! Aggregation of the public JSON format. No replay is needed for saved traces.
use anyhow::{Context, Result, ensure};
use clap::{Args, ValueEnum};
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
    /// First block number in the inclusive finalized range (non-genesis).
    #[arg(long, requires = "rpc", conflicts_with = "last")]
    from: Option<u64>,
    /// Last block number in the inclusive range; cannot exceed finalized head.
    #[arg(long, requires = "from", conflicts_with = "last")]
    to: Option<u64>,
    /// Most recent K blocks anchored to one finalized head.
    #[arg(long, requires="rpc", conflicts_with_all=["from", "to"], value_parser=clap::value_parser!(u32).range(1..=10000))]
    last: Option<u32>,
    /// Concurrent block replays (proof request concurrency is separate).
    #[arg(long, default_value_t=2, value_parser=clap::value_parser!(u8).range(1..=4))]
    concurrency: u8,
    /// Shared maximum simultaneous proof RPC requests (1–64).
    #[arg(long, default_value_t=8, value_parser=clap::value_parser!(u8).range(1..=64))]
    proof_concurrency: u8,
    /// Persist authenticated proof nodes and compiled Wasm in this directory.
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Combined proof-node/compiled-Wasm disk cache limit in bytes (default 1 GiB).
    #[arg(long, default_value_t = 1073741824)]
    cache_max_bytes: u64,
    /// Maximum rows in each ranked item, extrinsic and hook table (JSON is complete).
    #[arg(long, default_value_t = 15)]
    top: usize,
    /// Rank the item, extrinsic and hook tables by total proof bytes, operations or storage time.
    #[arg(long, value_enum, default_value_t = SortKey::Proof)]
    sort_by: SortKey,
    /// Aggregated JSON (- sends tables to stderr).
    #[arg(long)]
    json: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum SortKey {
    #[default]
    Proof,
    Ops,
    Time,
}
impl SortKey {
    fn label(self) -> &'static str {
        match self {
            Self::Proof => "proof bytes",
            Self::Ops => "operations",
            Self::Time => "storage time",
        }
    }
    /// The ranked cost of a tally; storage time is the union of its op intervals.
    fn cost(self, t: &Tally) -> u128 {
        match self {
            Self::Proof => t.proof_bytes as u128,
            Self::Ops => t.ops as u128,
            Self::Time => u128::from(t.storage_ns),
        }
    }
}

fn validate_range(from: u64, to: u64, finalized: u64) -> Result<()> {
    ensure!(
        from >= 1 && from <= to && to <= finalized,
        "range must be ordered, non-genesis and finalized (head #{finalized})"
    );
    ensure!(to - from < 10000, "range limit is 10000 blocks");
    Ok(())
}

pub fn read_trace(path: &Path) -> Result<AnnotatedBlock> {
    let block: AnnotatedBlock = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?,
    ))
    .with_context(|| format!("parsing block JSON {}", path.display()))?;
    ensure!(
        (1..=2).contains(&block.schema_version),
        "unsupported block JSON schema version {} in {}",
        block.schema_version,
        path.display()
    );
    ensure!(
        block.call.is_none(),
        "{} is a call trace; expected block --json",
        path.display()
    );
    Ok(block)
}

pub fn run(args: ReportArgs) -> Result<()> {
    let mut accumulator = Accumulator::default();
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
        validate_range(from, to, finalized)?;
        // Finalized number lookups are immutable. Resolve each hash once before replay.
        let numbers: Vec<_> = (from..=to).collect();
        for chunk in numbers.chunks(args.concurrency as usize) {
            std::thread::scope(|scope| {
                let (sender, receiver) = std::sync::mpsc::sync_channel(chunk.len());
                for &number in chunk {
                    let sender = sender.clone();
                    let rpc = rpc.fork();
                    let session = &session;
                    scope.spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            || -> Result<AnnotatedBlock> {
                                let hash =
                                    rpc.call("chain_getBlockHash", serde_json::json!([number]))?;
                                let hash = hash.as_str().context("block hash unavailable")?;
                                let replay =
                                    frame_trace_replay::online_with_session(&rpc, hash, session)?;
                                let loaded = crate::source::replay_loaded(replay, None)?;
                                let mut block = frame_trace_decode::annotate_at_block(
                                    &loaded.traced,
                                    loaded.metadata.as_ref(),
                                    loaded.block_number,
                                );
                                block.execution = loaded.execution;
                                Ok(block)
                            },
                        ))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("report replay worker panicked")));
                        let _ = sender.send((number, result));
                    });
                }
                drop(sender);
                // Fold as workers finish, releasing each trace even if another worker stalls.
                for (number, result) in receiver {
                    accumulator.record(format!("block #{number}"), result);
                }
            });
        }
    } else {
        for path in args.files.iter().chain(&args.from_json) {
            accumulator.record(path.display().to_string(), read_trace(path));
        }
    }
    let report = accumulator.finish();
    if let Some(path) = &args.json {
        let json = serde_json::to_string_pretty(&report)?;
        if path.as_os_str() == "-" {
            println!("{json}");
        } else {
            std::fs::write(path, json)
                .with_context(|| format!("writing report JSON {}", path.display()))?;
        }
    }
    let tables = render_sorted(&report, args.top, args.sort_by);
    if args.json.is_some_and(|p| p.as_os_str() == "-") {
        eprint!("{tables}");
    } else {
        print!("{tables}");
    }
    if let Some(error) = report.errors.first() {
        anyhow::bail!(
            "{} block input(s) failed; {}: {}",
            report.errors.len(),
            error.input,
            error.error
        );
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
                tally.proof_bytes = tally.proof_bytes.saturating_add(bytes);
                tally.proof_ops_observed += 1;
            }
            if let Some(t) = op.timing {
                intervals.push((t.start_ns, t.start_ns.saturating_add(t.duration_ns)));
            }
            if is_read(op) {
                tally.reads += 1;
                tally.bytes_read = tally.bytes_read.saturating_add(op.value_len.unwrap_or(0));
                match origin_index(op.origin.as_deref()) {
                    0 => tally.overlay += 1,
                    1 => tally.backend_warm += 1,
                    2 => tally.backend_cold += 1,
                    _ => tally.origin_unknown += 1,
                }
                if op.duplicate_read {
                    tally.duplicates += 1;
                    match origin_index(op.origin.as_deref()) {
                        0 => tally.duplicate_overlay += 1,
                        1 => tally.duplicate_backend_warm += 1,
                        2 => tally.duplicate_backend_cold += 1,
                        _ => tally.duplicate_origin_unknown += 1,
                    }
                }
            } else if matches!(
                op.op.as_str(),
                "put" | "clear" | "append" | "clear_prefix" | "kill_child"
            ) {
                tally.writes += 1;
                tally.bytes_written = tally
                    .bytes_written
                    .saturating_add(op.value_len.unwrap_or(0));
            } else if matches!(op.op.as_str(), "next_key" | "iteration") {
                tally.iterations += 1;
            }
        }
        tally.storage_ns = union_ns(intervals);
        tally
    }
    fn add(&mut self, other: &Self) {
        macro_rules! add { ($($field:ident),*) => { $(self.$field = self.$field.saturating_add(other.$field);)* }; }
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
    let mut total = 0u64;
    for (lo, hi) in intervals {
        total = total.saturating_add(hi.saturating_sub(lo.max(end)));
        end = end.max(hi);
    }
    total
}
#[derive(Serialize, Default)]
pub struct PhaseAggregate {
    pub tally: Tally,
    pub ops_share: f64,
    pub storage_time_share: f64,
    pub proof_bytes_share: Option<f64>,
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
    pub proof_bytes: Option<Distribution>,
    pub total_proof_bytes: usize,
    pub total_ops: usize,
    pub total_storage_ns: u64,
    pub storage_ns: Distribution,
    pub wall_ns: Option<Distribution>,
}
/// Provenance kept for every input block that ran a substitute runtime.
#[derive(Serialize)]
pub struct OverriddenBlock {
    pub block_hash: String,
    #[serde(flatten)]
    pub runtime: frame_trace_decode::RuntimeOverrideInfo,
}
#[derive(Serialize, Default)]
pub struct Report {
    pub schema_version: u32,
    pub errors: Vec<BlockError>,
    pub phase_order: Vec<String>,
    pub blocks: usize,
    pub block_hashes: BTreeSet<String>,
    pub totals: Tally,
    pub execution_proof_bytes: Option<usize>,
    pub unattributed_proof_bytes: Option<usize>,
    /// Attributed proof bytes beyond each block's final execution proof, summed over blocks.
    /// Rollback can re-add paths, so this is visible rather than clamped into `unattributed`.
    pub over_attributed_proof_bytes: Option<usize>,
    /// Blocks whose attributed proof bytes exceed their execution proof.
    pub over_attributed_blocks: usize,
    /// Blocks executed with a substitute runtime instead of their own `:code`.
    pub runtime_overrides: Vec<OverriddenBlock>,
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
/// Exact through 1024 samples; then fixed logarithmic buckets (64 per power of two).
/// Quantiles use bucket lower bounds, with value error <= 1/64 (1.5625%).
#[derive(Default)]
struct Samples {
    exact: Vec<u64>,
    buckets: BTreeMap<u16, usize>,
    count: usize,
}
impl Samples {
    fn bucket(v: u64) -> u16 {
        if v == 0 {
            return 0;
        }
        let exponent = 63 - v.leading_zeros();
        let fraction = (((v as u128 - (1u128 << exponent)) * 64) >> exponent) as u16;
        1 + exponent as u16 * 64 + fraction
    }
    fn lower(bucket: u16) -> u64 {
        if bucket == 0 {
            return 0;
        }
        let b = bucket - 1;
        (((64 + u128::from(b % 64)) << (b / 64)) / 64) as u64
    }
    fn push(&mut self, value: u64) {
        self.count += 1;
        if self.count <= 1024 {
            self.exact.push(value);
            return;
        }
        for v in self.exact.drain(..) {
            *self.buckets.entry(Self::bucket(v)).or_default() += 1;
        }
        *self.buckets.entry(Self::bucket(value)).or_default() += 1;
    }
    fn finish(self) -> Option<Distribution> {
        if self.count == 0 {
            return None;
        }
        if self.buckets.is_empty() {
            return Some(distribution(self.exact));
        }
        let rank = |r: usize| {
            let mut seen = 0;
            for (bucket, count) in &self.buckets {
                seen += count;
                if seen >= r {
                    return Self::lower(*bucket) as f64;
                }
            }
            unreachable!("nonempty quantile buckets")
        };
        Some(Distribution {
            median: (rank(self.count.div_ceil(2)) + rank(self.count / 2 + 1)) / 2.0,
            p95: rank((95 * self.count).div_ceil(100)),
        })
    }
}
#[derive(Default)]
struct ExtrinsicSamples {
    count: usize,
    ops: Samples,
    proof: Samples,
    storage: Samples,
    wall: Samples,
    total: Tally,
}
#[derive(Serialize)]
pub struct BlockError {
    pub input: String,
    pub error: String,
}
#[derive(Default)]
pub struct Accumulator {
    report: Report,
    samples: BTreeMap<String, ExtrinsicSamples>,
}
impl Accumulator {
    fn record(&mut self, input: String, block: Result<AnnotatedBlock>) {
        let result = block.and_then(|block| self.add(&block));
        if let Err(error) = result {
            self.report.errors.push(BlockError {
                input,
                error: format!("{error:#}"),
            });
        }
    }
    pub fn add(&mut self, block: &AnnotatedBlock) -> Result<()> {
        ensure!(
            self.report.block_hashes.insert(block.block_hash.clone()),
            "duplicate block hash in report input: {}",
            block.block_hash
        );
        self.fold(block);
        Ok(())
    }
    fn fold(&mut self, block: &AnnotatedBlock) {
        self.report.blocks += 1;
        let report = &mut self.report;
        report.block_hashes.insert(block.block_hash.clone());
        let total = Tally::of(block.all_phases().flat_map(|p| &p.ops));
        report.totals.add(&total);
        if let Some(runtime) = &block.runtime_override {
            report.runtime_overrides.push(OverriddenBlock {
                block_hash: block.block_hash.clone(),
                runtime: runtime.clone(),
            });
        }
        if let Some(info) = &block.execution {
            let execution = report.execution_proof_bytes.get_or_insert(0);
            *execution = execution.saturating_add(info.proof_bytes);
            let unattributed = report.unattributed_proof_bytes.get_or_insert(0);
            *unattributed =
                unattributed.saturating_add(info.proof_bytes.saturating_sub(total.proof_bytes));
            let over = total.proof_bytes.saturating_sub(info.proof_bytes);
            let over_total = report.over_attributed_proof_bytes.get_or_insert(0);
            *over_total = over_total.saturating_add(over);
            if over > 0 {
                report.over_attributed_blocks += 1;
            }
        }
        let spans: BTreeMap<_, _> = block.runtime_spans.iter().map(|s| (s.id, s)).collect();
        let mut hook_ops = BTreeMap::<u64, Vec<&AnnotatedOp>>::new();
        for span in &block.runtime_spans {
            if span.name.starts_with("on_") {
                let hook = report.hooks.entry(span.name.clone()).or_default();
                hook.count += 1;
                hook.duration_ns = hook.duration_ns.saturating_add(span.duration_ns);
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
            if !report.phases.contains_key(phase_name) {
                report.phase_order.push(phase_name.to_owned());
            }
            report
                .phases
                .entry(phase_name.to_owned())
                .or_default()
                .tally
                .add(&tally);
            if phase.kind == "apply_extrinsic" {
                let sample = self
                    .samples
                    .entry(phase.call.clone().unwrap_or_else(|| "<unknown>".into()))
                    .or_default();
                sample.count += 1;
                sample.ops.push(tally.ops as u64);
                if tally.ops > 0 && tally.proof_ops_observed == tally.ops {
                    sample.proof.push(tally.proof_bytes as u64);
                }
                sample.storage.push(tally.storage_ns);
                if let Some(t) = phase
                    .wall_time
                    .as_ref()
                    .filter(|t| t.source == "runtime_span")
                {
                    sample.wall.push(t.duration_ns);
                }
                sample.total.add(&tally);
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
    pub fn finish(self) -> Report {
        let mut report = self.report;
        report.schema_version = 2;
        for phase in report.phases.values_mut() {
            let share = |n, d| if d == 0 { 0.0 } else { n as f64 / d as f64 };
            phase.ops_share = share(phase.tally.ops, report.totals.ops);
            phase.storage_time_share = share(
                phase.tally.storage_ns as usize,
                report.totals.storage_ns as usize,
            );
            phase.proof_bytes_share = (phase.tally.proof_ops_observed > 0)
                .then(|| share(phase.tally.proof_bytes, report.totals.proof_bytes));
        }
        for (name, values) in self.samples {
            report.extrinsics.insert(
                name,
                ExtrinsicAggregate {
                    count: values.count,
                    ops: values.ops.finish().expect("extrinsic samples"),
                    proof_bytes: if values.proof.count == values.count {
                        values.proof.finish()
                    } else {
                        None
                    },
                    storage_ns: values.storage.finish().expect("extrinsic samples"),
                    wall_ns: if values.wall.count == values.count {
                        values.wall.finish()
                    } else {
                        None
                    },
                    total_proof_bytes: values.total.proof_bytes,
                    total_ops: values.total.ops,
                    total_storage_ns: values.total.storage_ns,
                },
            );
        }
        report
    }
}
pub fn aggregate(blocks: &[AnnotatedBlock]) -> Report {
    let mut accumulator = Accumulator::default();
    // Also used for a single call summary, where duplicate validation is immaterial.
    for block in blocks {
        accumulator.fold(block);
    }
    accumulator.finish()
}
#[cfg(test)]
pub fn render(r: &Report, top: usize) -> String {
    render_sorted(r, top, SortKey::Proof)
}
fn render_sorted(r: &Report, top: usize, sort: SortKey) -> String {
    override_lines(r, top) + &render_body(r, top, sort)
}
/// The report tables without the leading override provenance.
pub fn render_body(r: &Report, top: usize, sort: SortKey) -> String {
    let t = &r.totals;
    let mut out = format!(
        "{} block(s); {} ops; reads {} = overlay {} + backend {} (warm {}, cold {}) + unknown {}\nDuplicates {} = overlay {}, warm {}, cold {}, unknown {}; proof {} ({} ops observed; {} unattributed{})\n",
        r.blocks,
        t.ops,
        t.reads,
        t.overlay,
        t.backend_warm.saturating_add(t.backend_cold),
        t.backend_warm,
        t.backend_cold,
        t.origin_unknown,
        t.duplicates,
        t.duplicate_overlay,
        t.duplicate_backend_warm,
        t.duplicate_backend_cold,
        t.duplicate_origin_unknown,
        proof_bytes_text(t),
        t.proof_ops_observed,
        r.unattributed_proof_bytes
            .map_or_else(|| "n/a".into(), |bytes| format!("{bytes} B")),
        r.over_attributed_proof_bytes
            .filter(|&bytes| bytes > 0)
            .map_or_else(String::new, |bytes| format!("; {bytes} B over-attributed"))
    );
    if let Some(bytes) = r.over_attributed_proof_bytes.filter(|&bytes| bytes > 0) {
        out += &format!(
            "warning: attributed proof bytes exceed the execution proof by {bytes} B in {} block(s); \
             rollback can re-add proof paths, so per-op sums are not standalone proofs\n",
            r.over_attributed_blocks
        );
    }
    out += "\n";
    let mut phases =
        crate::summary::table(1, &["phase", "ops %", "storage %", "proof %", "proof B"]);
    for name in &r.phase_order {
        let p = &r.phases[name];
        phases.add_row(vec![
            name.clone(),
            format!("{:.1}", p.ops_share * 100.0),
            format!("{:.1}", p.storage_time_share * 100.0),
            p.proof_bytes_share
                .map_or_else(|| "n/a".into(), |share| format!("{:.1}", share * 100.0)),
            proof_label(&p.tally),
        ]);
    }
    out += &format!(
        "Phase shares\n{phases}\n\nTop items by {}\n{}\n",
        sort.label(),
        item_table(&r.items, top, sort)
    );
    out += &truncated(r.items.len(), top);
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
    let mut rows: Vec<_> = r.extrinsics.iter().collect();
    rows.sort_by(|a, b| {
        let cost = |e: &ExtrinsicAggregate| match sort {
            SortKey::Proof => e.total_proof_bytes as u128,
            SortKey::Ops => e.total_ops as u128,
            SortKey::Time => u128::from(e.total_storage_ns),
        };
        cost(b.1).cmp(&cost(a.1)).then(a.0.cmp(b.0))
    });
    for (name, e) in rows.into_iter().take(top) {
        exts.add_row(vec![
            name.clone(),
            e.count.to_string(),
            pair(&e.ops, 1.0),
            e.proof_bytes
                .as_ref()
                .map_or_else(|| "n/a".into(), |d| pair(d, 1.0)),
            pair(&e.storage_ns, 1000.0),
        ]);
    }
    out += &format!(
        "\nExtrinsic distributions (ranked by total {})\n{exts}\n{}",
        sort.label(),
        truncated(r.extrinsics.len(), top)
    );
    if !r.hooks.is_empty() {
        let mut hooks =
            crate::summary::table(1, &["hook: pallet", "n", "ops", "proof B", "span µs"]);
        let mut rows: Vec<_> = r.hooks.iter().collect();
        rows.sort_by(|a, b| {
            sort.cost(&b.1.tally)
                .cmp(&sort.cost(&a.1.tally))
                .then(b.1.tally.ops.cmp(&a.1.tally.ops))
                .then(a.0.cmp(b.0))
        });
        for (name, h) in rows.into_iter().take(top) {
            hooks.add_row(vec![
                name.clone(),
                h.count.to_string(),
                h.tally.ops.to_string(),
                proof_label(&h.tally),
                format!("{:.1}", h.duration_ns as f64 / 1000.0),
            ]);
        }
        out += &format!(
            "\nHooks (observed runtime spans, ranked by {})\n{hooks}\n",
            sort.label()
        );
        out += &truncated(r.hooks.len(), top);
    }
    if !r.errors.is_empty() {
        out += "\nErrors\n";
        for error in &r.errors {
            out += &format!("{}: {}\n", error.input, error.error);
        }
    }
    out
}
/// Substitute runtimes are named before any number they produced.
pub fn override_lines(r: &Report, top: usize) -> String {
    if r.runtime_overrides.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "RUNTIME OVERRIDE: {} of {} block(s) used a runtime override; their spans and timings \
         describe the substitute Wasm, not the chain's own code\n",
        r.runtime_overrides.len(),
        r.blocks
    );
    for o in r.runtime_overrides.iter().take(top) {
        let runtime = &o.runtime;
        out += &format!(
            "  {}: executed {} ({}) instead of :code {}",
            o.block_hash, runtime.code_hash, runtime.path, runtime.chain_code_hash
        );
        if !runtime.version_differences.is_empty() {
            out += &format!("; version: {}", runtime.version_differences.join("; "));
        }
        out += "\n";
    }
    out + &truncated(r.runtime_overrides.len(), top)
}
fn truncated(n: usize, top: usize) -> String {
    if n > top {
        format!("… {} more\n", n - top)
    } else {
        String::new()
    }
}
/// `proof_label` with its unit; no unit when there is no evidence.
fn proof_bytes_text(t: &Tally) -> String {
    if t.proof_ops_observed == 0 {
        "n/a".into()
    } else if t.proof_ops_observed < t.ops {
        format!("{} B (partial)", t.proof_bytes)
    } else {
        format!("{} B", t.proof_bytes)
    }
}
pub fn proof_label(t: &Tally) -> String {
    if t.proof_ops_observed == 0 {
        "n/a".into()
    } else if t.proof_ops_observed < t.ops {
        format!("{} (partial)", t.proof_bytes)
    } else {
        t.proof_bytes.to_string()
    }
}
/// Shared origin column: overlay, warm, cold, unknown.
pub fn origin_index(origin: Option<&str>) -> usize {
    match origin {
        Some("overlay") => 0,
        Some("backend_warm") => 1,
        Some("backend_cold") => 2,
        _ => 3,
    }
}
fn pair(d: &Distribution, scale: f64) -> String {
    format!("{:.1}/{:.1}", d.median / scale, d.p95 / scale)
}
pub fn item_table(
    items: &BTreeMap<String, Tally>,
    top: usize,
    sort: SortKey,
) -> comfy_table::Table {
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
        sort.cost(b.1)
            .cmp(&sort.cost(a.1))
            .then(b.1.ops.cmp(&a.1.ops))
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
            proof_label(t),
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

#[cfg(test)]
mod round5_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    fn legacy() -> AnnotatedBlock {
        serde_json::from_str(include_str!("../../../fixtures/model/pre-e86f740.json")).unwrap()
    }
    #[test]
    fn legacy_json_keeps_missing_proof_and_origins_unrecorded() {
        let block = legacy();
        assert_eq!(block.schema_version, 1);
        assert!(
            block
                .all_phases()
                .flat_map(|p| &p.ops)
                .all(|op| op.origin.is_none() && op.proof_bytes_added.is_none())
        );
        let report = aggregate(std::slice::from_ref(&block));
        assert_eq!(report.totals.proof_ops_observed, 0);
        assert_eq!(report.totals.reads, report.totals.origin_unknown);
        assert!(report.extrinsics.values().all(|e| e.proof_bytes.is_none()));
        assert!(render(&report, 1).contains("proof n/a"));
        assert!(crate::summary::render(&block, 1).contains("n/a"));
    }
    #[test]
    fn report_ranges_are_inclusive_finalized_non_genesis_and_bounded() {
        for (from, to, head) in [
            (4, 3, 5),
            (1, 6, 5),
            (0, 1, 5),
            (1, 10001, 10001),
            (u64::MAX, 0, u64::MAX),
        ] {
            assert!(validate_range(from, to, head).is_err());
        }
        assert!(validate_range(1, 10000, 10000).is_ok());
        assert!(validate_range(5, 5, 5).is_ok());
    }
    #[test]
    fn crafted_json_max_durations_and_tallies_saturate() {
        let mut value = serde_json::to_value(legacy()).unwrap();
        value["runtime_spans"] = serde_json::json!([
            {"id":1,"parent_id":null,"name":"on_initialize: Huge","target":"pallet_huge","wasm":true,
             "fields":{},"start_ns":0,"end_ns":u64::MAX,"duration_ns":u64::MAX,"enters":1,"proof_start_bytes":0,"proof_end_bytes":0},
            {"id":2,"parent_id":null,"name":"on_initialize: Huge","target":"pallet_huge","wasm":true,
             "fields":{},"start_ns":0,"end_ns":u64::MAX,"duration_ns":u64::MAX,"enters":1,"proof_start_bytes":0,"proof_end_bytes":0}
        ]);
        for op in value["phases"][0]["ops"].as_array_mut().unwrap() {
            op["timing"] = serde_json::json!({"start_ns":0,"duration_ns":u64::MAX});
            op["proof_bytes_added"] = serde_json::json!(usize::MAX);
            op["value_len"] = serde_json::json!(usize::MAX);
        }
        let block: AnnotatedBlock = serde_json::from_value(value).unwrap();
        let report = aggregate(&[block.clone(), block]);
        assert_eq!(report.hooks["on_initialize: Huge"].duration_ns, u64::MAX);
        assert_eq!(report.totals.storage_ns, u64::MAX);
        assert_eq!(report.totals.proof_bytes, usize::MAX);
        assert!(
            report.totals.bytes_read == usize::MAX || report.totals.bytes_written == usize::MAX
        );
        let _ = render(&report, 3);
    }
    #[test]
    fn quantile_sketch_is_bounded_and_respects_documented_value_error() {
        let mut samples = Samples::default();
        for value in 1..=100_000 {
            samples.push(value);
        }
        assert!(samples.exact.is_empty());
        assert!(samples.buckets.len() <= 4097);
        let result = samples.finish().unwrap();
        assert!((50_000.5 - result.median).abs() <= 50_000.5 / 64.0);
        assert!((95_000.0 - result.p95).abs() <= 95_000.0 / 64.0);
        for value in [0, 1, 63, 64, 127, 128, u64::MAX] {
            let estimate = Samples::lower(Samples::bucket(value));
            assert!(estimate <= value);
            assert!(value - estimate <= value / 64);
        }
    }
    #[test]
    fn top_extrinsics_sort_by_cost_and_phase_order_follows_execution() {
        let mut block = legacy();
        let phases: Vec<_> = block
            .phases
            .iter_mut()
            .filter(|p| p.kind == "apply_extrinsic")
            .collect();
        for (i, phase) in phases.into_iter().enumerate() {
            phase.call = Some(
                if i == 0 {
                    "Alphabet.first"
                } else {
                    "Zebra.expensive"
                }
                .into(),
            );
            if i == 0 {
                let op = phase.ops[0].clone();
                phase.ops.resize(100, op);
            }
            for op in &mut phase.ops {
                op.timing = Some(frame_trace_model::MeasuredTiming {
                    start_ns: 0,
                    duration_ns: if i == 0 { 1 } else { 10000 },
                });
                op.proof_bytes_added = Some(if i == 0 { 1 } else { 1000 });
            }
        }
        let r = aggregate(&[block]);
        let text = render_sorted(&r, 1, SortKey::Proof);
        assert!(text.contains("Zebra.expensive"));
        assert!(!text.contains("Alphabet.first"));
        let by_ops = render_sorted(&r, 1, SortKey::Ops);
        assert!(by_ops.contains("Alphabet.first") && !by_ops.contains("Zebra.expensive"));
        let by_time = render_sorted(&r, 1, SortKey::Time);
        assert!(by_time.contains("Zebra.expensive") && !by_time.contains("Alphabet.first"));
        assert!(text.contains("… 1 more"));
        assert_eq!(r.phase_order.first().unwrap(), "initialization");
        assert_eq!(r.phase_order.last().unwrap(), "finalization");
    }
    #[test]
    fn completed_blocks_fold_immediately_and_errors_do_not_drop_successes() {
        let mut accumulator = Accumulator::default();
        let block = legacy();
        accumulator.record("ok".into(), Ok(block.clone()));
        accumulator.record("missing".into(), Err(anyhow::anyhow!("missing fixture")));
        accumulator.record("duplicate".into(), Ok(block));
        let report = accumulator.finish();
        assert_eq!(report.blocks, 1);
        assert_eq!(report.errors.len(), 2);
        assert!(report.errors[1].error.contains("duplicate block hash"));
    }
    #[test]
    fn missing_proof_evidence_prints_no_unit_and_present_evidence_does() {
        let block = legacy();
        let text = render(&aggregate(std::slice::from_ref(&block)), 1);
        assert!(
            text.contains("proof n/a (0 ops observed; n/a unattributed)"),
            "{text}"
        );
        assert!(!text.contains("n/a B"));
        assert!(!crate::summary::render(&block, 1).contains("n/a B"));
        let mut value = serde_json::to_value(&block).unwrap();
        for phase in value["phases"].as_array_mut().unwrap() {
            for op in phase["ops"].as_array_mut().unwrap() {
                op["proof_bytes_added"] = serde_json::json!(2);
            }
        }
        value["execution"] = serde_json::json!({"root_check":true,"proof_bytes":100,
            "rpc_calls":0,"measured_rpc_calls":0,"proof_rounds":0});
        let block: AnnotatedBlock = serde_json::from_value(value).unwrap();
        let r = aggregate(std::slice::from_ref(&block));
        let text = render(&r, 1);
        let attributed = r.totals.proof_bytes;
        assert!(
            text.contains(&format!(
                "proof {attributed} B (partial) ({} ops observed; {} B unattributed)",
                r.totals.proof_ops_observed,
                100 - attributed
            )),
            "{text}"
        );
        assert_eq!(r.over_attributed_proof_bytes, Some(0));
        assert!(!text.contains("over-attributed"));
    }
    #[test]
    fn over_attribution_is_reported_instead_of_clamped_to_zero() {
        let mut value = serde_json::to_value(legacy()).unwrap();
        for phase in value["phases"].as_array_mut().unwrap() {
            for op in phase["ops"].as_array_mut().unwrap() {
                op["proof_bytes_added"] = serde_json::json!(10);
            }
        }
        value["execution"] = serde_json::json!({"root_check":true,"proof_bytes":5,
            "rpc_calls":0,"measured_rpc_calls":0,"proof_rounds":0});
        let over: AnnotatedBlock = serde_json::from_value(value.clone()).unwrap();
        value["block_hash"] = serde_json::json!("0x33");
        value["execution"]["proof_bytes"] = serde_json::json!(1_000_000);
        let under: AnnotatedBlock = serde_json::from_value(value).unwrap();
        let attributed = aggregate(std::slice::from_ref(&over)).totals.proof_bytes;
        let r = aggregate(&[over, under]);
        assert_eq!(r.over_attributed_proof_bytes, Some(attributed - 5));
        assert_eq!(r.over_attributed_blocks, 1);
        assert_eq!(r.unattributed_proof_bytes, Some(1_000_000 - attributed));
        assert_eq!(r.execution_proof_bytes, Some(1_000_005));
        let text = render(&r, 1);
        assert!(
            text.contains(&format!("; {} B over-attributed)", attributed - 5)),
            "{text}"
        );
        assert!(text.contains("warning: attributed proof bytes exceed the execution proof"));
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["over_attributed_proof_bytes"], attributed - 5);
        assert_eq!(json["over_attributed_blocks"], 1);
    }
    #[test]
    fn sort_key_ranks_items_and_hooks_as_well_as_extrinsics() {
        let mut value = serde_json::to_value(legacy()).unwrap();
        value["runtime_spans"] = serde_json::json!([
            {"id":1,"parent_id":null,"name":"on_initialize: Few","target":"pallet_few","wasm":true,
             "fields":{},"start_ns":0,"end_ns":1,"duration_ns":1,"enters":1,"proof_start_bytes":0,"proof_end_bytes":0},
            {"id":2,"parent_id":null,"name":"on_finalize: Many","target":"pallet_many","wasm":true,
             "fields":{},"start_ns":0,"end_ns":1,"duration_ns":1,"enters":1,"proof_start_bytes":0,"proof_end_bytes":0}
        ]);
        let mut n = 0;
        let mut slowest = String::new();
        for phase in value["phases"].as_array_mut().unwrap() {
            for op in phase["ops"].as_array_mut().unwrap() {
                // Op 0 alone is slow: it decides the storage-time ranking of items and hooks.
                op["timing"] = serde_json::json!({"start_ns": n * 1_000_000,
                    "duration_ns": if n == 0 { 900_000 } else { 1 }});
                if n == 0 {
                    slowest = op["item"].as_str().unwrap().to_owned();
                }
                let index = op["item"] == ":extrinsic_index";
                // `:extrinsic_index` has fewer ops but far more proof bytes.
                op["proof_bytes_added"] = serde_json::json!(if index { 1000 } else { 1 });
                op["parent_span"] = serde_json::json!(if n == 0 {
                    1
                } else if n <= 5 {
                    2
                } else {
                    99
                });
                if n == 0 {
                    op["proof_bytes_added"] = serde_json::json!(5000);
                }
                n += 1;
            }
        }
        let block: AnnotatedBlock = serde_json::from_value(value).unwrap();
        let r = aggregate(&[block]);
        let section = |text: &str, from: &str, to: &str| {
            let start = text.find(from).unwrap();
            let end = text[start..].find(to).map_or(text.len(), |i| start + i);
            text[start..end].to_owned()
        };
        let by_proof = render_sorted(&r, 1, SortKey::Proof);
        let by_ops = render_sorted(&r, 1, SortKey::Ops);
        let items = section(&by_proof, "Top items by proof bytes", "Extrinsic");
        assert!(
            items.contains(":extrinsic_index") && !items.contains("<unknown>"),
            "{by_proof}"
        );
        let items = section(&by_ops, "Top items by operations", "Extrinsic");
        assert!(
            items.contains("<unknown>") && !items.contains(":extrinsic_index"),
            "{by_ops}"
        );
        let hooks = section(
            &by_proof,
            "Hooks (observed runtime spans, ranked by proof bytes)",
            "…",
        );
        assert!(
            hooks.contains("Few") && !hooks.contains("Many"),
            "{by_proof}"
        );
        let hooks = section(
            &by_ops,
            "Hooks (observed runtime spans, ranked by operations)",
            "…",
        );
        assert!(hooks.contains("Many") && !hooks.contains("Few"), "{by_ops}");
        let by_time = render_sorted(&r, 1, SortKey::Time);
        let items = section(&by_time, "Top items by storage time", "Extrinsic");
        assert!(items.contains(&slowest), "{by_time}");
        let hooks = section(
            &by_time,
            "Hooks (observed runtime spans, ranked by storage time)",
            "…",
        );
        assert!(
            hooks.contains("Few") && !hooks.contains("Many"),
            "{by_time}"
        );
    }
    #[test]
    fn summary_and_report_tables_match_checked_in_goldens() {
        let block = legacy();
        assert_eq!(
            crate::summary::render(&block, 2),
            include_str!("../../../fixtures/model/summary.golden.txt")
        );
        assert_eq!(
            render(&aggregate(&[block]), 2),
            include_str!("../../../fixtures/model/report.golden.txt")
        );
    }
}
