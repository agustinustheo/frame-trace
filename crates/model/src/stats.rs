//! Storage statistics and duplicate-read detection.

use std::collections::HashSet;

use serde::Serialize;

use crate::op::StorageOp;

/// Storage totals for a phase or a whole block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Stats {
    /// Ops, including ones that are neither reads nor writes (storage root).
    pub ops: usize,
    /// `get`, `exists` and `hash` ops.
    pub reads: usize,
    /// `put`, `clear`, `append`, `clear_prefix` and `kill_child` ops.
    pub writes: usize,
    /// Distinct `(child trie, key)` pairs touched by reads or writes. Prefixes count as keys.
    pub distinct_keys: usize,
    /// Sum of value lengths returned by reads (known lengths only).
    pub bytes_read: usize,
    /// Sum of value lengths written or appended (known lengths only).
    pub bytes_written: usize,
    /// Ops flagged `duplicate_read` (see [`mark_duplicate_reads`]).
    pub duplicate_reads: usize,
}

impl Stats {
    /// Totals of `ops`. Duplicate reads are taken from the ops' `duplicate_read` flags.
    pub fn of(ops: &[StorageOp]) -> Self {
        let mut stats = Stats {
            ops: ops.len(),
            ..Stats::default()
        };
        let mut touched = HashSet::new();
        for op in ops.iter().filter(|op| op.kind.has_key()) {
            let len = op.value_len.unwrap_or(0);
            if op.kind.is_read() {
                stats.reads += 1;
                stats.bytes_read += len;
                stats.duplicate_reads += usize::from(op.duplicate_read);
            } else {
                stats.writes += 1;
                stats.bytes_written += len;
            }
            touched.insert((op.child_trie.as_deref(), op.key.as_slice()));
        }
        stats.distinct_keys = touched.len();
        stats
    }
}

/// Flags every read of a `(child trie, key)` that was already read earlier in `ops`.
///
/// Called once per phase. A repeated read counts whatever happened to the key in between. Such
/// reads are usually served by the overlay, so they are cheap on the node, but benchmarks may
/// still charge them as database reads.
pub fn mark_duplicate_reads(ops: &mut [StorageOp]) {
    let mut seen: HashSet<(Option<Vec<u8>>, Vec<u8>)> = HashSet::new();
    for op in ops.iter_mut().filter(|op| op.kind.is_read()) {
        op.duplicate_read = !seen.insert((op.child_trie.clone(), op.key.clone()));
    }
}
