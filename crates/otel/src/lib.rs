//! OpenTelemetry export for frame-trace.
//!
//! One trace per block, following the trace model in `SPEC.md`:
//!
//! ```text
//! block 0xabc…                                     root span, block totals
//! ├─ initialization                                phase span, phase totals
//! │  └─ get System::Number                         storage op span
//! ├─ extrinsic 1: Balances.transfer_keep_alive
//! │  ├─ get System::BlockHash(0)                   its check reads
//! │  ├─ post inherents (on_poll)                   nested, first signed extrinsic only
//! │  └─ get System::Account(5Grw…)
//! └─ finalization
//! ```
//!
//! `state_traceBlock` gives order but no timing, so ops get synthetic, evenly spaced timestamps
//! and every span carries `frame.timing = "synthetic"`. Spans are built with the OpenTelemetry
//! SDK directly (not through `tracing-opentelemetry`).

use std::time::{Duration, SystemTime};

mod delivery;

pub use delivery::{Delivery, DeliveryHandle, Tracked};
use frame_trace_decode::{AnnotatedBlock, AnnotatedOp, AnnotatedPhase};
use frame_trace_model::Stats;
use opentelemetry::{
    Array, Context, KeyValue, StringValue, Value,
    trace::{SpanKind, TraceContextExt, Tracer, TracerProvider as _},
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracer, SdkTracerProvider, SpanExporter},
};

/// `db.system.name` for every storage op.
pub const DB_SYSTEM: &str = "substrate";

/// Instrumentation scope name.
const SCOPE: &str = "frame-trace";

/// Longest span name; longer labels are cut.
const MAX_NAME_CHARS: usize = 160;

/// Errors from building or flushing an exporter.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The OTLP exporter could not be built.
    #[error("cannot build OTLP exporter: {0}")]
    Build(#[from] opentelemetry_otlp::ExporterBuildError),
    /// Export or shutdown failed.
    #[error("export failed: {0}")]
    Export(#[from] opentelemetry_sdk::error::OTelSdkError),
    /// Some spans never reached the backend: rejected by the exporter or dropped by the SDK.
    #[error(
        "{lost} of {total} spans were not exported{}",
        first_error.as_ref().map(|e| format!(" (first error: {e})")).unwrap_or_default()
    )]
    Incomplete {
        /// Spans not exported.
        lost: usize,
        /// Spans created.
        total: usize,
        /// The first export error, if the exporter reported one.
        first_error: Option<String>,
    },
}

/// How spans are laid out in time.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Start of the root span.
    pub start: SystemTime,
    /// Width of each storage op span.
    pub op_width: Duration,
}

impl Default for Timing {
    /// Starts now, 1 ms per op: readable in Jaeger, and obviously not real.
    fn default() -> Self {
        Self {
            start: SystemTime::now(),
            op_width: Duration::from_millis(1),
        }
    }
}

/// Spans [`record_block`] creates for `block`: root, phases and ops.
pub fn span_count(block: &AnnotatedBlock) -> usize {
    1 + block.all_phases().map(|p| 1 + p.ops.len()).sum::<usize>()
}

/// A tracer provider exporting through `exporter` with a batch processor whose queue holds
/// `max_spans` spans, so a whole block is exported without drops (the SDK default queue of 2048
/// would drop most of a busy block).
pub fn provider<E: SpanExporter + 'static>(exporter: E, max_spans: usize) -> SdkTracerProvider {
    let batch = BatchConfigBuilder::default()
        .with_max_queue_size(max_spans.max(2048))
        .with_max_export_batch_size(512)
        .build();
    SdkTracerProvider::builder()
        .with_span_processor(
            BatchSpanProcessor::builder(exporter)
                .with_batch_config(batch)
                .build(),
        )
        .with_max_attributes_per_span(256)
        .with_resource(Resource::builder().with_service_name(SCOPE).build())
        .build()
}

/// An OTLP/HTTP (protobuf) span exporter. `/v1/traces` is appended to `endpoint` unless present,
/// so `http://127.0.0.1:4318` works.
pub fn otlp_http_exporter(endpoint: &str) -> Result<opentelemetry_otlp::SpanExporter, Error> {
    let base = endpoint.trim_end_matches('/');
    let url = if base.ends_with("/v1/traces") {
        base.to_owned()
    } else {
        format!("{base}/v1/traces")
    };
    Ok(opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(url)
        .build()?)
}

/// Exports `block` to an OTLP/HTTP endpoint. See [`export_block`].
pub fn export_otlp(block: &AnnotatedBlock, endpoint: &str, timing: Timing) -> Result<usize, Error> {
    export_block(otlp_http_exporter(endpoint)?, block, timing)
}

/// Exports `block` through `exporter` and waits until every span was handed over. Returns the
/// number of spans, or [`Error::Incomplete`] if any span was rejected or dropped.
pub fn export_block<E: SpanExporter + 'static>(
    exporter: E,
    block: &AnnotatedBlock,
    timing: Timing,
) -> Result<usize, Error> {
    let (exporter, delivery) = Tracked::new(exporter);
    let provider = provider(exporter, span_count(block));
    let total = record_block(&provider.tracer(SCOPE), block, timing);
    let flushed = provider.force_flush();
    let shutdown = provider.shutdown();

    let delivery = delivery.get();
    if delivery.exported < total {
        return Err(Error::Incomplete {
            lost: total - delivery.exported,
            total,
            first_error: delivery
                .first_error
                .or_else(|| flushed.err().map(|e| e.to_string())),
        });
    }
    flushed?;
    shutdown?;
    Ok(total)
}

/// Records `block` as one trace on `tracer`. Returns the number of spans created.
///
/// Op `seq` occupies the time slot `[start + seq * op_width, start + (seq + 1) * op_width)`, so
/// a nested phase sits inside its parent's span at the point where it ran.
pub fn record_block(tracer: &SdkTracer, block: &AnnotatedBlock, timing: Timing) -> usize {
    let at = |slot: usize| timing.start + timing.op_width * u32::try_from(slot).unwrap_or(u32::MAX);
    let end = block
        .phases
        .iter()
        .filter_map(AnnotatedPhase::seq_span)
        .map(|span| span.end)
        .max()
        .unwrap_or(0);

    let mut attrs = vec![
        KeyValue::new("frame.timing", "synthetic"),
        KeyValue::new("frame.block.hash", block.block_hash.clone()),
        KeyValue::new("frame.block.parent_hash", block.parent_hash.clone()),
        KeyValue::new(
            "frame.block.extrinsics",
            count(
                block
                    .phases
                    .iter()
                    .filter(|p| p.extrinsic_index.is_some())
                    .count(),
            ),
        ),
        KeyValue::new("frame.trace.skipped_events", count(block.skipped_events)),
    ];
    if let Some(number) = block.block_number {
        attrs.push(KeyValue::new("frame.block.number", count(number)));
    }
    attrs.extend(stats_attributes(&block.totals));
    let root = tracer
        .span_builder(block_name(block))
        .with_kind(SpanKind::Internal)
        .with_start_time(at(0))
        .with_attributes(attrs)
        .start_with_context(tracer, &Context::new());
    let root_cx = Context::new().with_span(root);

    let spans = 1 + block
        .phases
        .iter()
        .map(|phase| record_phase(tracer, &root_cx, phase, &at))
        .sum::<usize>();
    root_cx.span().end_with_timestamp(at(end.max(1)));
    spans
}

fn record_phase(
    tracer: &SdkTracer,
    parent: &Context,
    phase: &AnnotatedPhase,
    at: &impl Fn(usize) -> SystemTime,
) -> usize {
    let slots = phase.seq_span().unwrap_or(0..0);
    let mut attrs = vec![
        KeyValue::new("frame.timing", "synthetic"),
        KeyValue::new("frame.phase", phase.kind),
    ];
    if let Some(index) = phase.extrinsic_index {
        attrs.push(KeyValue::new("frame.extrinsic.index", i64::from(index)));
    }
    if let Some(call) = &phase.call {
        attrs.push(KeyValue::new("frame.extrinsic.call", call.clone()));
    }
    attrs.extend(stats_attributes(&phase.stats));
    let span = tracer
        .span_builder(phase.label.clone())
        .with_kind(SpanKind::Internal)
        .with_start_time(at(slots.start))
        .with_attributes(attrs)
        .start_with_context(tracer, parent);
    let cx = parent.with_span(span);

    let mut spans = 1 + phase.ops.len();
    for op in &phase.ops {
        let slot = op.seq;
        let mut span = tracer
            .span_builder(truncate(&op.label))
            .with_kind(SpanKind::Internal)
            .with_start_time(at(slot))
            .with_attributes(op_attributes(op, phase.extrinsic_index))
            .start_with_context(tracer, &cx);
        opentelemetry::trace::Span::end_with_timestamp(&mut span, at(slot + 1));
    }
    if let Some(nested) = &phase.post_inherents {
        spans += record_phase(tracer, &cx, nested, at);
    }
    cx.span().end_with_timestamp(at(slots.end));
    spans
}

fn op_attributes(op: &AnnotatedOp, extrinsic_index: Option<u32>) -> Vec<KeyValue> {
    let mut attrs = vec![
        KeyValue::new("frame.timing", "synthetic"),
        KeyValue::new("db.system.name", DB_SYSTEM),
        KeyValue::new("db.operation.name", op.op),
        KeyValue::new("frame.storage.item", op.item.clone()),
        KeyValue::new("frame.storage.key", op.key.clone()),
        KeyValue::new("frame.storage.seq", count(op.seq)),
        KeyValue::new("frame.storage.duplicate_read", op.duplicate_read),
        KeyValue::new("frame.storage.key_complete", op.key_complete),
    ];
    if let Some(pallet) = &op.pallet {
        attrs.push(KeyValue::new("db.collection.name", pallet.clone()));
    }
    if !op.map_keys.is_empty() {
        let keys = op.map_keys.iter().cloned().map(StringValue::from).collect();
        attrs.push(KeyValue::new(
            "frame.storage.map_keys",
            Value::Array(Array::String(keys)),
        ));
    }
    if let Some(len) = op.value_len {
        attrs.push(KeyValue::new("frame.storage.value_len", count(len)));
    }
    if let Some(trie) = &op.child_trie {
        attrs.push(KeyValue::new("frame.storage.child_trie", trie.clone()));
    }
    if op.child_trie_truncated {
        attrs.push(KeyValue::new("frame.storage.child_trie_truncated", true));
    }
    if op.key_truncated {
        attrs.push(KeyValue::new("frame.storage.key_truncated", true));
    }
    if op.value_truncated {
        attrs.push(KeyValue::new("frame.storage.value_truncated", true));
    }
    if let Some(index) = extrinsic_index {
        attrs.push(KeyValue::new("frame.extrinsic.index", i64::from(index)));
    }
    attrs
}

fn stats_attributes(stats: &Stats) -> [KeyValue; 8] {
    [
        KeyValue::new("frame.storage.ops", count(stats.ops)),
        KeyValue::new("frame.storage.reads", count(stats.reads)),
        KeyValue::new("frame.storage.writes", count(stats.writes)),
        KeyValue::new("frame.storage.distinct_keys", count(stats.distinct_keys)),
        KeyValue::new("frame.storage.bytes_read", count(stats.bytes_read)),
        KeyValue::new("frame.storage.bytes_written", count(stats.bytes_written)),
        KeyValue::new(
            "frame.storage.duplicate_reads",
            count(stats.duplicate_reads),
        ),
        KeyValue::new("frame.storage.truncated_keys", count(stats.truncated_keys)),
    ]
}

fn block_name(block: &AnnotatedBlock) -> String {
    let hash = &block.block_hash;
    let short = match (hash.get(..10), hash.get(hash.len().saturating_sub(4)..)) {
        (Some(head), Some(tail)) if hash.len() > 16 => format!("{head}…{tail}"),
        _ => hash.clone(),
    };
    match block.block_number {
        Some(n) => format!("block #{n} ({short})"),
        None => format!("block {short}"),
    }
}

/// OTel integers are `i64`; counts never get near the limit.
fn count<T: TryInto<i64>>(n: T) -> i64 {
    n.try_into().unwrap_or(i64::MAX)
}

fn truncate(label: &str) -> String {
    match label.char_indices().nth(MAX_NAME_CHARS) {
        Some((cut, _)) => format!("{}…", &label[..cut]),
        None => label.to_owned(),
    }
}
