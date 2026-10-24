//! A traced block with names: the form the exporters and the CLI consume.

use frame_trace_model::{Phase, PrintedBytes, Stats, StorageOp, TracedBlock};
use serde::Serialize;

use crate::{KeyName, Metadata, key::well_known};

/// A [`TracedBlock`] with every key and extrinsic named. Serialises to the `--json` output.
#[derive(Debug, Clone, Serialize)]
pub struct AnnotatedBlock {
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

/// One phase with its ops.
#[derive(Debug, Clone, Serialize)]
pub struct AnnotatedPhase {
    /// `initialization`, `apply_extrinsic`, `finalization` or `unsegmented`.
    pub kind: &'static str,
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
#[derive(Debug, Clone, Serialize)]
pub struct AnnotatedOp {
    /// Position in the block.
    pub seq: usize,
    /// Operation name (`get`, `put`, ...).
    pub op: &'static str,
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

/// Names the keys and extrinsics of `block`. Without metadata, keys outside the well-known set
/// are `<unknown>` and extrinsics are unnamed.
pub fn annotate(block: &TracedBlock, metadata: Option<&Metadata>) -> AnnotatedBlock {
    AnnotatedBlock {
        block_number: None,
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
        kind: phase.kind.name(),
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
        seq: op.seq,
        op: op.kind.as_str(),
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
