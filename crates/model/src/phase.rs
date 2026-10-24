//! Splitting a block's storage ops into execution phases.
//!
//! # The rule
//!
//! FRAME records the current phase in `System::ExecutionPhase` (a `Phase` enum:
//! `ApplyExtrinsic(u32)` = `0x00 ++ u32 LE`, `Finalization` = `0x01`, `Initialization` = `0x02`).
//! Each change is a `Put` to that key in the event stream. During `Executive::execute_block`
//! (`substrate/frame/executive/src/lib.rs`, `substrate/frame/system/src/lib.rs`) the sequence is:
//!
//! 1. `reset_events`, `on_runtime_upgrade` if the runtime changed, then `System::initialize`,
//!    which writes `ExecutionPhase = Initialization` and `:extrinsic_index = 0`.
//! 2. `on_initialize` hooks, then `note_finished_initialize` writes `ApplyExtrinsic(0)`.
//! 3. Extrinsic `i` runs. At its end `note_applied_extrinsic` writes `:extrinsic_index = i + 1`,
//!    `ExecutionPhase = ApplyExtrinsic(i + 1)`, then kills `ExtrinsicWeightReclaimed`.
//!    The first non-inherent extrinsic also runs `inherents_applied` (`on_poll`, multi-block
//!    migrations) before it is dispatched.
//! 4. `note_finished_extrinsics` takes `:extrinsic_index`, writes `ExtrinsicCount`, then
//!    `ExecutionPhase = Finalization`.
//! 5. `on_idle`, `on_finalize`, `System::finalize` (which deletes `ExecutionPhase`), storage root.
//!
//! So frame-trace segments like this:
//!
//! - Ops before the first marker belong to `Initialization`.
//! - A `Put` of `ExecutionPhase` with a value is a marker. The marker op itself belongs to the
//!   phase it ends (it is that phase's bookkeeping); the next op starts the new phase.
//! - The last `ApplyExtrinsic(n)` segment is the one closed by the `Finalization` marker. No
//!   extrinsic `n` exists (`n` is the extrinsic count), so its ops (the weight-reclaim cleanup
//!   of the last extrinsic, `inherents_applied` when the block has only inherents, and
//!   `note_finished_extrinsics`) are folded into `Finalization`.
//! - Adjacent segments of the same kind are merged and empty segments dropped.
//! - Deleting `ExecutionPhase` is not a marker.
//! - With no markers at all (not a FRAME runtime, or the trace was filtered by key), the whole
//!   block is one `Unsegmented` phase.
//!
//! `:extrinsic_index` is written right before each phase marker, so it carries the same
//! information; it is not used because `ExecutionPhase` also marks finalization.

use std::ops::Range;

use crate::{
    keys,
    op::{OpKind, StorageOp},
};

/// An execution phase of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PhaseKind {
    /// Block initialization: `on_runtime_upgrade`, `on_initialize`.
    Initialization,
    /// Applying the extrinsic with this index.
    ApplyExtrinsic(u32),
    /// Block finalization: `on_idle`, `on_finalize`, storage root.
    Finalization,
    /// No phase markers were found.
    Unsegmented,
}

impl PhaseKind {
    /// Stable lowercase name, used as an attribute value.
    pub fn name(self) -> &'static str {
        match self {
            PhaseKind::Initialization => "initialization",
            PhaseKind::ApplyExtrinsic(_) => "apply_extrinsic",
            PhaseKind::Finalization => "finalization",
            PhaseKind::Unsegmented => "unsegmented",
        }
    }

    /// The extrinsic index for [`PhaseKind::ApplyExtrinsic`].
    pub fn extrinsic_index(self) -> Option<u32> {
        match self {
            PhaseKind::ApplyExtrinsic(i) => Some(i),
            _ => None,
        }
    }

    /// Decodes a SCALE-encoded `frame_system::Phase`.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0, a, b, c, d] => Some(PhaseKind::ApplyExtrinsic(u32::from_le_bytes([
                *a, *b, *c, *d,
            ]))),
            [1] => Some(PhaseKind::Finalization),
            [2] => Some(PhaseKind::Initialization),
            _ => None,
        }
    }
}

impl std::fmt::Display for PhaseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PhaseKind::ApplyExtrinsic(i) => write!(f, "extrinsic {i}"),
            other => f.write_str(other.name()),
        }
    }
}

/// If `op` writes a new `System::ExecutionPhase`, the phase it starts.
pub fn phase_marker(op: &StorageOp) -> Option<PhaseKind> {
    if op.kind != OpKind::Put || op.child_trie.is_some() || op.key != keys::EXECUTION_PHASE {
        return None;
    }
    PhaseKind::decode(op.value.as_deref()?)
}

/// Splits `ops` into phases (see the module docs for the rule). The ranges cover `ops` in order.
pub fn segment(ops: &[StorageOp]) -> Vec<(PhaseKind, Range<usize>)> {
    // (kind, range, kind of the marker that closed it)
    let mut raw: Vec<(PhaseKind, Range<usize>, Option<PhaseKind>)> = Vec::new();
    let mut current = PhaseKind::Initialization;
    let mut start = 0;
    for (i, op) in ops.iter().enumerate() {
        if let Some(next) = phase_marker(op) {
            raw.push((current, start..i + 1, Some(next)));
            current = next;
            start = i + 1;
        }
    }
    if raw.is_empty() {
        return if ops.is_empty() {
            Vec::new()
        } else {
            vec![(PhaseKind::Unsegmented, 0..ops.len())]
        };
    }
    raw.push((current, start..ops.len(), None));

    let mut out: Vec<(PhaseKind, Range<usize>)> = Vec::new();
    for (kind, range, closed_by) in raw {
        let kind = match (kind, closed_by) {
            (PhaseKind::ApplyExtrinsic(_), Some(PhaseKind::Finalization)) => {
                PhaseKind::Finalization
            }
            _ => kind,
        };
        match out.last_mut() {
            Some((last, last_range)) if *last == kind => last_range.end = range.end,
            _ if range.is_empty() => {}
            _ => out.push((kind, range)),
        }
    }
    out
}
