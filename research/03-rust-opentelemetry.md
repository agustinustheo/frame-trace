# Rust OpenTelemetry stack for frame-trace (research as of 2026-10-25)

Scope: exporting Polkadot SDK (Substrate) node traces (block / extrinsic / runtime API spans, and high-volume storage ops) to OpenTelemetry.

Method: crates.io API for versions and dates; `.crate` tarballs from static.crates.io read for Cargo.toml, CHANGELOG and source defaults (no code executed); docs.rs, opentelemetry.io and GitHub release pages via WebFetch; a few WebSearch queries. Items marked **[unverified]** came from memory or secondary sources and need a check before relying on them.

---

## 1. Versions, dates, compatibility

### Latest versions (crates.io, checked 2026-10-25)

| Crate | Latest | Released | MSRV (`rust-version`) | Notes |
|---|---|---|---|---|
| `opentelemetry` | 0.33.1 | 2026-10-09 | 1.75.0 | 0.33.0 on 2026-09-18; 0.32.0 on 2026-05-08 |
| `opentelemetry_sdk` | 0.33.1 | 2026-10-09 | 1.75.0 | 0.32.1 on 2026-05-26 |
| `opentelemetry-otlp` | 0.33.1 | 2026-10-09 | 1.75.0 | 0.31.1 (2026-03-19) added `tls-ring`/`tls-aws-lc`/`tls-provider-agnostic` |
| `opentelemetry-semantic-conventions` | 0.33.1 | 2026-10-09 | (same family) | 0.32.1 on 2026-06-26 |
| `opentelemetry-appender-tracing` | 0.33.1 | 2026-10-09 | (same family) | Bridges `tracing` events to OTel **log** records |
| `opentelemetry-stdout` | 0.33.1 | 2026-10-09 | (same family) | Dev-only stdout exporter |
| `tracing-opentelemetry` | 0.34.0 | 2026-09-23 (CHANGELOG date 2026-09-20) | 1.75.0 | Depends on `opentelemetry ^0.33.0`, `tracing-subscriber ^0.3.22` |
| `tracing-subscriber` | 0.3.23 | 2026-03-13 | 1.65.0 | |
| `tracing` | 0.1.44 | 2025-12-18 | 1.65.0 | |

### Compatibility matrix (verified from `Cargo.toml` of the tarballs)

| tracing-opentelemetry | needs opentelemetry | needs tracing-subscriber | needs tracing |
|---|---|---|---|
| 0.34.0 | `^0.33.0` (features `trace`) | `^0.3.22` | `^0.1.35` |
| 0.33.0 | `^0.32.0` | `^0.3.22` | `^0.1.35` |
| 0.32.1 | `^0.31.0` | `^0.3.22` | `^0.1.35` |

Rules that follow:
- Use **opentelemetry, opentelemetry_sdk, opentelemetry-otlp, opentelemetry-semantic-conventions, opentelemetry-appender-tracing all on the same minor (0.33.x)**. Mixing minors gives two copies of the API types and the trace provider won't be recognised.
- Use `tracing-opentelemetry = "0.34"` with `opentelemetry = "0.33"`.
- `tracing-subscriber` must be `>= 0.3.22`. Any tracing-opentelemetry release from 0.32.1 onward needs it.

### Breaking-change notes
- OTel 0.33.0 (2026-09-18): Logs and Metrics API/SDK stay stable. Logs and Metrics OTLP exporters were still RC. Distributed tracing remains **pre-stable** upstream.
- OTel 0.33.1 (2026-10-09): OTLP exporters for logs and metrics became stable. The tracing SDK/API is still not declared stable, so expect minor-version churn for traces.
- OTel 0.33.0 also split out `opentelemetry-propagator-b3` (B3 users must change imports).
- tracing-opentelemetry 0.33.0 (2026-05-18): **breaking**, fixed a deadlock when entering a span.
- tracing-opentelemetry 0.32.0 (2025-09-29): changed OTel context activation (trace/span IDs are now read from `OtelData` once built) and changed the timing code.
- tracing-opentelemetry 0.32.1 (2025-12-17): added `OpenTelemetryLayer::with_counting_event_filter` (event-count filtering layer).

### Is tracing-opentelemetry still the recommended bridge?
Yes, as far as I can find. It is maintained under `tokio-rs` and tracks upstream releases closely. The OTel project does not ship a newer official spans bridge. The official `tracing` bridge in the OTel org is `opentelemetry-appender-tracing`, which is for **logs** (events), not spans. Recommended pairing: `tracing-opentelemetry` for spans and `opentelemetry-appender-tracing` for log records if you want storage events as logs (see section 2).

### Polkadot SDK pin conflict (action needed)
The Polkadot SDK master `Cargo.toml` (fetched 2026-10-25) pins `tracing-subscriber = "=0.3.19"` (exact pin) and `tracing = "0.1.37"`. Every tracing-opentelemetry release from 0.32.1 up requires `^0.3.22`, so a node workspace on that pin cannot take the bridge as-is. Options:
- Bump the pin in the node workspace (a fork or patch), or
- Build frame-trace as a separate process that reads traces from the node (OTLP or the node's existing tracing output) and avoids linking into the node.

Confirm the exact pin in the target node's `Cargo.lock` before choosing. Older tracing-opentelemetry releases (0.30 and 0.31) were not checked.

---

## 2. Exporting spans

### Pipeline
`tracing` span/event -> `tracing_opentelemetry::OpenTelemetryLayer` -> OTel SDK `SdkTracerProvider` (span limits + sampler) -> `BatchSpanProcessor` (queue, timer) -> OTLP exporter -> Collector (recommended) -> backend.

### Batch span processor (opentelemetry_sdk 0.33.1, source-verified defaults)

| Setting | Default | Env var |
|---|---|---|
| `max_queue_size` | 2048 spans | `OTEL_BSP_MAX_QUEUE_SIZE` |
| `scheduled_delay` | 5000 ms | `OTEL_BSP_SCHEDULE_DELAY` |
| `max_export_batch_size` | 512 spans (must be <= queue size) | `OTEL_BSP_MAX_EXPORT_BATCH_SIZE` |
| `max_export_timeout` | 30000 ms | `OTEL_BSP_EXPORT_TIMEOUT` |
| `max_concurrent_exports` | 1 | `OTEL_BSP_MAX_CONCURRENT_EXPORTS` (only honoured by the async-runtime processor, behind feature `experimental_trace_batch_span_processor_with_async_runtime`) |

The spec says the queue drops spans when full. Export runs on a timer, on a full batch, or on `force_flush`. Shutdown must call `shutdown()`/`force_flush` or the last batch is lost.

Sizing for storage volume: 5,000 storage-op spans in one 6-second block fills the default 2048-span queue twice over. Such a burst would be dropped unless the queue is enlarged (for example `OTEL_BSP_MAX_QUEUE_SIZE=65536`, `OTEL_BSP_MAX_EXPORT_BATCH_SIZE=4096`). Enlarging the queue is only a patch. The real fix is section 2 below.

### Runtime needs
- The **thread-based** `BatchSpanProcessor` (default) does not need tokio. Export runs on a dedicated thread.
- OTLP over gRPC (`grpc-tonic`) uses tonic and needs a tokio runtime. A Substrate node already has tokio, so this is fine there.
- The default OTLP HTTP path is `http-proto` with `reqwest-blocking-client` (no tokio). The OTLP docs warn against combining the async HTTP clients (`reqwest-client`, `hyper-client`) with the default batch processors. Use them only with the async-runtime processor.
- Recommendation: `grpc-tonic` to a local Collector on `:4317` when running inside the node (tokio present, better throughput). Use `http-proto` if you want the smallest dependency set.

### OTLP endpoints
| Protocol | Default endpoint | Notes |
|---|---|---|
| gRPC | `http://localhost:4317` | Set `OTEL_EXPORTER_OTLP_PROTOCOL=grpc` |
| HTTP/protobuf | `http://localhost:4318` + `/v1/traces` | Signal path appended automatically when using the base env var. Builder or per-signal endpoints are used verbatim and must include the path. |
| HTTP/JSON | `http://localhost:4318/v1/traces` | Needs feature `http-json` |

Env vars: `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`, `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS`. Precedence: builder > signal env > base env.

Feature flags for `opentelemetry-otlp 0.33.1`: `trace`, `metrics`, `logs` (all default); `grpc-tonic`, `http-proto` (default), `http-json`, `reqwest-blocking-client` (default), `reqwest-client`, `reqwest-rustls`, `hyper-client`, `gzip-tonic`, `zstd-tonic`, `gzip-http`, `zstd-http`, `tls-ring`, `tls-aws-lc`, `tls-provider-agnostic`, `tls-roots`, `tls-webpki-roots`, `internal-logs`.

### Resource attributes
Set once on the `Resource` builder:
- `service.name` = `frame-trace` for the exporter, or the node's name for node traces (for example `polkadot-node`).
- `service.version`, `service.instance.id` (node peer ID or a random UUID), `deployment.environment.name`.
- Chain identity: a custom `polkadot.chain.id` (chain spec id), and `polkadot.runtime.spec_version` (updated per runtime upgrade, so traces can be grouped by runtime).

### Sampling
- Head sampling in the SDK. Keep `ParentBased` at the root so child spans follow the parent decision. The spec's default is `ParentBased(AlwaysOn)`. Use `TraceIdRatioBased` only as the root sampler, and prefer the development-status `ProbabilitySampler` (the spec says `TraceIdRatioBased` is deprecated and will not be removed before 2027-01-01). Confirm the exact sampler constructor names in opentelemetry_sdk 0.33.1 before writing the code.
- Tail sampling in the Collector (contrib `tail_sampling` processor **[unverified name]**). Keep traces that contain an error, a slow block import, or a PolkaMeter-tagged span. Drop the rest. This is the right place to decide on whole-block traces.
- Record-always option: `AlwaysRecord` turns a `DROP` into `RECORD_ONLY`, so processors see every span (useful for metrics derived from spans).

### Span limits (opentelemetry_sdk 0.33.1, source-verified)

| Limit | Default | Env var |
|---|---|---|
| events per span | 128 | `OTEL_SPAN_EVENT_COUNT_LIMIT` |
| attributes per span | 128 | `OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT` |
| links per span | 128 | `OTEL_SPAN_LINK_COUNT_LIMIT` |
| attributes per event | 128 | not an env var in 0.33 |
| attributes per link | 128 | not an env var in 0.33 |

Behaviour (checked in `span.rs`): once a span holds `max_events_per_span` events, further events are **not stored**. The SDK only increments `dropped_count`. Oldest events are not evicted.

What this means for storage reads:
- A block with 5,000 storage reads recorded as span events on the block span keeps the first 128 and silently drops the other 4,872. The count is in `dropped_events_count` but the detail is gone.
- Raising the limit (for example to 10,000) makes one block span huge and slow to export. Not recommended.
- One child span per read: 5,000 spans per block at 6 s block time is about 830 spans/s per node if every read is traced. That fills the queue and floods the backend. Not recommended for every op.

### Recommendation for modelling high-volume storage ops

Use three layers, each sized to its volume:

1. **Block span (trace) with aggregated attributes.** One span per block import, plus one per extrinsic and per runtime API call. Attributes hold counts and totals, which cost nothing per op:
   `frame.storage.reads`, `frame.storage.writes`, `frame.storage.deletes`, `frame.storage.cache_hits`, `frame.storage.cache_misses`, `frame.storage.bytes_read`, `frame.storage.bytes_written`, `frame.storage.reads.top_pallet` (string of the heaviest pallet).
   Per-pallet breakdown should go in metrics, not attributes (attribute values are capped and the key set grows).

2. **Metrics for every op (the cheap, complete view).** Use OTel counters and histograms (`opentelemetry` metrics API, stable on 0.33) with the semconv metric `db.client.operation.duration` (histogram, seconds). Attributes: `db.system.name`, `db.operation.name`, `db.collection.name` (pallet only, about 60 values), `frame.storage.cache_hit`. Do not put the item name or the key in metric attributes (unbounded cardinality).

3. **Per-op detail only on demand, bounded.** Two options:
   - tracing events inside the block span, under a dedicated target (for example `frame_storage=trace`). Events are bounded by the 128-event limit, so the block span keeps the first 128 ops for debugging. Gate with `EnvFilter`, so it costs nothing unless enabled.
   - Log records via `opentelemetry-appender-tracing` (OTLP logs are stable as of 0.33.1). Log records have no per-span event cap, can carry the block trace/span IDs for correlation, and go through a separate batch processor. Use this if you need every op with full keys.

Additionally, emit child spans only for the **slowest N ops per block** (buffer in-process for the block, export the top N when the block completes). That gives tail-based detail without a Collector.

---

## 3. Semantic conventions for DB client spans (current stable)

Source: opentelemetry.io database spans page (fetched 2026-10-25) and the `opentelemetry-semantic-conventions 0.33.1` constants.

Stability: stable, except `db.response.returned_rows`, `db.query.parameter.<key>`, most `db.system.name` well-known values, and SQL-commenter propagation, which are still in development.

Constants present in the 0.33.1 crate: `DB_SYSTEM_NAME`, `DB_OPERATION_NAME`, `DB_COLLECTION_NAME`, `DB_NAMESPACE`, `DB_QUERY_TEXT`, `DB_QUERY_SUMMARY`, `DB_QUERY_PARAMETER`, `DB_RESPONSE_RETURNED_ROWS`, `DB_RESPONSE_STATUS_CODE`, `DB_CLIENT_OPERATION_DURATION`.

Span name (first available): `{db.query.summary}`, else `{db.operation.name} {target}`, else `{target}`, else `{db.system.name}`. Span kind: `CLIENT` by default. The spec allows `INTERNAL` for in-memory databases.

Required: `db.system.name`.
Conditionally required: `db.collection.name` (single collection, when available), `db.namespace` (when available), `db.operation.name` (when one operation describes the call), `error.type` (on failure), `server.port` (non-default port with `server.address`).
Recommended: `db.query.text` (sanitise literals), `server.address`, `db.query.summary` (low cardinality).

Sensitive data: `db.query.text` should not be collected unless sanitised. Parameter values (`db.query.parameter.<key>`) are not captured by default.

### Proposed mapping for Substrate storage ops

| Substrate concept | Attribute | Value / rule |
|---|---|---|
| storage access (any) | span kind | `INTERNAL`. The trie DB is embedded (on-disk, no network hop). The spec permits INTERNAL for in-memory stores; for an embedded on-disk store the default `CLIENT` is arguable. Pick one and document it. |
| database system | `db.system.name` | custom value such as `substrate_state` **[verify against spec's allowed custom values]** |
| chain / state DB instance | `db.namespace` | chain spec id, for example `polkadot` |
| pallet | `db.collection.name` | pallet name, decoded from the first 16 bytes of the trie key (`twox128(pallet)`) using the runtime metadata. Fall back to hex if unknown. |
| storage item | `frame.storage.item` (custom) | item name, decoded from the next 16 bytes (`twox128(item)`). Not a collection name, to keep the collection low cardinality. |
| operation | `db.operation.name` | `get`, `set`, `clear`, `exists`, `next_key`, `clear_prefix`, `iter` |
| span name | `db.query.summary` | `get System.Account` (pallet.item; bounded set, so OK as summary) |
| full trie key | `frame.storage.key` (custom) | hex. Off by default, on in debug mode only. Keys contain account IDs and are high cardinality. |
| value size | `frame.storage.value_size` (custom, int) | bytes returned or written |
| cache hit | `frame.storage.cache_hit` (custom, bool) | true when served from the state cache / shared cache |
| query text | `db.query.text` | not set (no query language). Do not put raw keys or values here. |
| rows | `db.response.returned_rows` | not applicable; omit (or use for `next_key` / `iter` item counts, custom if needed) |
| failure | `error.type` | for example `missing_trie_node`, `io_error` |
| server | `server.address` / `server.port` | omit (embedded store) |

Custom attributes should use a namespace prefix (`frame.`) to stay clear of semconv names.

---

## 4. Alternatives and complements

### Local viewers (check each before depending on it)
- **Jaeger v2 (recommended one-liner):** `docker run --rm -p 16686:16686 -p 4317:4317 -p 4318:4318 jaegertracing/jaeger:2.11.0` (tag from a secondary source **[verify tag]**). UI at `http://localhost:16686`. Jaeger v1 `all-in-one` reached end of life on 2025-12-31; do not use it. Whether Jaeger v2 renders OTel span events in the span detail view: **[unverified]**; Jaeger has historically shown them as "logs" on the span.
- **Grafana Tempo** (`grafana/tempo`, ports 4317/4318, pair with Grafana for the UI). Ingestion confirmed. How span events render: **[unverified]**.
- **OpenObserve** (single container, OTLP over HTTP with basic auth): **[unverified]** from memory, not checked this session.
- **SigNoz**: full stack, heavier (ClickHouse); **[unverified]** setup detail.

### Offline and profiling
- **OTLP file**: the OTel Collector contrib `file` exporter writes OTLP JSON or protobuf to disk. Run it via the `otel/opentelemetry-collector-contrib` image. Rotation is an option; without it, the file grows **[exact option names unverified]**. This is the most robust offline path for frame-trace: frame-trace exports OTLP to a local Collector and the Collector writes the file.
- **opentelemetry-stdout 0.33.1**: prints spans to stdout. Useful for dev, not for volume.
- **tracing-chrome 0.7.2** (last release 2024-03-15): writes Chrome trace JSON, viewable in `ui.perfetto.dev` or `chrome://tracing`. Stale; check it builds against the tracing version you pin.
- **tracing-perfetto 0.1.5** (last release 2025-01-02): native Perfetto protobuf. Stale-ish.
- **tracing-tracy 0.12.0** (released 2026-08-25): live Tracy profiler UI, very low overhead, good for interactive debugging. Needs the Tracy GUI.
- **tracing-flame 0.2.0** (2021-12-17): flamegraphs from span timings. Stale.
- **pprof 0.15.0** (2025-05-27): sampling CPU profiler, flamegraph output. Complements spans (answers "where is CPU", not "which block").

Recommendation: OTLP to a local Collector with a `file` exporter for archives, plus Jaeger v2 for interactive span browsing. Use tracing-tracy only for CPU work on a single node.

---

## 5. Overhead and zero-cost design

### Numbers
I could not find reliable published per-span numbers for `tracing` plus `tracing-opentelemetry` plus an OTLP exporter. The OTel spec benchmark reports throughput per core, not latency. Treat any figure as unknown until measured. Plan: a Criterion benchmark with (a) no subscriber, (b) `tracing` fmt-off, (c) bridge with no exporter, (d) bridge with batch exporter, (e) sampled out, and a block-import replay to compare.

### Zero-cost when disabled (verified features on `tracing 0.1.44`)
1. **Compile-time level cap**: features `max_level_{off,error,warn,info,debug,trace}` and `release_max_level_*`. Macros below the cap compile to nothing. Caveat: these are features of the `tracing` crate, so cargo feature unification applies to the whole binary. Enabling one anywhere sets it for everyone.
2. **No subscriber installed**: with no subscriber, callsites get no interest and cost is close to a single atomic check **[general tracing behaviour, not measured here]**.
3. **Per-layer filters**: attach the OTel layer with `.with_filter(EnvFilter::new("frame_storage=trace,info"))` so storage events are only recorded when enabled.
4. **Runtime reload**: `tracing_subscriber::reload` (verified module in 0.3.23) swaps the filter or adds/removes the OTel layer without a restart. The subscriber rebuilds the callsite interest cache on reload.
5. **Cargo feature in frame-trace**: gate the whole bridge behind `otel = ["dep:tracing-opentelemetry", ...]`. Builds without it have no OTel code at all.
6. **Sampling**: an unsampled trace still pays the span bookkeeping cost, so filter by level or target before sampling where possible.

---

## 6. Linking across processes (W3C trace context)

- Propagator: `opentelemetry_sdk::propagation::TraceContextPropagator` (verified in 0.33.1). Set it as the global text-map propagator; check the exact global-setter name in the 0.33 docs **[verify]**.
- The PolkaMeter load generator sends JSON-RPC to the node over HTTP. The node does not propagate `traceparent` by default **[unverified; likely needs an HTTP middleware on the RPC server that extracts the context and sets the parent with `OpenTelemetrySpanExt::set_parent`]**.
- Important limit: W3C context does not travel through an extrinsic. A transaction submitted at T is included in a block later, possibly on another node. So link by correlation rather than parenting:
  - Put the extrinsic hash on both the load-test request span and the node's extrinsic span (`frame.extrinsic.hash`).
  - On the node, use `OpenTelemetrySpanExt::add_link` to link the extrinsic span to the submission span when the submission context is known to the node (RPC path).
  - Join traces by extrinsic hash in the backend when the link is unavailable.
- Baggage (`opentelemetry_sdk` baggage propagator) can carry a PolkaMeter run ID to every span of that run on the same node. It does not survive through the chain.

---

## Recommended versions (exact)

```toml
[dependencies]
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "registry"] }  # check reload feature gating before use
tracing-opentelemetry = "0.34.0"          # -> opentelemetry 0.33
opentelemetry = "0.33.1"
opentelemetry_sdk = "0.33.1"
opentelemetry-otlp = { version = "0.33.1", default-features = false, features = ["trace", "grpc-tonic"] }
opentelemetry-semantic-conventions = "0.33.1"
opentelemetry-appender-tracing = "0.33.1" # optional, for storage events as log records
```

Alternative with no tokio dependency: `opentelemetry-otlp = { version = "0.33.1", features = ["trace", "http-proto", "reqwest-blocking-client"] }`.

MSRV for all OTel crates: 1.75.0. `tracing`/`tracing-subscriber`: 1.65.0. Check the node's toolchain.

Blockers to clear first:
1. Polkadot SDK pins `tracing-subscriber =0.3.19`; tracing-opentelemetry 0.32.1 and later require `^0.3.22`.
2. Verify the RPC path for W3C propagation.
3. Verify storage-access tracing targets in the node (which `trace!` events already exist) before adding new ones.

---

## Sources

- crates.io API: `https://crates.io/api/v1/crates/<name>` (versions and dates)
- Tarballs from `https://static.crates.io/crates/...` (Cargo.toml, CHANGELOG, source defaults): opentelemetry, opentelemetry_sdk, opentelemetry-otlp, opentelemetry-semantic-conventions 0.33.1; tracing-opentelemetry 0.32.1, 0.33.0, 0.34.0; tracing 0.1.44; tracing-subscriber 0.3.23; opentelemetry-appender-tracing 0.33.1
- OTel Rust releases: `https://github.com/open-telemetry/opentelemetry-rust/releases`
- tracing-opentelemetry CHANGELOG: `https://github.com/tokio-rs/tracing-opentelemetry/blob/main/CHANGELOG.md`
- OTLP exporter docs: `https://docs.rs/opentelemetry-otlp/latest/opentelemetry_otlp/`
- BatchConfigBuilder docs: `https://docs.rs/opentelemetry_sdk/latest/opentelemetry_sdk/trace/struct.BatchConfigBuilder.html`
- tracing-opentelemetry docs: `https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/`
- OTel DB span semconv: `https://opentelemetry.io/docs/specs/semconv/database/database-spans/`
- OTel trace SDK spec: `https://opentelemetry.io/docs/specs/otel/trace/sdk/`
- Polkadot SDK root `Cargo.toml`: `https://raw.githubusercontent.com/paritytech/polkadot-sdk/master/Cargo.toml`
