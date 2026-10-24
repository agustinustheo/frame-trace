//! Splitting a block's storage ops into execution phases.
//!
//! # What FRAME writes
//!
//! FRAME records the current phase in `System::ExecutionPhase` (a `Phase` enum:
//! `ApplyExtrinsic(u32)` = `0x00 ++ u32 LE`, `Finalization` = `0x01`, `Initialization` = `0x02`).
//! Each change is a `Put` to that key in the event stream. `Executive::execute_block`
//! (`substrate/frame/executive/src/lib.rs`, `substrate/frame/system/src/lib.rs`) runs:
//!
//! 1. `initialize_block`: `reset_events`, `on_runtime_upgrade` if the runtime changed,
//!    `System::initialize` (writes `ExecutionPhase = Initialization`, `:extrinsic_index = 0`),
//!    `on_initialize` hooks, then `note_finished_initialize` writes `ApplyExtrinsic(0)`.
//!    After that marker it still runs `PreInherents::pre_inherents`, `extrinsic_mode` (reads
//!    `MultiBlockMigrations::Cursor`) and `initial_checks` (reads `System::BlockHash(n - 1)`).
//! 2. Per extrinsic `i`, `do_apply_extrinsic`: `check` (signature, address lookup); for the first
//!    non-inherent, `inherents_applied` (reads `InherentsApplied`, writes it `true`, then
//!    `post_inherents` and `on_poll` or a multi-block-migration step); `note_extrinsic` (reads
//!    `:extrinsic_index`, writes `System::ExtrinsicData(i)`); the dispatch; and
//!    `note_applied_extrinsic`, which writes `:extrinsic_index = i + 1`,
//!    `ExecutionPhase = ApplyExtrinsic(i + 1)`, then kills `ExtrinsicWeightReclaimed`.
//! 3. If no extrinsic triggered it, `inherents_applied` runs now. Then `note_finished_extrinsics`
//!    takes `:extrinsic_index` (a read and a delete), writes `ExtrinsicCount`, and
//!    `ExecutionPhase = Finalization`.
//! 4. `post_transactions`, `on_idle`, `on_finalize`, `System::finalize` (deletes
//!    `ExecutionPhase`), storage root.
//!
//! # The rule
//!
//! - Ops before the first marker belong to `Initialization`.
//! - A `Put` of `ExecutionPhase` with a value is a marker. It ends the current phase and belongs
//!   to it, together with the `ExtrinsicWeightReclaimed` delete right after an
//!   `ApplyExtrinsic` marker (the cleanup of the extrinsic that just finished).
//! - An extrinsic owns everything from the end of the previous phase to its own marker,
//!   including its `check` reads (for signed extrinsics the transaction extensions' `implicit`
//!   data: `System::BlockHash(0)`, `System::Number`, `System::BlockHash(n)`), which run before
//!   its `System::ExtrinsicData(i)` write.
//! - The first signed extrinsic also runs `inherents_applied` between its `check` and that
//!   write. That work is a `PostInherents` phase nested inside the extrinsic: from the read of
//!   `InherentsApplied` before the `InherentsApplied = true` write up to the `:extrinsic_index`
//!   read before `ExtrinsicData(i)`. The extrinsic's stats leave it out.
//! - Extrinsic 0 is special: the ops after the `ApplyExtrinsic(0)` marker are the tail of
//!   `initialize_block` and go to `Initialization`; extrinsic 0 starts at its
//!   `inherents_applied` work if any, else at its `ExtrinsicData` write (with the
//!   `:extrinsic_index` read before it). Extrinsic 0 is an inherent in practice, and inherents
//!   have no `check` reads.
//! - Without an `ExtrinsicData` write (an older runtime) the whole segment is the extrinsic.
//! - The last `ApplyExtrinsic(n)` segment is the one closed by the `Finalization` marker. No
//!   extrinsic `n` exists (`n` is the extrinsic count). If it contains the
//!   `InherentsApplied = true` write (a block with only inherents), everything up to the
//!   `:extrinsic_index` take is `PostInherents`; the rest is `Finalization`.
//! - Adjacent segments of the same kind are merged and empty segments dropped.
//! - Deleting `ExecutionPhase` is not a marker.
//! - With no markers at all (not a FRAME runtime, or the trace was filtered by key), the whole
//!   block is one `Unsegmented` phase.
//!
//! Known limit: in a block without inherents, extrinsic 0 is signed and its `check` reads are
//! counted as `Initialization`.

use std::ops::Range;

use crate::{
    keys,
    op::{OpKind, StorageOp},
};

/// An execution phase of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PhaseKind {
    /// Block initialization: `on_runtime_upgrade`, `on_initialize`, initial checks.
    Initialization,
    /// Work run once all inherents are applied: `post_inherents`, `on_poll` or a multi-block
    /// migration step.
    PostInherents,
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
            PhaseKind::PostInherents => "post_inherents",
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
            PhaseKind::PostInherents => f.write_str("post inherents (on_poll)"),
            other => f.write_str(other.name()),
        }
    }
}

/// If `op` writes a new `System::ExecutionPhase`, the phase it starts.
pub fn phase_marker(op: &StorageOp) -> Option<PhaseKind> {
    if op.kind != OpKind::Put || !is_main(op, &keys::EXECUTION_PHASE) {
        return None;
    }
    PhaseKind::decode(op.value.as_deref()?)
}

/// Where a phase sits in the op list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Which phase.
    pub kind: PhaseKind,
    /// All its ops, including the nested `post_inherents` range.
    pub ops: Range<usize>,
    /// For the first signed extrinsic: the `inherents_applied` work (`PostInherents`) that runs
    /// inside it, between its `check` reads and its `ExtrinsicData` write.
    pub post_inherents: Option<Range<usize>>,
}

impl Segment {
    fn new(kind: PhaseKind, ops: Range<usize>) -> Self {
        Self {
            kind,
            ops,
            post_inherents: None,
        }
    }
}

/// Splits `ops` into phases (see the module docs for the rule). The top-level segments cover
/// `ops` in order.
pub fn segment(ops: &[StorageOp]) -> Vec<Segment> {
    // (kind, range, kind of the marker that closed it)
    let mut raw: Vec<(PhaseKind, Range<usize>, Option<PhaseKind>)> = Vec::new();
    let mut current = PhaseKind::Initialization;
    let mut start = 0;
    let mut i = 0;
    while i < ops.len() {
        let Some(next) = phase_marker(&ops[i]) else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        if next.extrinsic_index().is_some()
            && ops.get(end).is_some_and(|op| {
                op.kind == OpKind::Clear && is_main(op, &keys::EXTRINSIC_WEIGHT_RECLAIMED)
            })
        {
            end += 1;
        }
        raw.push((current, start..end, Some(next)));
        current = next;
        start = end;
        i = end;
    }
    if raw.is_empty() {
        return if ops.is_empty() {
            Vec::new()
        } else {
            vec![Segment::new(PhaseKind::Unsegmented, 0..ops.len())]
        };
    }
    raw.push((current, start..ops.len(), None));

    let mut out: Vec<Segment> = Vec::new();
    for (kind, range, closed_by) in raw {
        let pieces = match (kind, closed_by) {
            (PhaseKind::ApplyExtrinsic(_), Some(PhaseKind::Finalization)) => {
                split_before_finalization(ops, range)
            }
            (PhaseKind::ApplyExtrinsic(index), _) => split_extrinsic(ops, index, range),
            _ => vec![Segment::new(kind, range)],
        };
        for piece in pieces {
            match out.last_mut() {
                _ if piece.ops.is_empty() => {}
                Some(last)
                    if last.kind == piece.kind
                        && last.post_inherents.is_none()
                        && piece.post_inherents.is_none() =>
                {
                    last.ops.end = piece.ops.end
                }
                _ => out.push(piece),
            }
        }
    }
    out
}

/// Finds the `PostInherents` work inside the segment of extrinsic `index`, and for extrinsic 0
/// hands the `initialize_block` tail back to `Initialization`.
fn split_extrinsic(ops: &[StorageOp], index: u32, range: Range<usize>) -> Vec<Segment> {
    let extrinsic = PhaseKind::ApplyExtrinsic(index);
    let Some(data) = range.clone().find(|&i| {
        let op = &ops[i];
        op.kind == OpKind::Put
            && op.child_trie.is_none()
            && op.key.complete().and_then(keys::extrinsic_data_index) == Some(index)
    }) else {
        return vec![Segment::new(extrinsic, range)];
    };
    let body = with_preceding(ops, data, range.start, OpKind::Get, keys::EXTRINSIC_INDEX);
    let post = inherents_applied(ops, range.start..body);
    // Extrinsic 0 cannot be told apart from the `initialize_block` tail before it, so it
    // starts at its `inherents_applied` work, or else at its `ExtrinsicData` write.
    let start = match index {
        0 => post.unwrap_or(body),
        _ => range.start,
    };
    vec![
        Segment::new(PhaseKind::Initialization, range.start..start),
        Segment {
            kind: extrinsic,
            ops: start..range.end,
            post_inherents: post.map(|post| post..body),
        },
    ]
}

/// Splits the segment between the last extrinsic and the `Finalization` marker.
fn split_before_finalization(ops: &[StorageOp], range: Range<usize>) -> Vec<Segment> {
    if inherents_applied(ops, range.clone()).is_none() {
        return vec![Segment::new(PhaseKind::Finalization, range)];
    }
    // `note_finished_extrinsics` takes `:extrinsic_index`: a read followed by a delete.
    let take = range
        .clone()
        .find(|&i| {
            i > range.start
                && ops[i].kind == OpKind::Clear
                && is_main(&ops[i], keys::EXTRINSIC_INDEX)
                && ops[i - 1].kind == OpKind::Get
                && is_main(&ops[i - 1], keys::EXTRINSIC_INDEX)
        })
        .map_or(range.end, |i| i - 1);
    vec![
        Segment::new(PhaseKind::PostInherents, range.start..take),
        Segment::new(PhaseKind::Finalization, take..range.end),
    ]
}

/// Start of `inherents_applied` in `range`: the `InherentsApplied = true` write, or the read of
/// `InherentsApplied` right before it.
fn inherents_applied(ops: &[StorageOp], range: Range<usize>) -> Option<usize> {
    let put = range
        .clone()
        .find(|&i| ops[i].kind == OpKind::Put && is_main(&ops[i], &keys::INHERENTS_APPLIED))?;
    Some(with_preceding(
        ops,
        put,
        range.start,
        OpKind::Get,
        &keys::INHERENTS_APPLIED,
    ))
}

/// `at - 1` if that op (not before `floor`) is a `kind` op on `key`, else `at`.
fn with_preceding(ops: &[StorageOp], at: usize, floor: usize, kind: OpKind, key: &[u8]) -> usize {
    match at.checked_sub(1) {
        Some(prev) if prev >= floor && ops[prev].kind == kind && is_main(&ops[prev], key) => prev,
        _ => at,
    }
}

/// Whether `op` addresses exactly `key` in the main trie.
fn is_main(op: &StorageOp, key: &[u8]) -> bool {
    op.child_trie.is_none() && op.key.complete() == Some(key)
}
