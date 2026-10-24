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
            "initialization"
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

    // Extrinsic 1 first runs inherents_applied (ops 16..19), as a nested phase span.
    let nested = by_name(&spans, "post inherents (on_poll)");
    assert_eq!(nested.parent_span_id, ext1.span_context.span_id());
    assert_eq!(nested.start_time, ext1.start_time);
    assert_eq!(
        (nested.start_time, nested.end_time),
        (
            start + Duration::from_millis(16),
            start + Duration::from_millis(19)
        )
    );
    let nested_ops = spans
        .iter()
        .filter(|s| s.parent_span_id == nested.span_context.span_id())
        .count();
    assert_eq!(nested_ops, 3);

    // Then its own ops, children of the extrinsic span, in consecutive synthetic slots.
    let mut ops: Vec<&SpanData> = spans
        .iter()
        .filter(|s| s.parent_span_id == ext1.span_context.span_id() && s.name != nested.name)
        .collect();
    ops.sort_by_key(|s| s.start_time);
    assert_eq!(ops.len(), 10);
    assert_eq!(ops[0].start_time, nested.end_time);
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
    assert_eq!(child.name, "get child:0xbeef[0x01]");
}

fn synthetic_block(ops: usize) -> AnnotatedBlock {
    let op = |seq| AnnotatedOp {
        timing: None,
        parent_span: None,
        seq,
        op: "get".into(),
        origin: None,
        proof_bytes_added: None,
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
        execution: None,
        call: None,
        runtime_spans: Vec::new(),
        timing: None,
        block_number: Some(7),
        block_hash: "0x01".into(),
        parent_hash: "0x00".into(),
        skipped_events: 0,
        totals: Stats::default(),
        phases: vec![AnnotatedPhase {
            kind: "unsegmented".into(),
            wall_time: None,
            signed: None,
            extrinsic_index: None,
            call: None,
            call_error: None,
            label: "unsegmented".into(),
            stats: Stats::default(),
            ops: (0..ops).map(op).collect(),
            post_inherents: None,
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

#[test]
fn measured_intervals_and_runtime_parent_tree() {
    use frame_trace_model::{MeasuredTiming, ReplayCapture, ReplayEvent, RuntimeSpan};
    let span = |id, parent_id, name: &str, wasm, start_ns, end_ns| RuntimeSpan {
        id,
        parent_id,
        name: name.into(),
        target: if wasm {
            "runtime".into()
        } else {
            "sp_io::storage".into()
        },
        wasm,
        fields: Default::default(),
        start_ns,
        end_ns: Some(end_ns),
        duration_ns: end_ns - start_ns,
        enters: 1,
        proof_start_bytes: 0,
        proof_end_bytes: 0,
    };
    let capture = ReplayCapture {
        block_hash: "0x01".into(),
        parent_hash: "0x00".into(),
        duration_ns: 120,
        spans: vec![
            span(1, None, "execute_block", true, 10, 100),
            span(2, Some(1), "dispatch", true, 20, 70),
            span(3, Some(2), "get", false, 30, 50),
        ],
        events: vec![ReplayEvent {
            seq: 0,
            at_ns: 45,
            parent_id: Some(3),
            target: "state".into(),
            fields: [
                ("method".into(), "Get".into()),
                ("key".into(), "aa".into()),
                ("result".into(), "Some(00)".into()),
            ]
            .into(),
        }],
        transitions: vec![],
    };
    let traced = TracedBlock::from_replay(&capture).unwrap();
    assert_eq!(
        traced.ops[0].timing,
        Some(MeasuredTiming {
            start_ns: 30,
            duration_ns: 20
        })
    );
    let mut block = annotate(&traced, None);
    let start = SystemTime::UNIX_EPOCH;
    let timing = Timing {
        start,
        op_width: Duration::from_secs(1),
    };
    let spans = export(&block, timing);
    assert!(
        spans
            .iter()
            .all(|s| attr(s, "frame.timing") == Some("measured".into()))
    );
    let dispatch = by_name(&spans, "dispatch");
    assert_eq!(
        dispatch.parent_span_id,
        by_name(&spans, "unsegmented").span_context.span_id()
    );
    let op = by_name(&spans, "get <unknown>");
    assert_eq!(op.parent_span_id, dispatch.span_context.span_id());
    assert_eq!(
        (op.start_time, op.end_time),
        (
            start + Duration::from_nanos(30),
            start + Duration::from_nanos(50)
        )
    );
    block.runtime_spans.clear();
    let spans = export(&block, timing);
    assert_eq!(
        by_name(&spans, "get <unknown>").parent_span_id,
        by_name(&spans, "unsegmented").span_context.span_id()
    );
}

#[test]
fn traced_batch_keeps_named_extrinsic_runtime_and_ops_in_one_tree() {
    let replay = frame_trace_replay::replay_with_session(
        frame_trace_replay::Artifact::load(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/replay/people-traced"),
        )
        .unwrap(),
        None,
        &frame_trace_replay::ReplaySession::new(None)
            .unwrap()
            .with_rpc_metadata_fallback(true),
    )
    .unwrap();
    let metadata = Metadata::from_hex(&replay.artifact.metadata).unwrap();
    let block = annotate(
        &TracedBlock::from_replay(&replay.capture).unwrap(),
        Some(&metadata),
    );
    let spans = export(&block, Timing::default());
    let ids: std::collections::HashMap<_, _> = spans
        .iter()
        .map(|s| (s.span_context.span_id(), s))
        .collect();
    let phase = spans
        .iter()
        .find(|s| s.name.contains("Utility.batch") && s.name.starts_with("extrinsic"))
        .unwrap();
    assert!(attr(phase, "frame.storage.ops").is_some());
    let batch = by_name(&spans, "batch");
    let mut current = batch;
    let mut found = false;
    for _ in 0..spans.len() {
        if current.span_context.span_id() == phase.span_context.span_id() {
            found = true;
            break;
        }
        current = ids.get(&current.parent_span_id).unwrap();
    }
    assert!(
        found,
        "batch must descend from the named Utility.batch phase"
    );
    assert!(
        spans.iter().any(|s| attr(s, "db.operation.name").is_some()
            && s.parent_span_id == batch.span_context.span_id())
            || spans.iter().any(|s| s.name.contains("transfer_keep_alive")
                && s.parent_span_id == batch.span_context.span_id())
    );
    for span in &spans {
        if attr(span, "frame.proof.marginal_bytes").is_some() {
            assert_eq!(span.name, "apply_extrinsic");
        }
        if span.name == "apply_extrinsic" {
            let parent = ids[&span.parent_span_id];
            assert_eq!(attr(parent, "frame.phase"), Some("apply_extrinsic".into()));
            assert!(
                spans
                    .iter()
                    .any(|s| s.parent_span_id == span.span_context.span_id())
            );
        }
    }
    for phase in block.phases.iter().filter(|p| p.extrinsic_index.is_some()) {
        let exported = by_name(&spans, &phase.label);
        assert!(
            spans
                .iter()
                .any(|s| s.parent_span_id == exported.span_context.span_id())
        );
    }
}

#[test]
fn measured_hooks_descend_from_the_corresponding_phase() {
    use frame_trace_model::{MeasuredTiming, RuntimeSpan};
    let mut block = synthetic_block(1);
    block.timing = Some(MeasuredTiming {
        start_ns: 0,
        duration_ns: 100,
    });
    let original = block.phases[0].clone();
    block.phases = ["initialization", "finalization"]
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            let mut phase = original.clone();
            phase.kind = kind.to_string();
            phase.label = (*kind).into();
            phase.ops[0].parent_span = Some(i as u64 + 1);
            phase.ops[0].timing = Some(MeasuredTiming {
                start_ns: i as u64 * 50 + 5,
                duration_ns: 10,
            });
            phase
        })
        .collect();
    block.runtime_spans = ["on_initialize", "on_finalize"]
        .iter()
        .enumerate()
        .map(|(i, name)| RuntimeSpan {
            id: i as u64 + 1,
            parent_id: None,
            name: (*name).into(),
            target: "runtime".into(),
            wasm: true,
            fields: Default::default(),
            start_ns: i as u64 * 50,
            end_ns: Some(i as u64 * 50 + 40),
            duration_ns: 40,
            enters: 1,
            proof_start_bytes: 0,
            proof_end_bytes: 5,
        })
        .collect();
    let spans = export(&block, Timing::default());
    for (name, phase) in [
        ("on_initialize", "initialization"),
        ("on_finalize", "finalization"),
    ] {
        let hook = by_name(&spans, name);
        assert_eq!(
            hook.parent_span_id,
            by_name(&spans, phase).span_context.span_id()
        );
        assert!(
            spans
                .iter()
                .any(|s| s.parent_span_id == hook.span_context.span_id())
        );
        assert!(attr(hook, "frame.proof.marginal_bytes").is_none());
    }
}

#[test]
fn measured_phases_without_ops_use_only_real_runtime_bounds() {
    use frame_trace_model::{MeasuredTiming, RuntimeSpan};
    let mut block = synthetic_block(0);
    block.timing = Some(MeasuredTiming {
        start_ns: 0,
        duration_ns: 100,
    });
    let mut init = block.phases[0].clone();
    init.kind = "initialization".into();
    init.label = "initialization".into();
    let mut ext = init.clone();
    ext.kind = "apply_extrinsic".into();
    ext.label = "extrinsic".into();
    ext.extrinsic_index = Some(0);
    let mut nested = init.clone();
    nested.kind = "post_inherents".into();
    nested.label = "post inherents".into();
    ext.post_inherents = Some(Box::new(nested));
    let mut empty = init.clone();
    empty.kind = "finalization".into();
    empty.label = "untimed finalization".into();
    block.phases = vec![init, ext, empty];
    block.runtime_spans = [("on_initialize", 30, 40), ("on_poll", 50, 60)]
        .into_iter()
        .enumerate()
        .map(|(i, (name, start, end))| RuntimeSpan {
            id: i as u64 + 1,
            parent_id: None,
            name: name.into(),
            target: "runtime".into(),
            wasm: true,
            fields: Default::default(),
            start_ns: start,
            end_ns: Some(end),
            duration_ns: end - start,
            enters: 1,
            proof_start_bytes: 0,
            proof_end_bytes: 0,
        })
        .collect();
    let start = SystemTime::UNIX_EPOCH;
    let spans = export(
        &block,
        Timing {
            start,
            ..Timing::default()
        },
    );
    for (name, lo, hi) in [
        ("initialization", 30, 40),
        ("extrinsic", 50, 60),
        ("post inherents", 50, 60),
    ] {
        let phase = by_name(&spans, name);
        assert_eq!(
            (phase.start_time, phase.end_time),
            (
                start + Duration::from_nanos(lo),
                start + Duration::from_nanos(hi)
            )
        );
        assert_eq!(
            attr(phase, "frame.phase.timing_available"),
            Some(true.into())
        );
    }
    let untimed = by_name(&spans, "untimed finalization");
    assert_eq!(
        (untimed.start_time, untimed.end_time),
        (start, start + Duration::from_nanos(100))
    );
    assert_eq!(
        attr(untimed, "frame.phase.timing_available"),
        Some(false.into())
    );
}

#[test]
fn hook_names_resolve_pallet_metadata_and_fall_back_to_crates() {
    use frame_trace_model::{MeasuredTiming, RuntimeSpan};
    let json = String::from_utf8(fixture("model/two-extrinsics.json")).unwrap();
    let mut traced = TracedBlock::from_trace(&parse_trace_block(&json).unwrap()).unwrap();
    traced.timing = Some(MeasuredTiming {
        start_ns: 0,
        duration_ns: 100,
    });
    traced.runtime_spans = ["on_initialize", "on_finalize", "on_idle", "on_poll"]
        .into_iter()
        .enumerate()
        .map(|(i, name)| RuntimeSpan {
            id: i as u64 + 1,
            parent_id: None,
            name: name.into(),
            target: "pallet_balances::pallet".into(),
            wasm: true,
            fields: Default::default(),
            start_ns: 10 + i as u64 * 10,
            end_ns: Some(20 + i as u64 * 10),
            duration_ns: 10,
            enters: 1,
            proof_start_bytes: 0,
            proof_end_bytes: 0,
        })
        .collect();
    let md = Metadata::from_bytes(&fixture("metadata/westend-v16.scale")).unwrap();
    assert_eq!(
        md.pallet_for_crate("pallet_balances"),
        Some("Balances".into())
    );
    let block = annotate(&traced, Some(&md));
    let spans = export(&block, Timing::default());
    for (hook, phase) in [
        ("on_initialize", "initialization"),
        ("on_finalize", "finalization"),
        ("on_idle", "finalization"),
        ("on_poll", "post inherents (on_poll)"),
    ] {
        let hook = by_name(&spans, &format!("{hook}: Balances"));
        assert_eq!(
            hook.parent_span_id,
            by_name(&spans, phase).span_context.span_id()
        );
        assert_eq!(
            attr(hook, "frame.runtime.target"),
            Some("pallet_balances::pallet".into())
        );
    }
    let fallback = annotate(&traced, None);
    assert_eq!(fallback.runtime_spans[0].name, "on_initialize: Balances");
    let artifact = frame_trace_replay::Artifact::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/replay/call-westend-nonce"),
    )
    .unwrap();
    let asset_md = Metadata::from_hex(&artifact.metadata).unwrap();
    let candidates = asset_md.pallet_for_crate("pallet_assets").unwrap();
    assert!(candidates.contains(" / "), "{candidates}");
    for span in &mut traced.runtime_spans[..3] {
        span.target = "pallet_assets::pallet".into();
        span.name = "on_initialize".into();
    }
    let instances = annotate(&traced, Some(&asset_md));
    assert_eq!(
        instances.runtime_spans[..3]
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        [
            "on_initialize: Assets",
            "on_initialize: ForeignAssets",
            "on_initialize: PoolAssets"
        ]
    );
    let people = frame_trace_replay::Artifact::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/replay/people-traced"),
    )
    .unwrap();
    let people_md = Metadata::from_hex(&people.metadata).unwrap();
    let instances = annotate(&traced, Some(&people_md));
    assert_eq!(instances.runtime_spans[0].name, "on_initialize: Assets");
    assert_eq!(instances.runtime_spans[1].name, "on_initialize: PoolAssets");
}

#[test]
fn every_storage_op_has_a_collection_including_well_known_roots_and_iteration() {
    use frame_trace_model::{ReplayCapture, ReplayEvent};
    let system = "26aa394eea5630e07c48ae0c9558cef7";
    let capture = ReplayCapture {
        duration_ns: 100,
        events: [
            (
                "state",
                "Get",
                "key",
                frame_trace_replay::rpc::hex0(b":code"),
            ),
            (
                "state",
                "Get",
                "key",
                frame_trace_replay::rpc::hex0(b":extrinsic_index"),
            ),
            (
                "state",
                "Get",
                "key",
                frame_trace_replay::rpc::hex0(b":transaction_level:"),
            ),
            ("state", "StorageRoot", "storage_root", "00".into()),
            (
                "frame_trace_backend",
                "BackendIterNextKey",
                "prefix",
                system.into(),
            ),
            (
                "frame_trace_backend",
                "BackendIterNextPair",
                "prefix",
                String::new(),
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(seq, (target, method, field, value))| ReplayEvent {
            seq,
            at_ns: seq as u64 * 10,
            parent_id: None,
            target: target.into(),
            fields: [("method".into(), method.into()), (field.into(), value)].into(),
        })
        .collect(),
        ..Default::default()
    };
    let traced = TracedBlock::from_replay(&capture).unwrap();
    let md = Metadata::from_bytes(&fixture("metadata/westend-v16.scale")).unwrap();
    let spans = export(&annotate(&traced, Some(&md)), Timing::default());
    let mut ops: Vec<_> = spans
        .iter()
        .filter(|s| attr(s, "db.operation.name").is_some())
        .collect();
    ops.sort_by_key(|s| attr(s, "frame.storage.seq").map(|v| v.to_string()));
    let collections: Vec<_> = ops
        .iter()
        .map(|s| attr(s, "db.collection.name").unwrap().to_string())
        .collect();
    assert_eq!(
        collections,
        [
            "well_known",
            "well_known",
            "well_known",
            "state",
            "System",
            "state"
        ]
    );
}

#[test]
fn runtime_call_root_and_empty_calls_export_one_trace_with_explicit_semantics() {
    for ops in [0, 2] {
        let mut trace = synthetic_block(ops);
        trace.timing = Some(frame_trace_model::MeasuredTiming {
            start_ns: 0,
            duration_ns: 1_000_000,
        });
        trace.call = Some(frame_trace_decode::AnnotatedCall {
            method: "AccountNonceApi_account_nonce".into(),
            data: "0x00".into(),
            result: "0x00000000".into(),
            decoded: Some("0".into()),
            oracle_match: Some(true),
            oracle_source: Some("live".into()),
            proof_bytes: 1234,
            state_root: "0xab".into(),
        });
        let spans = export(&trace, Timing::default());
        let root = by_name(&spans, "call AccountNonceApi_account_nonce at #7");
        assert_eq!(root.parent_span_id, SpanId::INVALID);
        assert_eq!(attr(root, "frame.trace.kind"), Some("call".into()));
        assert_eq!(
            attr(root, "frame.root_check"),
            Some("not_applicable".into())
        );
        assert_eq!(attr(root, "frame.call.oracle_match"), Some(true.into()));
        assert_eq!(attr(root, "frame.call.oracle_source"), Some("live".into()));
        assert_eq!(attr(root, "frame.proof.bytes"), Some(1234i64.into()));
        assert!(
            spans
                .iter()
                .all(|s| s.span_context.trace_id() == root.span_context.trace_id())
        );
        assert_eq!(spans.len(), span_count(&trace));
    }
}

#[test]
fn observed_read_origins_and_proof_bytes_are_otlp_attributes() {
    let mut block = synthetic_block(3);
    for (op, origin) in
        block.phases[0]
            .ops
            .iter_mut()
            .zip(["overlay", "backend_warm", "backend_cold"])
    {
        op.origin = Some(origin.into());
        op.proof_bytes_added = Some(if origin == "backend_cold" { 100 } else { 0 });
    }
    let spans = export(&block, Timing::default());
    for origin in ["overlay", "backend_warm", "backend_cold"] {
        let span = spans
            .iter()
            .find(|s| attr(s, "frame.storage.origin") == Some(origin.into()))
            .unwrap();
        assert_eq!(
            attr(span, "frame.storage.proof_bytes"),
            Some(
                if origin == "backend_cold" {
                    100i64
                } else {
                    0i64
                }
                .into()
            )
        );
    }
}
