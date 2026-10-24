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
    /// `PostInherents` work nested inside this phase (only in the first signed extrinsic). Its
    /// ops are inside `ops` but counted in its own stats, not in this phase's.
    pub post_inherents: Option<Box<Phase>>,
}

impl Phase {
    /// Whether op `seq` belongs to this phase itself rather than to the nested phase.
    pub fn owns(&self, seq: usize) -> bool {
        self.ops.contains(&seq)
            && !self
                .post_inherents
                .as_ref()
                .is_some_and(|nested| nested.ops.contains(&seq))
    }
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
            .map(|segment| {
                let post_inherents = segment
                    .post_inherents
                    .map(|range| Box::new(phase(&mut ops, PhaseKind::PostInherents, range, None)));
                phase(&mut ops, segment.kind, segment.ops, post_inherents)
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

    /// The ops of `phase` itself, without those of a nested phase.
    pub fn phase_ops<'a>(&'a self, phase: &'a Phase) -> impl Iterator<Item = &'a StorageOp> {
        self.ops
            .get(phase.ops.clone())
            .unwrap_or_default()
            .iter()
            .filter(|op| phase.owns(op.seq))
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

/// Builds a phase: marks duplicate reads among its own ops and computes its stats.
fn phase(
    ops: &mut [StorageOp],
    kind: PhaseKind,
    range: Range<usize>,
    post_inherents: Option<Box<Phase>>,
) -> Phase {
    let mut phase = Phase {
        kind,
        ops: range.clone(),
        stats: Stats::default(),
        extrinsic: None,
        post_inherents,
    };
    let own = ops.get_mut(range).unwrap_or_default();
    mark_duplicate_reads(own.iter_mut().filter(|op| phase.owns(op.seq)));
    let own: Vec<&StorageOp> = own.iter().filter(|op| phase.owns(op.seq)).collect();
    phase.stats = Stats::of(own.iter().copied());
    phase.extrinsic = kind
        .extrinsic_index()
        .and_then(|i| extrinsic_bytes(own.iter().copied(), i));
    phase
}

/// The encoded extrinsic `index`, from the `System::ExtrinsicData` write in its phase.
/// The stored value is a SCALE `Vec<u8>` holding the encoded extrinsic.
fn extrinsic_bytes<'a>(
    ops: impl IntoIterator<Item = &'a StorageOp>,
    index: u32,
) -> Option<Vec<u8>> {
    ops.into_iter()
        .filter(|op| op.kind == OpKind::Put && op.child_trie.is_none())
        .find(|op| op.key.complete().and_then(keys::extrinsic_data_index) == Some(index))
        .and_then(|op| Vec::<u8>::decode(&mut op.value.as_deref()?).ok())
}
