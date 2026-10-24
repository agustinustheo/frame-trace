//! A traced block with names: the form the exporters and the CLI consume.

use frame_trace_model::{Stats, StorageOp, TracedBlock};
use serde::Serialize;

use crate::{KeyName, Metadata};

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
    /// Display name, for example `extrinsic 1: Balances.transfer_keep_alive`.
    pub label: String,
    /// Storage totals.
    pub stats: Stats,
    /// Ops in order.
    pub ops: Vec<AnnotatedOp>,
}

/// One storage op with its key named.
#[derive(Debug, Clone, Serialize)]
pub struct AnnotatedOp {
    /// Position in the block.
    pub seq: usize,
    /// Operation name (`get`, `put`, ...).
    pub op: &'static str,
    /// Child trie storage key (hex), if the op is on a child trie.
    pub child_trie: Option<String>,
    /// Raw key or prefix (hex, `0x`-prefixed).
    pub key: String,
    /// The node printed only the start of the key.
    pub key_truncated: bool,
    /// Pallet, when known.
    pub pallet: Option<String>,
    /// `Pallet::Item`, a well-known key, `child:<id>` or `<unknown>`.
    pub item: String,
    /// Decoded map keys.
    pub map_keys: Vec<String>,
    /// Display name, for example `get System::Account(5Grw...)`.
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
    let phases = block
        .phases
        .iter()
        .map(|phase| {
            let call = metadata
                .zip(phase.extrinsic.as_deref())
                .and_then(|(md, bytes)| md.call_name(bytes).ok())
                .map(|c| c.to_string());
            let label = match &call {
                Some(call) => format!("{}: {call}", phase.kind),
                None => phase.kind.to_string(),
            };
            AnnotatedPhase {
                kind: phase.kind.name(),
                extrinsic_index: phase.kind.extrinsic_index(),
                call,
                label,
                stats: phase.stats,
                ops: block
                    .phase_ops(phase)
                    .iter()
                    .map(|op| annotate_op(op, metadata))
                    .collect(),
            }
        })
        .collect();
    AnnotatedBlock {
        block_number: None,
        block_hash: block.block_hash.clone(),
        parent_hash: block.parent_hash.clone(),
        skipped_events: block.skipped_events,
        totals: block.totals(),
        phases,
    }
}

fn annotate_op(op: &StorageOp, metadata: Option<&Metadata>) -> AnnotatedOp {
    let name = match metadata {
        Some(md) => md.key_name(op.child_trie.as_deref(), &op.key),
        None => crate::key::well_known(&op.key).unwrap_or(match &op.child_trie {
            Some(trie) => KeyName::Child { trie: trie.clone() },
            None => KeyName::Unknown { pallet: None },
        }),
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
        child_trie: op.child_trie.as_deref().map(hex::encode),
        key: format!("0x{}", hex::encode(&op.key)),
        key_truncated: op.key_truncated,
        pallet: name.pallet().map(str::to_owned),
        item: name.item(),
        map_keys,
        label,
        value_len: op.value_len,
        value_truncated: op.value_truncated,
        duplicate_read: op.duplicate_read,
    }
}
