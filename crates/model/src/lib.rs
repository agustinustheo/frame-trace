//! Block trace model for frame-trace.
//!
//! Parses a `state_traceBlock` response ([`rpc`]), turns its `state` events into ordered
//! [`StorageOp`]s, splits them into execution [`Phase`]s and computes [`Stats`]. Pure data and
//! logic: no I/O, no runtime metadata.

mod block;
pub mod keys;
mod op;
mod phase;
pub mod replay;
pub mod rpc;
pub use replay::{MeasuredTiming, ReplayCapture, ReplayEvent, RuntimeSpan, SpanTransition};
mod stats;

pub use block::{Phase, TracedBlock};
pub use op::{OpKind, PrintedBytes, StorageOp, ops_from_events};
pub use phase::{PhaseKind, Segment, phase_marker, segment};
pub use stats::{Stats, mark_duplicate_reads};

/// Errors from parsing a block trace.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The input is not valid JSON or does not have the expected shape.
    #[error("invalid trace JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON-RPC call returned an error.
    #[error("RPC error: {0}")]
    Rpc(String),
    /// The node answered with a `traceError`.
    #[error("node could not trace the block: {0}")]
    Trace(String),
    /// A storage event had an unexpected format.
    #[error("event {event_index}: {reason}")]
    Event {
        /// Index in `BlockTrace::events`.
        event_index: usize,
        /// What was wrong.
        reason: String,
    },
}
