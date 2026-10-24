//! Monotonic capture emitted by self-replay. Offsets are nanoseconds from execution start.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, TracedBlock, rpc};

/// A measured interval on the replay clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasuredTiming {
    /// Offset from execution start.
    pub start_ns: u64,
    /// Inclusive elapsed time, including instrumentation overhead.
    pub duration_ns: u64,
}

/// An observed runtime or native host span. Parents refer to capture IDs, not OTel IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSpan {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub target: String,
    pub wasm: bool,
    pub fields: BTreeMap<String, String>,
    pub start_ns: u64,
    pub end_ns: Option<u64>,
    pub duration_ns: u64,
    pub enters: u32,
    pub proof_start_bytes: usize,
    pub proof_end_bytes: usize,
}

/// Every state event, including keyless runtime API boundary events, and backend attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayEvent {
    pub seq: usize,
    pub at_ns: u64,
    pub parent_id: Option<u64>,
    pub target: String,
    pub fields: BTreeMap<String, String>,
}

/// An actual span entry or exit on the monotonic clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanTransition {
    pub id: u64,
    pub at_ns: u64,
    pub enter: bool,
}

/// Capture input alongside the node's lossy `state_traceBlock` JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayCapture {
    pub block_hash: String,
    pub parent_hash: String,
    pub events: Vec<ReplayEvent>,
    /// Flat representation of a tree: each parent precedes its children.
    pub spans: Vec<RuntimeSpan>,
    #[serde(default)]
    pub transitions: Vec<SpanTransition>,
    pub duration_ns: u64,
}

impl TracedBlock {
    /// A runtime API call has one phase, including calls which write FRAME phase markers.
    pub fn from_call(capture: &ReplayCapture) -> Result<Self, Error> {
        let mut trace = Self::from_replay(capture)?;
        for op in &mut trace.ops {
            op.duplicate_read = false;
        }
        crate::stats::mark_duplicate_reads(trace.ops.iter_mut());
        trace.phases = vec![crate::Phase {
            kind: crate::PhaseKind::Unsegmented,
            ops: 0..trace.ops.len(),
            stats: trace.totals(),
            extrinsic: None,
            post_inherents: None,
        }];
        Ok(trace)
    }

    /// Uses the same parser and phase rules for both sources. Keyless events remain in the
    /// capture; recognized keyless storage operations also participate in phases and stats.
    pub fn from_replay(capture: &ReplayCapture) -> Result<Self, Error> {
        let trace = rpc::BlockTrace {
            block_hash: capture.block_hash.clone(),
            parent_hash: capture.parent_hash.clone(),
            events: capture
                .events
                .iter()
                .map(|e| rpc::Event {
                    target: if e.target == "frame_trace_backend" {
                        "state".into()
                    } else {
                        e.target.clone()
                    },
                    data: rpc::Data {
                        string_values: e.fields.clone().into_iter().collect(),
                    },
                    parent_id: e.parent_id,
                })
                .collect(),
            ..Default::default()
        };
        let mut block = Self::from_trace(&trace)?;
        let spans: std::collections::HashMap<_, _> =
            capture.spans.iter().map(|s| (s.id, s)).collect();
        for op in &mut block.ops {
            let event = &capture.events[op.event_index];
            let host = event
                .parent_id
                .and_then(|id| spans.get(&id).copied())
                .filter(|s| event.target == "state" && s.target.starts_with("sp_io::"));
            // Ext emits its event at the END of a host call. Native host spans give the full
            // interval even for runtimes built without Wasm tracing. Backend durations describe
            // attempts, not final overlay-aware host results.
            op.timing = Some(match host {
                Some(span) => MeasuredTiming {
                    start_ns: span.start_ns,
                    duration_ns: span
                        .end_ns
                        .unwrap_or(span.start_ns)
                        .saturating_sub(span.start_ns),
                },
                None => {
                    let duration_ns = event
                        .fields
                        .get("duration_ns")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    MeasuredTiming {
                        start_ns: event.at_ns.saturating_sub(duration_ns),
                        duration_ns,
                    }
                }
            });
            op.parent_span = event.parent_id;
        }
        block.runtime_spans = capture.spans.clone();
        block.timing = Some(MeasuredTiming {
            start_ns: 0,
            duration_ns: capture.duration_ns,
        });
        Ok(block)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{OpKind, Stats};

    #[test]
    fn keyless_child_index_and_iteration_events_participate_in_model() {
        let entries: &[(&str, &[(&str, &str)])] = &[
            (
                "state",
                &[("message", "Call"), ("method", "Core_execute_block")],
            ),
            ("state", &[("message", "keyless without a method")]),
            ("state", &[("method", "ClearPrefix"), ("prefix", "aa")]),
            (
                "state",
                &[
                    ("method", "ChildClearPrefix"),
                    ("prefix", "bb"),
                    ("child_info", "01"),
                ],
            ),
            ("state", &[("method", "ChildKill"), ("child_info", "01")]),
            ("state", &[("method", "StorageRoot")]),
            (
                "state",
                &[("method", "ChildStorageRoot"), ("child_info", "01")],
            ),
            (
                "state",
                &[
                    ("method", "IndexTransaction"),
                    ("index", "1"),
                    ("tx_hash", "aa"),
                    ("size", "3"),
                ],
            ),
            (
                "state",
                &[
                    ("method", "RenewTransactionIndex"),
                    ("index", "1"),
                    ("tx_hash", "aa"),
                ],
            ),
            (
                "frame_trace_backend",
                &[
                    ("method", "BackendNextKey"),
                    ("key", "aa"),
                    ("duration_ns", "5"),
                ],
            ),
            (
                "frame_trace_backend",
                &[
                    ("method", "BackendChildNextKey"),
                    ("key", "bb"),
                    ("child_info", "01"),
                    ("duration_ns", "5"),
                ],
            ),
            (
                "frame_trace_backend",
                &[
                    ("method", "BackendIterNextKey"),
                    ("child_info", ""),
                    ("duration_ns", "5"),
                ],
            ),
            (
                "frame_trace_backend",
                &[
                    ("method", "BackendIterNextPair"),
                    ("child_info", "01"),
                    ("duration_ns", "5"),
                ],
            ),
        ];
        let capture = ReplayCapture {
            events: entries
                .iter()
                .enumerate()
                .map(|(seq, (target, fields))| ReplayEvent {
                    seq,
                    at_ns: 10 * seq as u64,
                    parent_id: None,
                    target: (*target).into(),
                    fields: fields
                        .iter()
                        .map(|(k, v)| ((*k).into(), (*v).into()))
                        .collect(),
                })
                .collect(),
            duration_ns: 200,
            ..Default::default()
        };
        let block = TracedBlock::from_replay(&capture).unwrap();
        assert_eq!(capture.events.len(), 13);
        assert_eq!(block.skipped_events, 2);
        assert_eq!(block.ops.len(), 11);
        assert_eq!(block.ops[1].child_trie.as_ref().unwrap().head, [1]);
        assert_eq!(block.ops[2].kind, OpKind::KillChild);
        assert_eq!(block.ops[4].child_trie.as_ref().unwrap().head, [1]);
        assert_eq!(block.ops[5].kind, OpKind::IndexTransaction);
        assert_eq!(block.ops[6].kind, OpKind::RenewTransactionIndex);
        assert!(block.ops[9].child_trie.is_none());
        assert!(block.ops[10].child_trie.is_some());
        assert_eq!(
            block.ops[7].timing,
            Some(MeasuredTiming {
                start_ns: 85,
                duration_ns: 5
            })
        );
        let stats = Stats::of(&block.ops);
        assert_eq!((stats.ops, stats.reads, stats.writes), (11, 0, 3));
        assert_eq!(block.phases[0].stats, stats);
    }
}
