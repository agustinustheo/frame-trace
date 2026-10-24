//! Serde mirror of the `state_traceBlock` response.
//!
//! These types follow `sp_rpc::tracing` (`substrate/primitives/rpc/src/tracing.rs`) field by
//! field, including the camelCase renames, so a response from any node deserialises as is. We
//! mirror them instead of depending on `sp-rpc`, which would pull in a large part of the Polkadot
//! SDK for a handful of plain structs.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Response of the `state_traceBlock` RPC (an externally tagged enum on the wire:
/// `{"blockTrace": {...}}` or `{"traceError": {"error": "..."}}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceBlockResponse {
    /// The node could not trace the block.
    TraceError(TraceError),
    /// The trace of the block.
    BlockTrace(BlockTrace),
}

/// Error variant of [`TraceBlockResponse`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceError {
    /// Error message from the node.
    pub error: String,
}

/// All spans and events recorded while re-executing one block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockTrace {
    /// Hash of the traced block (hex, `0x`-prefixed).
    pub block_hash: String,
    /// Hash of its parent.
    pub parent_hash: String,
    /// Targets the node recorded (empty means all).
    #[serde(default)]
    pub tracing_targets: String,
    /// Storage key prefixes used to filter events (empty means no filter).
    #[serde(default)]
    pub storage_keys: String,
    /// Event methods used to filter events (empty means no filter).
    #[serde(default)]
    pub methods: String,
    /// Spans, only present for runtimes built with `with-tracing`.
    #[serde(default)]
    pub spans: Vec<Span>,
    /// Events in emission order. Storage events have target `state`.
    #[serde(default)]
    pub events: Vec<Event>,
}

/// A tracing span. The RPC carries no timing for spans.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// Span id.
    pub id: u64,
    /// Explicit parent, if any.
    pub parent_id: Option<u64>,
    /// Span name.
    pub name: String,
    /// Span target (usually a module path).
    pub target: String,
    /// Whether the span came from the Wasm runtime.
    pub wasm: bool,
}

/// A tracing event. Only string values survive the conversion in `sc-tracing`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Event target, `state` for storage events.
    pub target: String,
    /// Recorded field values.
    #[serde(default)]
    pub data: Data,
    /// Explicit parent span, if any.
    pub parent_id: Option<u64>,
}

/// Values recorded on an event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    /// Field name to formatted value.
    #[serde(default)]
    pub string_values: HashMap<String, String>,
}

impl Event {
    /// The value of a string field, if recorded.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.data.string_values.get(name).map(String::as_str)
    }
}

/// Parses a `state_traceBlock` response.
///
/// Accepts the full JSON-RPC envelope (`{"jsonrpc": "2.0", "result": ...}`), the bare
/// [`TraceBlockResponse`], or a bare [`BlockTrace`].
pub fn parse_trace_block(json: &str) -> Result<BlockTrace, Error> {
    let mut value: serde_json::Value = serde_json::from_str(json)?;

    if let Some(obj) = value.as_object_mut() {
        if let Some(err) = obj.get("error").filter(|e| !e.is_null()) {
            return Err(Error::Rpc(err.to_string()));
        }
        if let Some(result) = obj.remove("result") {
            value = result;
        }
    }

    let is_response = value
        .as_object()
        .is_some_and(|o| o.contains_key("blockTrace") || o.contains_key("traceError"));
    if !is_response {
        return Ok(serde_json::from_value(value)?);
    }
    match serde_json::from_value(value)? {
        TraceBlockResponse::BlockTrace(trace) => Ok(trace),
        TraceBlockResponse::TraceError(err) => Err(Error::Trace(err.error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_envelope_response_and_bare_trace() {
        let bare = r#"{"blockHash":"0x01","parentHash":"0x00","tracingTargets":"state",
            "storageKeys":"","methods":"","spans":[],
            "events":[{"target":"state","data":{"stringValues":{"method":"Get"}},"parentId":null}]}"#;
        let wrapped = format!(r#"{{"blockTrace":{bare}}}"#);
        let envelope = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{wrapped}}}"#);

        for input in [bare.to_string(), wrapped, envelope] {
            let trace = parse_trace_block(&input).unwrap();
            assert_eq!(trace.block_hash, "0x01");
            assert_eq!(trace.events[0].field("method"), Some("Get"));
        }
    }

    #[test]
    fn surfaces_trace_and_rpc_errors() {
        let trace_err = r#"{"result":{"traceError":{"error":"boom"}}}"#;
        assert!(matches!(parse_trace_block(trace_err), Err(Error::Trace(e)) if e == "boom"));

        let rpc_err = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"unsafe"}}"#;
        assert!(matches!(parse_trace_block(rpc_err), Err(Error::Rpc(_))));

        // Some clients write `"error": null` next to a successful result.
        let ok = r#"{"jsonrpc":"2.0","id":1,"error":null,"result":{"blockTrace":{"blockHash":"0x01","parentHash":"0x00"}}}"#;
        assert_eq!(parse_trace_block(ok).unwrap().block_hash, "0x01");
    }
}
