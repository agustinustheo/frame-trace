//! A traced block: storage ops split into phases, with stats.

use std::ops::Range;

use parity_scale_codec::Decode;

use crate::{
    Error, keys,
    op::{OpKind, StorageOp, ops_from_events},
    phase::{PhaseKind, segment},
    rpc::BlockTrace,
    stats::{Stats, mark_duplicate_reads},
};

/// One execution phase of a traced block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// Which phase this is.
    pub kind: PhaseKind,
    /// Range of the phase's ops in [`TracedBlock::ops`].
    pub ops: Range<usize>,
    /// Storage totals of the phase.
    pub stats: Stats,
    /// For `ApplyExtrinsic(i)`: the encoded extrinsic (with its length prefix), taken from the
    /// `System::ExtrinsicData` write in the trace.
    pub extrinsic: Option<Vec<u8>>,
}

/// The storage ops of one block, in order, split into phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedBlock {
    /// Block hash as reported by the node.
    pub block_hash: String,
    /// Parent hash as reported by the node.
    pub parent_hash: String,
    /// All storage ops in execution order.
    pub ops: Vec<StorageOp>,
    /// Phases covering `ops` in order.
    pub phases: Vec<Phase>,
    /// Events that were not storage ops.
    pub skipped_events: usize,
}

impl TracedBlock {
    /// Builds the model from a `state_traceBlock` result.
    pub fn from_trace(trace: &BlockTrace) -> Result<Self, Error> {
        let (mut ops, skipped_events) = ops_from_events(&trace.events)?;
        let phases = segment(&ops)
            .into_iter()
            .map(|(kind, range)| {
                let phase_ops = &mut ops[range.clone()];
                mark_duplicate_reads(phase_ops);
                Phase {
                    kind,
                    stats: Stats::of(phase_ops),
                    extrinsic: kind
                        .extrinsic_index()
                        .and_then(|i| extrinsic_bytes(phase_ops, i)),
                    ops: range,
                }
            })
            .collect();
        Ok(Self {
            block_hash: trace.block_hash.clone(),
            parent_hash: trace.parent_hash.clone(),
            ops,
            phases,
            skipped_events,
        })
    }

    /// The ops of `phase`.
    pub fn phase_ops(&self, phase: &Phase) -> &[StorageOp] {
        self.ops.get(phase.ops.clone()).unwrap_or_default()
    }

    /// Totals over the whole block. `distinct_keys` is block-wide; `duplicate_reads` is the sum
    /// of per-phase duplicates.
    pub fn totals(&self) -> Stats {
        Stats::of(&self.ops)
    }

    /// Number of `ApplyExtrinsic` phases.
    pub fn extrinsic_count(&self) -> usize {
        self.phases
            .iter()
            .filter(|p| p.kind.extrinsic_index().is_some())
            .count()
    }
}

/// The encoded extrinsic `index`, from the `System::ExtrinsicData` write in its phase.
/// The stored value is a SCALE `Vec<u8>` holding the encoded extrinsic.
fn extrinsic_bytes(ops: &[StorageOp], index: u32) -> Option<Vec<u8>> {
    ops.iter()
        .filter(|op| op.kind == OpKind::Put && op.child_trie.is_none())
        .find(|op| op.key.complete().and_then(keys::extrinsic_data_index) == Some(index))
        .and_then(|op| Vec::<u8>::decode(&mut op.value.as_deref()?).ok())
}
