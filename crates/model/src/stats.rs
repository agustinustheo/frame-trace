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
    /// Keys the node printed truncated are not counted here (see `truncated_keys`).
    pub distinct_keys: usize,
    /// Sum of value lengths returned by reads (known lengths only).
    pub bytes_read: usize,
    /// Sum of value lengths written or appended (known lengths only).
    pub bytes_written: usize,
    /// Ops flagged `duplicate_read` (see [`mark_duplicate_reads`]).
    pub duplicate_reads: usize,
    /// Reads and writes whose key or child trie the node printed truncated. Their identity is
    /// uncertain, so they are left out of `distinct_keys` and duplicate detection.
    pub truncated_keys: usize,
}

impl Stats {
    /// Totals of `ops`. Duplicate reads are taken from the ops' `duplicate_read` flags.
    pub fn of<'a>(ops: impl IntoIterator<Item = &'a StorageOp>) -> Self {
        let mut stats = Stats::default();
        let mut touched = HashSet::new();
        for op in ops {
            stats.ops += 1;
            if !op.kind.has_key() {
                continue;
            }
            let len = op.value_len.unwrap_or(0);
            if op.kind.is_read() {
                stats.reads += 1;
                stats.bytes_read += len;
                stats.duplicate_reads += usize::from(op.duplicate_read);
            } else if op.kind.is_write() {
                stats.writes += 1;
                stats.bytes_written += len;
            }
            match op.key_id() {
                Some(id) => {
                    touched.insert(id);
                }
                None => stats.truncated_keys += 1,
            }
        }
        stats.distinct_keys = touched.len();
        stats
    }
}

/// Flags every read of a `(child trie, key)` that was already read earlier in `ops`.
///
/// Called once per phase. A repeated read counts whatever happened to the key in between. Such
/// reads are usually served by the overlay, so they are cheap on the node, but benchmarks may
/// still charge them as database reads. Reads of truncated keys are never flagged.
pub fn mark_duplicate_reads<'a>(ops: impl IntoIterator<Item = &'a mut StorageOp>) {
    let mut seen: HashSet<(Option<Vec<u8>>, Vec<u8>)> = HashSet::new();
    for op in ops.into_iter().filter(|op| op.kind.is_read()) {
        op.duplicate_read = match op.key_id() {
            Some((child, key)) => !seen.insert((child.map(<[u8]>::to_vec), key.to_vec())),
            None => false,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OpKind,
        op::{PrintedBytes, ops_from_events},
        rpc::{Data, Event},
    };

    fn ops(events: &[&[(&str, &str)]]) -> Vec<StorageOp> {
        let events: Vec<Event> = events
            .iter()
            .map(|fields| Event {
                target: "state".into(),
                data: Data {
                    string_values: fields
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                },
                parent_id: None,
            })
            .collect();
        ops_from_events(&events).unwrap().0
    }

    #[test]
    fn child_trie_reads_are_keyed_by_trie_and_key() {
        let mut ops = ops(&[
            &[
                ("method", "ChildGet"),
                ("child_info", "01"),
                ("key", "aa"),
                ("result", "Some(00)"),
            ],
            &[
                ("method", "ChildGet"),
                ("child_info", "02"),
                ("key", "aa"),
                ("result", "Some(00)"),
            ],
            &[("method", "Get"), ("key", "aa"), ("result", "None")],
            &[
                ("method", "ChildGet"),
                ("child_info", "01"),
                ("key", "aa"),
                ("result", "Some(00)"),
            ],
            &[
                ("method", "ChildPut"),
                ("child_info", "02"),
                ("key", "bb"),
                ("value", "Some(0102)"),
            ],
            &[("method", "ChildKill"), ("child_info", "03")],
        ]);
        mark_duplicate_reads(&mut ops);
        let dups: Vec<bool> = ops.iter().map(|op| op.duplicate_read).collect();
        assert_eq!(dups, [false, false, false, true, false, false]);
        assert_eq!(ops[5].kind, OpKind::KillChild);

        let stats = Stats::of(&ops);
        // (01,aa) (02,aa) (main,aa) (02,bb) (03,"")
        assert_eq!((stats.reads, stats.writes, stats.distinct_keys), (4, 2, 5));
        assert_eq!(
            (stats.bytes_read, stats.bytes_written, stats.duplicate_reads),
            (3, 2, 1)
        );
    }

    #[test]
    fn truncated_keys_are_not_duplicates_or_distinct_keys() {
        let mut ops = ops(&[
            &[("method", "Get"), ("key", "aa...bb"), ("result", "None")],
            &[("method", "Get"), ("key", "aa...bb"), ("result", "None")],
            &[("method", "Get"), ("key", "aa...cc"), ("result", "None")],
            &[
                ("method", "ChildGet"),
                ("child_info", "01...02"),
                ("key", "aa"),
                ("result", "None"),
            ],
            &[("method", "Get"), ("key", "aa"), ("result", "None")],
            &[("method", "Get"), ("key", "aa"), ("result", "None")],
        ]);
        assert_eq!(ops[0].key.tail, Some(vec![0xbb]));
        assert_eq!(ops[4].key, PrintedBytes::full([0xaa]));
        mark_duplicate_reads(&mut ops);
        let dups: Vec<bool> = ops.iter().map(|op| op.duplicate_read).collect();
        assert_eq!(dups, [false, false, false, false, false, true]);

        let stats = Stats::of(&ops);
        assert_eq!(
            (
                stats.distinct_keys,
                stats.truncated_keys,
                stats.duplicate_reads
            ),
            (1, 4, 1)
        );
    }
}
