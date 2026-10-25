//! A traced block with names: the form the exporters and the CLI consume.

use frame_trace_model::{Phase, PrintedBytes, Stats, StorageOp, TracedBlock};
use parity_scale_codec::Decode;
use serde::{Deserialize, Serialize};

use crate::{KeyName, Metadata, key::well_known};

/// A [`TracedBlock`] with every key and extrinsic named. Serialises to the `--json` output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatedBlock {
    /// Public JSON format version; absent versions are legacy v1.
    #[serde(default = "legacy_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub execution: Option<ExecutionInfo>,
    /// Runtime API context, absent for block traces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<AnnotatedCall>,
    /// A substitute runtime executed instead of the state's `:code`; absent normally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_override: Option<RuntimeOverrideInfo>,
    /// Observed span tree, empty for legacy JSON.
    pub runtime_spans: Vec<frame_trace_model::RuntimeSpan>,
    /// Replay duration, absent for synthetic input.
    pub timing: Option<frame_trace_model::MeasuredTiming>,
    /// Block number. `state_traceBlock` does not report it, so it is `None` unless the caller
    /// knows it (for example from the RPC source).
    pub block_number: Option<u64>,
    /// Block hash.
    pub block_hash: String,
    /// Parent hash.
    pub parent_hash: String,
    /// Events in the trace that were not storage ops.
    pub skipped_events: usize,
    /// Totals over the block.
    pub totals: Stats,
    /// Phases in execution order.
    pub phases: Vec<AnnotatedPhase>,
}

/// One authenticated runtime API execution. Block root checks do not apply to calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatedCall {
    pub method: String,
    pub data: String,
    pub result: String,
    pub decoded: Option<String>,
    pub oracle_match: Option<bool>,
    pub oracle_source: Option<String>,
    pub proof_bytes: usize,
    pub state_root: String,
}

/// One phase with its ops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatedPhase {
    /// Measured phase bounds (including compute), absent if no defining spans exist.
    #[serde(default)]
    pub wall_time: Option<PhaseTiming>,
    /// Whether the encoded extrinsic is signed; absent when bytes are unavailable.
    #[serde(default)]
    pub signed: Option<bool>,
    /// `initialization`, `apply_extrinsic`, `finalization` or `unsegmented`.
    pub kind: String,
    /// Extrinsic index for `apply_extrinsic`.
    pub extrinsic_index: Option<u32>,
    /// `Pallet.call` of the extrinsic, when its bytes were in the trace and decoded.
    pub call: Option<String>,
    /// Why the extrinsic bytes did not decode. Usually the metadata is not the block's runtime.
    pub call_error: Option<String>,
    /// Display name, for example `extrinsic 1: Balances.transfer_keep_alive`.
    pub label: String,
    /// Storage totals of the phase's own ops (without `post_inherents`).
    pub stats: Stats,
    /// The phase's own ops in order (without those of `post_inherents`).
    pub ops: Vec<AnnotatedOp>,
    /// `inherents_applied` work (`on_poll`) nested inside the first signed extrinsic, between its
    /// `check` reads and its own work.
    pub post_inherents: Option<Box<AnnotatedPhase>>,
}

impl AnnotatedBlock {
    /// Every phase, each followed by its nested phase.
    pub fn all_phases(&self) -> impl Iterator<Item = &AnnotatedPhase> {
        self.phases.iter().flat_map(AnnotatedPhase::with_nested)
    }
}

impl AnnotatedPhase {
    /// This phase, then its nested phase if any.
    pub fn with_nested(&self) -> impl Iterator<Item = &AnnotatedPhase> {
        std::iter::once(self).chain(self.post_inherents.as_deref())
    }

    /// Positions in the block of the first and past-the-last op, nested phase included.
    pub fn seq_span(&self) -> Option<std::ops::Range<usize>> {
        let seqs = self.with_nested().flat_map(|p| &p.ops).map(|op| op.seq);
        let (min, max) = seqs.fold(None, |acc: Option<(usize, usize)>, s| {
            Some(acc.map_or((s, s), |(lo, hi)| (lo.min(s), hi.max(s))))
        })?;
        Some(min..max + 1)
    }
}

/// One storage op with its key named.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotatedOp {
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub proof_bytes_added: Option<usize>,
    /// Measured local replay interval.
    pub timing: Option<frame_trace_model::MeasuredTiming>,
    /// Parent capture span ID.
    pub parent_span: Option<u64>,
    /// Position in the block.
    pub seq: usize,
    /// Operation name (`get`, `put`, ...).
    pub op: String,
    /// Child trie storage key (hex, `0x`-prefixed), if the op is on a child trie.
    pub child_trie: Option<String>,
    /// The node printed the child trie key truncated (`<head>...<tail>` in `child_trie`).
    pub child_trie_truncated: bool,
    /// Raw key or prefix (hex, `0x`-prefixed); `<head>...<tail>` when truncated.
    pub key: String,
    /// The node printed only the first and last 512 bytes of the key. Such keys are named from
    /// their prefix only and never counted as duplicates.
    pub key_truncated: bool,
    /// The key was named in full: all map keys decoded, nothing left over. False for prefixes
    /// (`clear_prefix`), truncated keys, unknown keys and keys that do not decode.
    pub key_complete: bool,
    /// Pallet, when known.
    pub pallet: Option<String>,
    /// `Pallet::Item`, a well-known key, `child:<id>` or `<unknown>`.
    pub item: String,
    /// Decoded map keys.
    pub map_keys: Vec<String>,
    /// The key's name, for example `System::Account(5Grw...)`; `…` marks missing map keys.
    pub name: String,
    /// Display name of the op, for example `get System::Account(5Grw...)`.
    pub label: String,
    /// Length of the value read or written, when known.
    pub value_len: Option<usize>,
    /// The value was printed truncated by the node.
    pub value_truncated: bool,
    /// The key was already read earlier in this phase.
    pub duplicate_read: bool,
}

fn legacy_schema_version() -> u32 {
    1
}

/// Name keys and extrinsics; without metadata, only well-known keys are named.
pub fn annotate(block: &TracedBlock, metadata: Option<&Metadata>) -> AnnotatedBlock {
    annotate_at_block(block, metadata, None)
}

/// Name hooks with an authenticated block number for OnIdle rotation.
pub fn annotate_at_block(
    block: &TracedBlock,
    metadata: Option<&Metadata>,
    number: Option<u64>,
) -> AnnotatedBlock {
    let mut observed = std::collections::BTreeMap::<(String, String), usize>::new();
    for span in &block.runtime_spans {
        let krate = span.target.split("::").next().unwrap_or(&span.target);
        *observed
            .entry((span.name.clone(), krate.into()))
            .or_default() += 1;
    }
    let mut hooks = std::collections::BTreeMap::<(String, String), usize>::new();
    AnnotatedBlock {
        schema_version: 2,
        execution: None,
        call: None,
        runtime_override: None,
        runtime_spans: block
            .runtime_spans
            .iter()
            .cloned()
            .map(|mut span| {
                if matches!(
                    span.name.as_str(),
                    "on_initialize" | "on_finalize" | "on_idle" | "on_poll"
                ) {
                    let krate = span.target.split("::").next().unwrap_or(&span.target);
                    let occurrence = hooks.entry((span.name.clone(), krate.into())).or_default();
                    let candidates = metadata
                        .map(|md| md.pallets_for_crate(krate))
                        .unwrap_or_default();
                    let ordered =
                        metadata.and_then(|md| md.hook_instances(krate, &span.name, number));
                    let pallet = ordered
                        .filter(|names| {
                            observed.get(&(span.name.clone(), krate.into())) == Some(&names.len())
                        })
                        .and_then(|names| names.get(*occurrence).cloned())
                        .or_else(|| (!candidates.is_empty()).then(|| candidates.join(" | ")))
                        .unwrap_or_else(|| crate_label(krate));
                    *occurrence += 1;
                    if krate != "runtime" && !krate.is_empty() {
                        span.name = format!("{}: {pallet}", span.name);
                    }
                }
                span
            })
            .collect(),
        timing: block.timing,
        block_number: number,
        block_hash: block.block_hash.clone(),
        parent_hash: block.parent_hash.clone(),
        skipped_events: block.skipped_events,
        totals: block.totals(),
        phases: block
            .phases
            .iter()
            .map(|phase| annotate_phase(block, phase, metadata))
            .collect(),
    }
}

fn annotate_phase(
    block: &TracedBlock,
    phase: &Phase,
    metadata: Option<&Metadata>,
) -> AnnotatedPhase {
    let decoded = metadata
        .zip(phase.extrinsic.as_deref())
        .map(|(md, bytes)| md.call_name(bytes));
    let (call, call_error) = match decoded {
        Some(Ok(call)) => (Some(call.to_string()), None),
        Some(Err(err)) => (None, Some(err.to_string())),
        None => (None, None),
    };
    let label = match &call {
        Some(call) => format!("{}: {call}", phase.kind),
        None => phase.kind.to_string(),
    };
    AnnotatedPhase {
        wall_time: phase_bounds(block, phase),
        signed: phase.extrinsic.as_ref().and_then(|bytes| {
            let mut cursor = bytes.as_slice();
            parity_scale_codec::Compact::<u32>::decode(&mut cursor).ok()?;
            cursor.first().map(|b| b & 0x80 != 0)
        }),
        kind: phase.kind.name().into(),
        extrinsic_index: phase.kind.extrinsic_index(),
        call,
        call_error,
        label,
        stats: phase.stats,
        ops: block
            .phase_ops(phase)
            .map(|op| annotate_op(op, metadata))
            .collect(),
        post_inherents: phase
            .post_inherents
            .as_ref()
            .map(|nested| Box::new(annotate_phase(block, nested, metadata))),
    }
}

fn annotate_op(op: &StorageOp, metadata: Option<&Metadata>) -> AnnotatedOp {
    let child = op.child_trie.as_ref().map(|trie| trie.head.as_slice());
    let name = match (metadata, op.key.complete()) {
        (Some(md), Some(key)) => md.key_name(child, key),
        // Never decode map keys from a truncated key: the middle is missing.
        (Some(md), None) => md.item_name(child, &op.key.head),
        (None, key) => match (child, key.and_then(well_known)) {
            (Some(trie), _) => KeyName::Child {
                trie: trie.to_vec(),
                key: op.key.head.clone(),
            },
            (None, Some(name)) => name,
            (None, None) => KeyName::Unknown { pallet: None },
        },
    };
    let map_keys = match &name {
        KeyName::Storage { keys, .. } => keys.clone(),
        _ => Vec::new(),
    };
    let label = if op.kind.has_key() {
        format!("{} {name}", op.kind)
    } else {
        op.kind.to_string()
    };
    AnnotatedOp {
        timing: op.timing,
        origin: op.origin.clone(),
        proof_bytes_added: op.proof_bytes_added,
        parent_span: op.parent_span,
        seq: op.seq,
        op: op.kind.as_str().into(),
        child_trie: op.child_trie.as_ref().map(PrintedBytes::to_hex),
        child_trie_truncated: op
            .child_trie
            .as_ref()
            .is_some_and(PrintedBytes::is_truncated),
        key: op.key.to_hex(),
        key_truncated: op.key.is_truncated(),
        key_complete: name.is_complete() && op.key_id().is_some(),
        pallet: name.pallet().map(str::to_owned),
        item: name.item(),
        map_keys,
        name: name.to_string(),
        label,
        value_len: op.value_len,
        value_truncated: op.value_truncated,
        duplicate_read: op.duplicate_read,
    }
}

fn crate_label(krate: &str) -> String {
    krate
        .replace("pallet_", "")
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
        })
        .collect()
}

#[cfg(test)]
mod naming_tests {
    use super::*;
    #[test]
    fn crate_fallbacks_strip_pallet_and_use_camel_case() {
        assert_eq!(crate_label("pallet_aura"), "Aura");
        assert_eq!(crate_label("cumulus_pallet_xcmp_queue"), "CumulusXcmpQueue");
        assert_eq!(crate_label("unknown_runtime_crate"), "UnknownRuntimeCrate");
    }
}

/// Explicit offsets on the replay clock. Duration includes compute and instrumentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseTiming {
    pub start_ns: u64,
    pub end_ns: u64,
    pub duration_ns: u64,
    /// runtime_span includes observed executive/hook spans; storage_envelope has only calls.
    pub source: String,
}
fn phase_bounds(block: &TracedBlock, phase: &Phase) -> Option<PhaseTiming> {
    let mut extrinsic = 0;
    let mut bounds: Option<(u64, u64)> = None;
    for span in &block.runtime_spans {
        let matches = match span.name.as_str() {
            "apply_extrinsic" => {
                let i = extrinsic;
                extrinsic += 1;
                phase.kind.extrinsic_index() == Some(i)
            }
            "initialize_block" | "init_block" | "initial_checks" | "on_initialize" => {
                phase.kind.name() == "initialization"
            }
            "finalize_block" | "final_checks" | "on_finalize" | "on_idle" => {
                phase.kind.name() == "finalization"
            }
            "inherents_applied" => phase.kind.name() == "post_inherents",
            _ => false,
        };
        if matches && let Some(end) = span.end_ns {
            bounds = Some(bounds.map_or((span.start_ns, end), |(lo, hi)| {
                (lo.min(span.start_ns), hi.max(end))
            }));
        }
    }
    let source = if bounds.is_some() {
        "runtime_span"
    } else {
        "storage_envelope"
    };
    // Include native calls outside executive wrappers as well. The explicit interval
    // is the envelope of observed evidence, never a fabricated full-phase timer.
    for timing in block.phase_ops(phase).filter_map(|op| op.timing) {
        let end = timing.start_ns.saturating_add(timing.duration_ns);
        bounds = Some(bounds.map_or((timing.start_ns, end), |(lo, hi)| {
            (lo.min(timing.start_ns), hi.max(end))
        }));
    }
    bounds.map(|(start_ns, end_ns)| PhaseTiming {
        start_ns,
        end_ns,
        duration_ns: end_ns.saturating_sub(start_ns),
        source: source.into(),
    })
}

/// The executed Wasm was not the authenticated parent-state `:code`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOverrideInfo {
    /// The substitute Wasm file.
    pub path: String,
    /// Blake2-256 of the executed substitute Wasm.
    pub code_hash: String,
    /// Blake2-256 of the authenticated `:code` that was not executed.
    pub chain_code_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionInfo {
    pub root_check: bool,
    pub proof_bytes: usize,
    pub rpc_calls: usize,
    pub measured_rpc_calls: usize,
    pub proof_rounds: usize,
}

#[cfg(test)]
mod phase_timing_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn phase_json_has_explicit_bounds_and_marks_storage_only_envelopes() {
        let raw = frame_trace_model::rpc::parse_trace_block(include_str!(
            "../../../fixtures/model/two-extrinsics.json"
        ))
        .unwrap();
        let mut block = TracedBlock::from_trace(&raw).unwrap();
        for op in &mut block.ops {
            op.timing = Some(frame_trace_model::MeasuredTiming {
                start_ns: 10 + op.seq as u64 * 10,
                duration_ns: 5,
            });
        }
        block.runtime_spans = vec![frame_trace_model::RuntimeSpan {
            id: 1,
            parent_id: None,
            name: "init_block".into(),
            target: "runtime".into(),
            wasm: true,
            fields: Default::default(),
            start_ns: 1,
            end_ns: Some(99),
            duration_ns: 98,
            enters: 1,
            proof_start_bytes: 0,
            proof_end_bytes: 0,
        }];
        let named = annotate(&block, None);
        let init = named.phases[0].wall_time.as_ref().unwrap();
        assert_eq!((init.start_ns, init.end_ns, init.duration_ns), (1, 99, 98));
        assert_eq!(init.source, "runtime_span");
        let other = named.phases[1].wall_time.as_ref().unwrap();
        assert_eq!(other.duration_ns, other.end_ns - other.start_ns);
        assert_eq!(other.source, "storage_envelope");
        let json = serde_json::to_value(&named).unwrap();
        assert_eq!(json["phases"][0]["wall_time"]["end_ns"], 99);
        let loaded: AnnotatedBlock = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.phases[0].wall_time.as_ref().unwrap().duration_ns, 98);
    }
}
