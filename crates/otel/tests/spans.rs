//! Span tree and attributes, checked with the SDK's in-memory exporter.

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use std::time::{Duration, SystemTime};

use frame_trace_decode::{AnnotatedBlock, AnnotatedOp, AnnotatedPhase, Metadata, annotate};
use frame_trace_model::{Stats, TracedBlock, rpc::parse_trace_block};
use std::sync::atomic::{AtomicUsize, Ordering};

use frame_trace_otel::{Timing, export_block, provider, record_block, span_count};
use opentelemetry::{
    Value,
    trace::{SpanId, TracerProvider},
};
use opentelemetry_sdk::{
    error::{OTelSdkError, OTelSdkResult},
    trace::{InMemorySpanExporter, SpanData, SpanExporter},
};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture exists")
}

fn export(block: &AnnotatedBlock, timing: Timing) -> Vec<SpanData> {
    let exporter = InMemorySpanExporter::default();
    let provider = provider(exporter.clone(), span_count(block));
    let created = record_block(&provider.tracer("test"), block, timing);
    provider.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    assert_eq!(spans.len(), created);
    spans
}

fn attr(span: &SpanData, key: &str) -> Option<Value> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.clone())
}

fn by_name<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    spans
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no span named {name}"))
}

#[test]
fn builds_block_phase_op_tree() {
    let json = String::from_utf8(fixture("model/two-extrinsics.json")).unwrap();
    let block = TracedBlock::from_trace(&parse_trace_block(&json).unwrap()).unwrap();
    let md = Metadata::from_bytes(&fixture("metadata/westend-v16.scale")).unwrap();
    let annotated = annotate(&block, Some(&md));
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let spans = export(
        &annotated,
        Timing {
            start,
            op_width: Duration::from_millis(1),
        },
    );

    assert_eq!(spans.len(), 1 + 5 + 38);
    let trace_id = spans[0].span_context.trace_id();
    assert!(spans.iter().all(|s| s.span_context.trace_id() == trace_id));
    assert!(
        spans
            .iter()
            .all(|s| attr(s, "frame.timing") == Some("synthetic".into()))
    );

    let root = by_name(&spans, "block 0x11111111…1111");
    assert_eq!(root.parent_span_id, SpanId::INVALID);
    assert_eq!(attr(root, "frame.storage.reads"), Some(Value::I64(17)));
    assert_eq!(attr(root, "frame.block.extrinsics"), Some(Value::I64(2)));
    assert_eq!(
        (root.start_time, root.end_time),
        (start, start + Duration::from_millis(38))
    );

    let phases: Vec<&SpanData> = spans
        .iter()
        .filter(|s| s.parent_span_id == root.span_context.span_id())
        .collect();
    let mut names: Vec<&str> = phases.iter().map(|s| s.name.as_ref()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "extrinsic 0: Timestamp.set",
            "extrinsic 1: Balances.transfer_keep_alive",
            "finalization",
            "initialization",
            "post inherents (on_poll)"
        ]
    );

    let ext1 = by_name(&spans, "extrinsic 1: Balances.transfer_keep_alive");
    assert_eq!(attr(ext1, "frame.phase"), Some("apply_extrinsic".into()));
    assert_eq!(attr(ext1, "frame.extrinsic.index"), Some(Value::I64(1)));
    assert_eq!(
        attr(ext1, "frame.storage.duplicate_reads"),
        Some(Value::I64(2))
    );
    assert_eq!(
        attr(ext1, "frame.storage.distinct_keys"),
        Some(Value::I64(7))
    );
    assert_eq!(
        attr(
            by_name(&spans, "extrinsic 0: Timestamp.set"),
            "frame.extrinsic.call"
        ),
        Some("Timestamp.set".into())
    );

    // Ops of extrinsic 1 are its children, in order, with consecutive synthetic slots.
    let mut ops: Vec<&SpanData> = spans
        .iter()
        .filter(|s| s.parent_span_id == ext1.span_context.span_id())
        .collect();
    ops.sort_by_key(|s| s.start_time);
    assert_eq!(ops.len(), 10);
    assert_eq!(ops[0].start_time, ext1.start_time);
    assert_eq!(ops[9].end_time, ext1.end_time);
    assert!(ops.windows(2).all(|w| w[0].end_time == w[1].start_time));

    let alice = "get System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)";
    let (first, again) = (ops[2], ops[3]);
    assert_eq!((first.name.as_ref(), again.name.as_ref()), (alice, alice));
    assert_eq!(attr(first, "db.system.name"), Some("substrate".into()));
    assert_eq!(attr(first, "db.operation.name"), Some("get".into()));
    assert_eq!(attr(first, "db.collection.name"), Some("System".into()));
    assert_eq!(
        attr(first, "frame.storage.item"),
        Some("System::Account".into())
    );
    assert_eq!(attr(first, "frame.storage.value_len"), Some(Value::I64(16)));
    assert_eq!(
        attr(first, "frame.storage.duplicate_read"),
        Some(Value::Bool(false))
    );
    assert_eq!(
        attr(again, "frame.storage.duplicate_read"),
        Some(Value::Bool(true))
    );
    assert_eq!(attr(first, "frame.extrinsic.index"), Some(Value::I64(1)));
    assert_eq!(
        attr(first, "frame.storage.key_complete"),
        Some(Value::Bool(true))
    );
    let key = attr(first, "frame.storage.key")
        .unwrap()
        .as_str()
        .into_owned();
    assert!(key.starts_with("0x26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da9"));

    let child = ops
        .iter()
        .find(|s| attr(s, "frame.storage.child_trie").is_some())
        .unwrap();
    assert_eq!(child.name, "get child:0xbeef");
}

fn synthetic_block(ops: usize) -> AnnotatedBlock {
    let op = |seq| AnnotatedOp {
        seq,
        op: "get",
        child_trie: None,
        child_trie_truncated: false,
        key: "0x00".into(),
        key_truncated: false,
        key_complete: false,
        pallet: None,
        item: "<unknown>".into(),
        map_keys: Vec::new(),
        name: "<unknown>".into(),
        label: "get <unknown>".into(),
        value_len: None,
        value_truncated: false,
        duplicate_read: seq > 0,
    };
    AnnotatedBlock {
        block_number: Some(7),
        block_hash: "0x01".into(),
        parent_hash: "0x00".into(),
        skipped_events: 0,
        totals: Stats::default(),
        phases: vec![AnnotatedPhase {
            kind: "unsegmented",
            extrinsic_index: None,
            call: None,
            call_error: None,
            label: "unsegmented".into(),
            stats: Stats::default(),
            ops: (0..ops).map(op).collect(),
        }],
    }
}

#[test]
fn exports_thousands_of_ops_without_drops() {
    let spans = export(&synthetic_block(5000), Timing::default());
    assert_eq!(spans.len(), 5002);
    assert!(spans.iter().any(|s| s.name == "block #7 (0x01)"));

    // `export_block` counts what the exporter accepted (the in-memory exporter clears its
    // spans on shutdown, so the count is the check).
    let exported = export_block(
        InMemorySpanExporter::default(),
        &synthetic_block(5000),
        Timing::default(),
    );
    assert_eq!(exported.unwrap(), 5002);
}

/// Rejects the first batch, accepts the rest (like a backend answering HTTP 400 once).
#[derive(Debug, Default)]
struct RejectFirstBatch {
    calls: AtomicUsize,
}

impl SpanExporter for RejectFirstBatch {
    async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => Err(OTelSdkError::InternalFailure("HTTP 400".into())),
            _ => Ok(()),
        }
    }
}

#[test]
fn reports_spans_lost_to_export_errors() {
    let err = export_block(
        RejectFirstBatch::default(),
        &synthetic_block(2000),
        Timing::default(),
    )
    .unwrap_err();
    let frame_trace_otel::Error::Incomplete {
        lost,
        total,
        first_error,
    } = &err
    else {
        panic!("expected Incomplete, got {err}");
    };
    // The batch processor exports in batches of 512.
    assert_eq!((*lost, *total), (512, 2002));
    assert!(
        first_error
            .as_deref()
            .is_some_and(|e| e.contains("HTTP 400"))
    );
    assert!(
        err.to_string()
            .starts_with("512 of 2002 spans were not exported")
    );
}
