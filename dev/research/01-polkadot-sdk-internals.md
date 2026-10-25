# 01 - Polkadot SDK internals relevant to frame-trace

Research reader report. Read/grep only, nothing built, nothing committed.

## Source snapshot

- Clone: `/tmp/pm-audit/polkadot-sdk` (shallow, depth 1)
- Commit: `14f57c4e28081367fbfb21cf845eb34d043865d5`
- Commit date: 2026-10-24 08:52:54 +0000
- Subject: "Bump ark-vrf version to 0.6.1 (#13528)"
- Previous clone HEAD was `783fad4`; the fetch force-updated the branch.
- Because the clone is shallow, `git log` / `git blame` are unavailable. Maintenance evidence below comes from `gh` PR/issue searches against paritytech/polkadot-sdk.

Path shorthand used below: `SDK/` = `/tmp/pm-audit/polkadot-sdk/`.

---

## Executive summary (what frame-trace can reuse)

1. **Storage operations are already logged by the host.** `sp-state-machine`'s externalities (`SDK/substrate/primitives/state-machine/src/ext.rs`) emit `trace!(target: "state", method = "Get" | "Put" | ..., key = ..., ...)` for almost every storage op. This works on any node, even with a non-tracing runtime, and is what `state_traceBlock` filters on. It is the cheapest source of "which keys, in what order". It has no timestamps and no cache/DB/proof info.
2. **`state_traceBlock` is the existing replay tool.** It re-executes one block at its parent state under a private `tracing` subscriber and returns ordered spans/events. Limitations: no timing in the output, parent links are mostly lost, and wasm spans need a `with-tracing` runtime.
3. **Timing needs the live path.** `sc_tracing::ProfilingLayer` plus a custom `TraceHandler` receives `SpanDatum` with `overall_time`, and the same layer receives the host `state` events live. It needs a custom node binary (a logger hook, as Polkadot's `runtime-metrics` already does) and `--tracing-targets` to be set.
4. **Spans inside the Wasm runtime** come from `sp_tracing` macros, compiled in only with `with-tracing`. `frame-executive` (block, extrinsic, validate) and pallet macros (every `#[pallet::call]` dispatch, `on_initialize`, `on_finalize`, `on_runtime_upgrade`, `before_all`) already emit spans. Nothing in the tree asserts on this output.
5. **Per-block DB and proof stats exist but are coarse.** `sc-client-db` `UsageInfo` gives per-block counts (state reads/writes, overlay writes, modified reads). `cache_reads` is never populated in this tree. Proof size is available per recorder via `ProofSizeExt` and the `storage_proof_size` host function (parachains only).
6. **Benchmarking DB tracking (`BenchmarkingState`) is not reusable as-is** for a live node. It only works on a genesis-built temporary DB, and the generic backend methods are `unimplemented!()`. Its `KeyTracker` logic is a good template to copy.
7. **Storage key decoding** has all the metadata needed (V15/V16 `PalletStorageMetadataIR` + `StorageHasherIR`). No in-tree metadata-driven key decoder exists; decoding crates (`subxt` 0.50, `frame-decode`) are external dependencies.
8. **Constants are not traceable from storage.** `#[pallet::constant]` values are baked into metadata and read via `Get::get()` in code. Only `parameter_types! { pub storage ... }` and the `frame-parameters` (dynamic params) pallet read storage.
9. **No OpenTelemetry anywhere in the SDK.** `polkadot-node-jaeger` and `mick-jaeger` were removed in Sept/Oct 2024. Build OTel export on our side.

---

## 1. sp-tracing: `with-tracing`, Wasm to host forwarding, spans in FRAME

### 1.1 Crate and macros

- Crate: `SDK/substrate/primitives/tracing/` (`src/lib.rs`, `src/types.rs`, `Cargo.toml`).
- Features (`SDK/substrate/primitives/tracing/Cargo.toml`): `with-tracing = ["codec/derive", "codec/full"]`; `std` pulls `tracing-subscriber`.
- Macros: `within_span!` and `enter_span!` (`src/lib.rs`, around lines 181-260). The docs say the raw `span!` family yields nothing without the feature, so use `enter_span!` / `within_span!`.
- No-op gating (`src/lib.rs`):
  ```rust
  #[cfg(all(not(feature = "std"), not(feature = "with-tracing")))]
  macro_rules! enter_span { ... => {}; ... }
  #[cfg(any(feature = "std", feature = "with-tracing"))]
  macro_rules! enter_span { ... }
  ```
  So in a `no_std` Wasm build without `with-tracing`, every span macro compiles to nothing.
- Feature propagation to enable it in a runtime:
  - `SDK/substrate/primitives/io/Cargo.toml:76`: `with-tracing = ["sp-tracing/with-tracing"]`
  - `SDK/substrate/frame/executive/Cargo.toml:43`: `with-tracing = ["sp-tracing/with-tracing"]`
  - `SDK/umbrella/Cargo.toml:584-587`: umbrella `with-tracing` enables `frame-executive`, `sp-io`, `sp-tracing`.
  - Example runtime: `SDK/substrate/bin/node/runtime/Cargo.toml:54` `with-tracing = ["polkadot-sdk/with-tracing"]`.
  - The `state_traceBlock` RPC docs (see section 2) tell operators to add `with-tracing = ["frame-executive/with-tracing", "sp-io/with-tracing"]` to the runtime.
- CI: `SDK/.github/workflows/tests-misc.yml:262` runs `cargo test ... sp-tracing --no-default-features --features=with-tracing`. That is the only tracing-feature CI line I found. No test builds a `with-tracing` runtime and asserts on its spans.

### 1.2 Wasm to host transport

Wasm side, `SDK/substrate/primitives/io/src/relay_chain.rs`:

- `#[runtime_interface(wasm_only, no_tracing)] pub trait WasmTracing` (around line 1765-1830). Methods:
  - `enabled(metadata: WasmMetadata) -> bool`: calls `tracing::dispatcher::get_default(|d| d.enabled(metadata))` on the host. Comment says implementations may cache per block.
  - `enter_span(span: WasmEntryAttributes) -> u64`: host builds a `tracing::Span`, calls `clone_span` + `enter`, returns the span id.
  - `event(event: WasmEntryAttributes)`: host calls `event.emit()`.
  - `exit(span: u64)`: host calls `d.exit(&id)`.
- Wasm-side subscriber, `mod tracing_setup` (`#[cfg(all(substrate_runtime, feature = "with-tracing"))]`), struct `PassingTracingSubscriber` implements `tracing_core::Subscriber`. `record` and `record_follows_from` are `unimplemented!`, so values cannot be recorded after span creation.
- `pub fn init_tracing()` (around line 1879) installs the global default subscriber once. It is a no-op outside `substrate_runtime + with-tracing`.
- Host function set: `SDK/substrate/primitives/io/src/relay_chain.rs:1960` `pub type SubstrateHostFunctions = (storage::HostFunctions, ..., wasm_tracing::HostFunctions, ...)`. So any node built with `sp_io::SubstrateHostFunctions` can host these calls.

Data model, `SDK/substrate/primitives/tracing/src/types.rs`:

- `WasmLevel` (ERROR..TRACE), `WasmValue` (U8..U64, Bool, Str, Formatted, Encoded(SCALE bytes)), `WasmFields`, `WasmValuesSet`.
- `WasmMetadata { name, target, level, file, line, module_path, is_span, fields }`.
- `WasmEntryAttributes { parent_id: Option<u64>, metadata: WasmMetadata, fields: WasmValuesSet }`.
- Consts `WASM_TRACE_IDENTIFIER`, `WASM_NAME_KEY`, `WASM_TARGET_KEY`. Wasm has no static callsites, so the real span name and target travel as field values under the generic identifier. The host unpacks them (`SDK/substrate/client/tracing/src/lib.rs` in `on_exit`, and `SDK/substrate/client/tracing/src/block/mod.rs` `patch_and_filter`).
- Host re-emission: `impl From<WasmEntryAttributes> for tracing::Span` (`Span::child_of`) and `WasmEntryAttributes::emit()` (`Event::child_of`).
- Parent handling (verify on a running node): the wasm side sets `parent_id` only from `attrs.parent()` / `evt.parent()`, which are explicit parents. Contextual nesting is not transported. So nested wasm spans will usually arrive with `parent_id = None`. Nesting must be reconstructed from timing, or from explicit `parent:` in the macro.

### 1.3 Where spans are emitted in FRAME

Executive, `SDK/substrate/frame/executive/src/lib.rs` (all calls first do `sp_io::init_tracing()`):

| Entry point | Span(s) | Lines (approx) |
|---|---|---|
| `initialize_block` | `enter_span!(TRACE, "init_block")` | 599-600 |
| `initial_checks` | `"initial_checks"` | 677 |
| `execute_block` | `within_span! { info_span!("execute_block", ?block) ... }` | 690-692 |
| `finalize_block` | `"finalize_block"` | 785-786 |
| `do_apply_extrinsic` | `info_span!("apply_extrinsic", ext=?HexDisplay(encoded))` | 857-868 |
| `final_checks` | `"final_checks"` | 920 |
| `validate_transaction` | `"validate_transaction"`, `"using_encoded"`, `"check"`, `"dispatch_info"`, `"validate"` | 957-991 |

Pallet macros, `SDK/substrate/frame/support/procedural/src/pallet/expand/`:

- `call.rs:538`: every `#[pallet::call]` dispatch wraps the call in `enter_span!(trace_span!(stringify!(#fn_name)))`. So each dispatch shows as a span named after the call (for example `transfer_allow_death`), with the pallet's module path as target.
- `hooks.rs`: `on_finalize` (151), `on_initialize` (201), `before_all` (220), `on_runtime_update` (243; this is the `on_runtime_upgrade` hook, the span name is spelled that way in the source).
- Not spanned: `on_idle` (no span in `on_idle_hook`, executive lines ~804-857), `inherents`, `frame_system` internals, and the `state_*` RPC paths.

Client side (native, not Wasm): `SDK/substrate/client/transaction-pool/src/common/api.rs:322-336` uses `within_span!` around `validate_transaction`. This is not part of the runtime.

### 1.4 Maintained and working?

- Compiles and is referenced by live code: the executive and pallet macros and `sp_io::init_tracing` are on master. `frame/executive/src/tests.rs` exercises `apply_extrinsic`, but no test asserts on span output.
- Test coverage I found: `sp-tracing` unit tests in `tests-misc.yml`, and `SDK/substrate/client/executor/runtime-test/src/lib.rs:283-286` calls `init_tracing`.
- Real consumer in the tree: Polkadot's `runtime-metrics` feature (`SDK/polkadot/runtime/metrics/Cargo.toml:31`: `runtime-metrics = ["frame-benchmarking", "sp-tracing/with-tracing"]`) and `SDK/polkadot/node/metrics/src/runtime/mod.rs` consume events from the wasm (see section 2.4).
- Gaps: `handle_span` in that consumer is a no-op, so spans are used less than events.

Verdict: present, compiled into the executive, and used in production by runtime-metrics. Not regression-tested end to end.

---

## 2. sc-tracing: `--tracing-targets`, `--tracing-receiver`, `state_traceBlock`

### 2.1 CLI flags

- `SDK/substrate/client/cli/src/params/shared_params.rs:86-92`:
  - `--tracing-targets <TARGETS>` (`Option<String>`), "Sets a custom profiling filter. Syntax is the same as for logging (`--log`)".
  - `--tracing-receiver <RECEIVER>` (`TracingReceiver`, default `Log`).
- `SDK/substrate/client/tracing/src/lib.rs:66`: `pub enum TracingReceiver { Log }`. Only the log receiver exists.
- Wiring: `SDK/substrate/client/cli/src/config.rs:653-655`:
  ```rust
  if let Some(tracing_targets) = self.tracing_targets()? {
      let tracing_receiver = self.tracing_receiver()?;
      logger.with_profiling(tracing_receiver, tracing_targets);
  }
  ```
  `logging/mod.rs:167-168` merges profiling targets into the env filter. `logging/mod.rs:285-302` adds `ProfilingLayer` to the global subscriber.
- Important: custom profilers (`LoggerBuilder::with_custom_profiling`, `logging/mod.rs:251`) are attached only inside the `if let Some(profiling)` branch. So a custom handler is only active when `--tracing-targets` is also passed.

### 2.2 ProfilingLayer and TraceHandler (the live path)

`SDK/substrate/client/tracing/src/lib.rs`:

- `pub struct ProfilingLayer { targets: Vec<(String, Level)>, trace_handlers: Vec<Box<dyn TraceHandler>> }` (line ~59).
- `pub trait TraceHandler: Send + Sync { fn handle_span(&self, &SpanDatum); fn handle_event(&self, &TraceEvent); }` (line ~78).
- `pub struct SpanDatum { id, parent_id, name, target, level, line, start_time: Instant, overall_time: Duration, values: Values }` (line ~102). Timing is here.
- `pub struct TraceEvent { name, target, level, values: Values, parent_id }` (line ~87). No timestamp.
- `pub struct Values { bool_values, i64_values, u64_values, string_values: FxHashMap<..> }` (line ~125).
- `impl Layer for ProfilingLayer` (around line 260-400):
  - `on_new_span`: records parent (explicit or contextual via `ctx.lookup_current()`) and values.
  - `on_enter`: sets `start_time`.
  - `on_exit`: adds to `overall_time`, unpacks `WASM_TRACE_IDENTIFIER` spans (sets `wasm=true`, restores name and target), then dispatches.
  - `on_event`: dispatches if the target and level pass `check_target`.
  - Contextual parents are resolved here, unlike the block tracer (see 2.3).
- `LogTraceHandler` (line ~393) prints `"{target}: {name}, time: {nanos}, id, parent_id, values"`.
- Attach a custom handler: `LoggerBuilder::with_custom_profiling(Box<dyn TraceHandler>)`, `SDK/substrate/client/tracing/src/logging/mod.rs:251`.

### 2.3 Block replay: `BlockExecutor` and `state_traceBlock`

Module `SDK/substrate/client/tracing/src/block/mod.rs` (409 lines):

- `const DEFAULT_TARGETS: &str = "pallet,frame,state";` (line 50). `const TRACE_TARGET = "block_trace"`. `REQUIRED_EVENT_FIELD = "method"` (line 53).
- `pub trait TracingExecuteBlock<Block>` (line ~56):
  - `execute_block(orig_hash, block)`: runs the block on the same thread.
  - `call_recorded(block_hash, method, call_data) -> Vec<u8>`: default returns `CallRecordedUnsupported`. This is newer than the rest of the module.
- `DefaultExecuteBlock` calls `runtime_api().execute_block(parent_hash, block)`.
- `BlockSubscriber` (line ~140) is a `tracing::Subscriber`:
  - `enabled`: events must carry a `method` field; spans always pass. Targets are prefix matches.
  - `new_span` records `attrs.parent()` (explicit only), not contextual.
  - `event` records `event.parent()` (explicit only).
  - `enter` and `exit` are no-ops. Durations are never computed.
  - Pushes `WASM_TRACE_IDENTIFIER` into targets so wasm spans always pass the filter.
- `BlockExecutor::new(client, block, targets, storage_keys, methods, execute_block)` and `trace_block()`:
  - Strips the seal from the header (`prepared_block`).
  - Runs `execute_block` inside `dispatcher::with_default(&dispatch, ...)` so every span and event from the host and the wasm lands in `BlockSubscriber`.
  - Post-filters events: `storage_keys` (matches the `key` string value by prefix) and `methods` (matches the `method` string value). Empty string means no filter. If `storage_keys` is set, events without a `key` value are dropped.
  - Post-filters spans by target (`patch_and_filter`).

### 2.4 RPC surface

- Trait: `SDK/substrate/client/rpc-api/src/state/mod.rs:295`: `#[method(name = "state_traceBlock", blocking, with_extensions)] fn trace_block(block: Hash, targets: Option<String>, storage_keys: Option<String>, methods: Option<String>) -> Result<sp_rpc::tracing::TraceBlockResponse, Error>`.
- Doc comment (same file, lines ~143-290):
  - Node requirements: fully synced archive node, tracing-enabled Wasm for the runtime versions you want.
  - "Storage events primarily come from `primitives/state-machine/src/ext.rs`."
  - Maximum payload 15 MB; the doc suggests narrowing with targets and storage keys.
  - Node flags: `--state-cache-size`, `--db-cache`.
- Impl: `SDK/substrate/client/rpc/src/state/state_full.rs:481-498`: `BlockExecutor::new(...).trace_block()`, errors mapped to `invalid_block`. No `DenyUnsafe` check on this method in the impl. The trait attribute `with_extensions` is present, but the impl does not use the extension. I read this as not gated. Verify on a node before relying on it.
- Output types, `SDK/substrate/primitives/rpc/src/tracing.rs`:
  - `BlockTrace { block_hash, parent_hash, tracing_targets, storage_keys, methods, spans: Vec<Span>, events: Vec<Event> }`
  - `Span { id: u64, parent_id: Option<u64>, name, target, wasm: bool }`. No timing, no values.
  - `Event { target, data: Data { string_values }, parent_id: Option<u64> }`. No name, no level, no timestamp. The conversion at `SDK/substrate/client/tracing/src/lib.rs:442-451` keeps only `string_values`.
  - `TraceError { error }`
  - `enum TraceBlockResponse { TraceError(TraceError), BlockTrace(BlockTrace) }`
- Implication: the events list is in emission order (the `Vec` is drained in order), which gives total order per block. `parent_id` is usually `None` for nested events, so per-extrinsic grouping must come from marker events (see section 3.5).

### 2.5 `state_callRecorded` (new, same family)

- Trait `SDK/substrate/client/rpc-api/src/state/mod.rs` (around line 300-312): `#[method(name = "state_callRecorded", blocking, with_extensions)] fn call_recorded(name, bytes, block) -> Bytes`. Docs say it is unsafe (denied calls report `CallRecordedDenied`) and only parachains can service it.
- Impl `state_full.rs:500-515`: uses `TracingExecuteBlock::call_recorded`, maps `CallRecordedUnsupported` to `Error::CallRecordedUnsupported`.
- Parachain implementation `SDK/cumulus/client/service/src/lib.rs` `ParachainTracingExecuteBlock` (line ~581+): registers `ProofSizeExt` (replaying a stored recording if present, else the live recorder) and calls `execute_block` or `contextual_call` with a `ProofRecorder`.
- Wiring: `cumulus/test/service`, `cumulus/polkadot-omni-node/lib/src/common/spec.rs:601` wire `ParachainTracingExecuteBlock`. The omni-node dev path (`nodes/aura.rs:478`) passes `None`. The solochain node (`SDK/substrate/bin/node/cli/src/service.rs:587`) and the relay chain (`polkadot/node/service/src/builder/mod.rs:537`) pass `None`, so there `state_callRecorded` is unsupported and `state_traceBlock` runs without proof recording.
- Upstream PR history for this area, all closed: #8798 "replay block without final checks" (closed 2025-06-09), #8845 "replay the block with proof recording" (closed 2025-07-15, not merged; cites issue #7270), #9214 "always enable proof recording to replay block" (closed 2025-09-29). The landed form is on master as above. I did not identify the PR that merged `state_callRecorded`.

### 2.6 Maintenance and known issues

- Test: `SDK/cumulus/zombienet/zombienet-sdk/tests/zombie_ci/block_bundling/tracing_block.rs:34-97` submits a transfer on a parachain collator and calls `state_traceBlock` with default args, expecting a `BlockTrace`. This is the only end-to-end test I found.
- Open issue #7270 (2025-01-21): Westend Coretime `state_traceBlock` returns "Cannot resolve a block range" for some blocks.
- Open issue #68 (since 2020): "Make clear which RPC methods are safe/unsafe". Relevant to the gating question above.
- Recent related change: PR #12374 (merged 2026-08-03) fixes block replay in pallet-revive `trace_block`/`trace_tx`, dropping traces for txs rejected with `ExhaustsResources` during replay. That shows the replay path is still being touched.
- Revive has its own runtime-API tracer, `trace_block_versioned` (`SDK/substrate/frame/revive/src/lib.rs:3349`, `.../types/src/runtime_api/payloads/trace_block.rs`), called through `state_call`. That is an EVM-level tracer and is a precedent for "trace in the runtime API, not via host events".

### 2.7 Polkadot runtime-metrics: precedent for a live custom TraceHandler

- `SDK/polkadot/node/metrics/src/runtime/mod.rs`: `impl sc_tracing::TraceHandler for RuntimeMetricsProvider` (line ~153-154). It consumes events whose `target` is `"metrics"` and decodes an encoded update (`handle_event`). `handle_span` is empty.
- `logger_hook()` (line ~218-230): `logger_builder.with_custom_profiling(Box::new(metrics_provider))`.
- Used by `SDK/polkadot/cli/src/command.rs:350`.
- Pattern to copy: a `logger_hook` that registers a handler, plus `--tracing-targets` on the command line.

---

## 3. Storage access tracing on the host

### 3.1 Storage events (`target: "state"`)

File: `SDK/substrate/primitives/state-machine/src/ext.rs`. Each op is a `trace!(target: "state", method = "...", ext_id = ..., ...)`.

| Ext method | `method` value | Fields logged | Line (approx) |
|---|---|---|---|
| `storage` | `Get` | `ext_id`, `key` (hex), `result` (hex, Option), `result_encoded` (SCALE of Option) | 155 |
| `storage_hash` | `Hash` | `ext_id`, `key`, `result` | 180 |
| `child_storage` | `ChildGet` | `ext_id`, `child_info` (storage key, hex), `key`, `result` | 200 |
| `child_storage_hash` | `ChildHash` | ... | 222 |
| `exists_storage` | `Exists` | `ext_id`, `key` | 241 |
| `exists_child_storage` | `ChildExists` | ... | 263 |
| `place_storage` | `Put` | `ext_id`, `key`, `value` (Debug of Option<hex>), `value_encoded` | 361 |
| `place_child_storage` | `ChildPut` | `ext_id`, `child_info`, `key`, `value`, `value_encoded` | 384 |
| `kill_child_storage` | `ChildKill` | `ext_id`, `child_info` | 403 |
| `clear_prefix` | `ClearPrefix` | `ext_id`, `prefix` | 422 |
| `clear_child_prefix` | `ChildClearPrefix` | `ext_id`, ... | 451 |
| `storage_append` | `Append` | `ext_id`, `key`, `value` (appended item) | 471 |
| `storage_root_with_version` | `StorageRoot` | ... | 496 |
| `child_storage_root_with_version` | `ChildStorageRoot` | ... | 523 |
| `storage_index_transaction` | `IndexTransaction` | ... | 536 |
| `storage_renew_transaction_index` | `RenewTransactionIndex` | ... | 554 |

Not traced:

- `next_storage_key` (line 273) and `next_child_storage_key` (line 311). Iteration is invisible. Any key discovered only through iteration (for example `clear_prefix` internals or `iter_keys`) appears only via the subsequent `Get`.
- `storage_root` (line ~486) is not traced directly; `storage_root_with_version` is.

Runtime call boundary: `SDK/substrate/primitives/state-machine/src/lib.rs` `execute_aux` (around lines 296-345) emits `trace!(target: "state", ext_id, method = %self.method, parent_hash, input, timeout, "Call")` and `trace!(target: "state", ext_id, ?result, "Return")`. So each runtime call (`state_call`, block execution, and so on) is bracketed by `Call` / `Return` events with the same `ext_id`. This is how to group ops by runtime invocation.

What is NOT in the events:
- Whether the read hit the trie cache, the in-memory overlay, or the DB. The event is emitted in `Ext` above all of those (see 3.3).
- Trie node count or proof size.
- Old value on `Put` (only the new value).
- Timing.

Caveat on field format: values are formatted with `HexDisplay` and `?` (Debug). `value = ?value.as_ref().map(HexDisplay::from)` gives a Debug string of an `Option`. Parse it, do not assume raw hex. Verify exact output on a real trace before writing a parser.

Caveat on cost: fields are evaluated only when the `state` target is enabled. `result_encoded` / `value_encoded` allocate, so keep the filter tight in production.

### 3.2 Other host targets

- `SDK/substrate/primitives/state-machine/src/basic.rs:256`, `ext.rs:431`: `warn!(target: "trie")` for refused child-key operations. These are warnings, not access logs.
- `trie_backend_essence.rs:650-840`: `warn!(target: "trie")` on DB failures only.
- `SDK/substrate/client/db/src/lib.rs:1617-2030`: `target: "db"` for canonicalization and commit. Block-level only, no keys.
- `SDK/substrate/client/tracing/src/block/mod.rs`: `target: "state_tracing"` debug lines for the replay itself.

So the only per-key host log is `target: "state"` in `ext.rs`.

### 3.3 Cache, overlay, DB, proof

- Overlay: `OverlayedChanges` lives inside `Ext` (`ext.rs`, `self.overlay.storage(key)` is consulted first). `StateMachineStats` (`SDK/substrate/primitives/state-machine/src/stats.rs:65-79`) counts `reads_modified` (hit an overlay value) and `writes_overlay` in `RefCell<u64>`s. Not per key.
- DB and cache tallies, `SDK/substrate/primitives/state-machine/src/stats.rs:26-60` `UsageUnit { ops, bytes }`, `UsageInfo { reads, writes, nodes_writes, overlay_writes, removed_nodes, cache_reads, modified_reads, memory, started, span }`.
  - `SDK/substrate/client/db/src/record_stats_state.rs:116-135`: `RecordStatsState::storage` calls `usage.tally_key_read(key, value, false)`. Both call sites pass `cache = false`, so `cache_reads` (`stats.rs:57-64` `tally_read` cache branch) is never incremented in this tree. Treat `state_reads_cache` (`SDK/substrate/client/db/src/lib.rs:2532`) as always zero.
  - `UsageInfo` is attached per block in `sc-client-db` (`lib.rs:2516-2534`) and is also logged at import time. It is per block, not per key.
- Trie cache metrics: `SDK/substrate/primitives/trie/src/cache/metrics.rs` registers Prometheus counters `trie_cache_shared_hits`, `trie_cache_shared_fetch_attempts`, `local_hits`, `local_fetch_attempts`, and `trie_cache_shared_update_duration`, labelled by `cache_type` (node/value). They are process-global counters, not per key or per call. Use them for dashboards only.
- Proof recorder: `SDK/substrate/primitives/trie/src/recorder.rs`:
  - `pub struct Recorder<H>` (line ~157), `recorded_keys()` (line ~194): `HashMap<Root, HashMap<key, RecordedForKey>>`, the per-key trie access record, keyed by storage root (so child tries are separated).
  - `estimate_encoded_size()` (line ~242): current encoded proof size estimate.
  - `start_transaction` / `rollback_transaction` / `commit_transaction`: lets callers record speculatively.
  - Recorded keys come from the trie, so reads served by the overlay or by a fully-cached value may not appear. Verify with a test before relying on it.
- Proof size to runtime: `SDK/substrate/primitives/trie/src/proof_size_extension.rs`: `ProofSizeExt` (line 29, `decl_extension!`), `RecordingProofSizeProvider` (line 86, records every `estimate_encoded_size` answer in order), `ReplayProofSizeProvider` (line 177, replays them). Host function `storage_proof_size` in `SDK/cumulus/primitives/proof-size-hostfunction/src/lib.rs:38` returns `PROOF_RECORDING_DISABLED` if no extension is registered.
  - Recording is wired at block import when `enable_import_proof_recording` is set (`SDK/substrate/client/service/src/client/client.rs:853`), and in parachain replay (`cumulus/client/service/src/lib.rs`).
  - Recordings are stored per block in AUX by `cumulus/client/proof-size-recording/src/lib.rs` (`load_proof_size_recording` line 67, `register_proof_size_recording_cleanup` line 107).
  - For frame-trace: `storage_proof_size()` can be called from a runtime at any point, so proof-size deltas per extrinsic are possible on parachains that register the extension.

### 3.4 Benchmarking tracker vs. the live path

See section 4. Live nodes use `sc-client-db`'s normal state, which has no per-key tracker.

### 3.5 Extrinsic boundaries in the trace

Useful markers already in storage writes (executed inside the same `Ext`, so they appear as `Put` events):

- `:extrinsic_index` (`SDK/substrate/primitives/storage/src/lib.rs:219`, hex `3a65787472696e7369635f696e646578`). Written before each extrinsic (`frame_system::set_extrinsic_index` at `SDK/substrate/frame/system/src/lib.rs:2268`, `note_applied_extrinsic` at 2368, read/taken in `note_finished_extrinsics` at 2412).
- `System::ExecutionPhase` (`frame_system/src/lib.rs:2406`, `2416`, `2422`): `Phase::ApplyExtrinsic(i)` / `Phase::Finalization` / `Phase::Initialization`. The key is `twox128("System") ++ twox128("ExecutionPhase")`, so it is decodable without metadata.
- `System::ExtrinsicData` (`lib.rs:2360`): stores the encoded extrinsic per index.
- The `apply_extrinsic` span (executive, section 1.3) gives the extrinsic bytes, but as a span attribute, not a boundary in the event stream.

---

## 4. Benchmarking DB tracking

### 4.1 Host interface

- `SDK/substrate/primitives/externalities/src/lib.rs:289-327`: `Externalities` trait methods, documented "Benchmarking related functionality and shouldn't be used anywhere else!":
  - `read_write_count() -> (reads, repeat_reads, writes, repeat_writes)`
  - `reset_read_write_count()`
  - `get_whitelist()`, `set_whitelist(Vec<TrackedStorageKey>)`
  - `proof_size() -> Option<u32>` (default `None`)
  - `get_read_and_written_keys() -> Vec<(Vec<u8>, u32, u32, bool)>`
- Runtime interface: `SDK/substrate/frame/benchmarking/src/utils.rs` around lines 268-350, `#[sp_runtime_interface::runtime_interface] pub trait Benchmarking` with `wipe_db`, `commit_db`, `read_write_count`, `reset_read_write_count`, `get/set/add_to/remove_from_whitelist`, `get_read_and_written_keys`, `proof_size`. Wrapped as `frame_benchmarking::benchmarking::HostFunctions`. Included in benchmarking-cli (`SDK/substrate/utils/frame/benchmarking-cli/src/pallet/command.rs:68`), `SDK/substrate/bin/node/cli/src/service.rs:66`, `SDK/cumulus/polkadot-omni-node/lib/src/common/types.rs:40`.
- Benchmark macro usage: `SDK/substrate/frame/benchmarking/src/v1.rs:1098-1170`:
  - reset after setup (`reset_read_write_count`), so setup reads are not counted.
  - `wipe_db` deferred per repeat.
  - After the extrinsic: `commit_db()`, then `read_write_count()`, then `get_read_and_written_keys()`, which become `BenchmarkResult { reads, repeat_reads, writes, repeat_writes, proof_size, keys }`.

### 4.2 Implementation: `BenchmarkingState`

File: `SDK/substrate/client/db/src/bench.rs` (740 lines).

- `struct KeyTracker { enable_tracking, main_keys: LinkedHashMap<Vec<u8>, TrackedStorageKey>, child_keys: LinkedHashMap<Vec<u8>, LinkedHashMap<Vec<u8>, TrackedStorageKey>> }` (line 61). Insertion-ordered by first access, not by access sequence. Keys are full keys, not truncated.
- `TrackedStorageKey { key, reads, writes, whitelisted, child_trie_key }` (`SDK/substrate/primitives/storage/src/lib.rs:50-57`).
- `pub struct BenchmarkingState<Hasher>` (line 75). Constructor `new(genesis: Storage, cache_size, record_proof, enable_tracking)` (line ~130): builds an in-memory trie from genesis. This is for benchmarks, not for a live chain.
- Read path: `storage()` / `storage_hash()` / `child_storage()` / `exists_*` / `next_*_key` all call `add_read_key(child_key, key)` (lines 358-443) before delegating to the trie.
- Write path: `commit()` (line ~489-530) calls `add_write_key` for each changed key in the drained overlay, not at `set` time. So writes are counted at commit only, and in-block re-writes count once.
- Reads served by the overlay (`Ext`) do not reach `BenchmarkingState`, so they are not counted.
- Counting: `read_write_count()` (line 558) counts distinct keys read and `repeat_reads` as extra reads per key. `get_read_and_written_keys()` (line 593) truncates each key to its first 32 bytes (`prefix_length = tracker.key.len().min(32)`). With `twox128(pallet) ++ twox128(item)` that gives per-storage-item rows; map keys are dropped. Whitelisted keys are excluded.
- Proof: `proof_size()` (line 634) estimates from `proof_recorder` when `record_proof` is set.

### 4.3 Reusability

- `BenchmarkingState` is tied to `sc-client-db` internals and genesis construction. It cannot wrap a live client's state.
- The default `Backend` impl (`SDK/substrate/primitives/state-machine/src/backend.rs:357-380`) calls `unimplemented!()` for `read_write_count`, `reset_read_write_count`, `proof_size`, and `get_read_and_written_keys`. `BasicExternalities` and `ReadOnlyExternalities` do the same (`basic.rs:348-365`, `read_only.rs:211-228`). So calling the benchmarking host functions against a live node's externalities panics.
- Reusable idea: `KeyTracker::add_read_key` / `add_write_key` (bench.rs 249-330) is a small, self-contained tracker. Copy the logic, do not depend on the crate. The 32-byte prefix trick is the same idea as section 5.

---

## 5. Decoding raw keys to Pallet::Item

### 5.1 Key layout

- Storage prefix: `storage_prefix(pallet_name, storage_name) = twox_128(pallet_name) ++ twox_128(storage_name)` (`SDK/substrate/frame/support/src/storage/mod.rs:1739-1748`).
- Map entries: `prefix ++ hasher(key1) ++ key1 [++ hasher(key2) ++ key2 ...]`.
- `StorageHasherIR` (`SDK/substrate/primitives/metadata-ir/src/types.rs:412-420`): `Blake2_128, Blake2_256, Blake2_128Concat, Twox128, Twox256, Twox64Concat, Identity`.
- Decode concatenated keys with `frame_support::hash::ReversibleStorageHasher::reverse(x)` (`SDK/substrate/frame/support/src/hash.rs:73-77`, impls at lines 92, 110, 132 for `Identity`, `Twox64Concat`, `Blake2_128Concat`). Only the three concat-style hashers are reversible.

### 5.2 Where the pallet and item names come from in metadata

- `PalletStorageMetadataIR { prefix, entries }` (`types.rs:344-349`). `prefix` is `PalletInfo::name::<Pallet>()`, i.e. the name given in `construct_runtime!`, not the crate name. Generated in `SDK/substrate/frame/support/procedural/src/pallet/expand/storage.rs:921-934`.
- `StorageEntryMetadataIR { name, modifier, ty: StorageEntryTypeIR, default, docs, deprecation_info }` (`types.rs:365-375`). `name` is `Prefix::STORAGE_PREFIX` (`SDK/substrate/frame/support/src/storage/types/map.rs:501`), so `#[pallet::storage_prefix = "X"]` renames both the prefix and the metadata name, consistently.
- `StorageEntryTypeIR::Plain(ty)` or `Map { hashers: Vec<StorageHasherIR>, key, value }` (`types.rs:432-440`). `NMap` has several hashers and a tuple key. Map entries use `hashers: vec![Hasher::METADATA]` (map.rs:504).
- So: `raw_key` starts with `twox128(section.prefix) ++ twox128(entry.name)`. Build a `HashMap<[u8;32], (pallet, item, entry)>` at startup from metadata, then decode the remainder with `hashers` and the `key` type via scale-info. Fallback: unknown 32-byte prefix means "unmapped". Well-known raw keys (`:code`, `:heappages`, `:extrinsic_index`, `:changes_trie`, child-storage markers) are plain and do not match.

### 5.3 Metadata versions and crates in this repo

- Metadata V14/V15/V16 IR: `SDK/substrate/primitives/metadata-ir/src/{v14,v15,v16}.rs`; V16 is "unstable" in the constant list (`lib.rs:43-44`). Output uses `frame_metadata::v16::RuntimeMetadataV16`.
- Workspace deps: `frame-metadata = 23.0.1` (`SDK/Cargo.toml:862`), `scale-info = 2.11.6` (line 1319), `subxt = 0.50` (line 1449), `subxt-metadata` (1450), `subxt-rpcs` (1451). `Cargo.lock` also has `frame-decode` 0.17.2, 0.9.0, 0.7.1, pulled by `subxt`/`subxt-core`/`subxt-metadata`.
- In-tree decoding helpers: none that map keys to Pallet::Item. `state_getKeysPaged`/`state_queryStorage` RPCs are not metadata-aware. `SDK/substrate/utils/frame/storage-access-test-runtime` is a block-validation test runtime, not a decoder. `sp-metadata-ir` only builds metadata.
- Practical recommendation: use `subxt` 0.50 (or `frame-decode`) for metadata decoding and key decoding, rather than reimplementing SCALE-typeinfo walking. Pin the same version the runtime uses for metadata.

---

## 6. Constants and parameter-like storage

### 6.1 `#[pallet::constant]`

- Expansion: `SDK/substrate/frame/support/procedural/src/pallet/expand/constants.rs`.
  - Default value bytes computed at metadata build time: `let value = <T::Const as Get<_>>::get(); value.encode()` (lines ~66-70).
  - Metadata is `pallet_constants_metadata() -> Vec<PalletConstantMetadataIR>` (lines ~120-135). `PalletConstantMetadataIR { name, ty, value: Vec<u8>, docs, deprecation_info }` (`SDK/substrate/primitives/metadata-ir/src/types.rs:506-512`).
  - Runtime access is `T::Const::get()` or `Pallet::<T>::const()`, which is plain code. No storage read, no host call, no event.
- Conclusion: a pallet constant read is invisible to storage tracing, and is only observable by instrumenting the `Get` impl (not generic) or the pallet function that reads it. The constant's value is in metadata, so frame-trace can show "constant X = Y" for a pallet, but not "accessed in extrinsic Z".

### 6.2 `parameter_types!`

- `SDK/substrate/frame/support/src/macros.rs`:
  - Plain `const` (`IMPL_CONST`, line ~139): compiled-in, invisible.
  - `storage NAME: T = default` (`IMPL_STORAGE`, lines ~181-205): key is `twox_128(":" ++ NAME ++ ":")` (lines 185-187). `get()` does `unhashed::get(&key).unwrap_or(default)`, so every access is a raw `Get` event with that key. Doc comment at lines 29-36.
  - `static` (`parameter_types_impl_thread_local`): test-only, std-only.
- Traceability: `storage` parameters are real storage reads, but the key is a raw `:NAME:` form (no pallet prefix), so decoding must map these explicitly. Build the map from the runtime's `parameter_types!` list. The source is not in metadata, so this is a runtime-specific table.

### 6.3 Dynamic parameters pallet

- `SDK/substrate/frame/parameters/src/lib.rs`: the `Parameters` storage map (line ~176-178): `StorageMap<_, Blake2_128Concat, KeyOf<T>, ValueOf<T>, OptionQuery>` with pallet name `Parameters` and item `Parameters`. Doc comment (lines 23-60): "this is incurring one storage read per access".
- Read path: `Get` impl reads `Parameters::<T>::get(key)` (lines ~240-260). So dynamic-parameter accesses appear as `Parameters::Parameters` reads with the parameter key in the Blake2_128Concat part, and are decodable with section 5.
- Dynamic param defaults are compiled-in (no storage read when not set).

### 6.4 What is traceable

| Source | Traceable via storage events? | How |
|---|---|---|
| `#[pallet::constant]` / `Get` (const) | No | Code-only. Metadata has the value. Needs instrumentation. |
| `parameter_types! { const }` | No | Code-only. |
| `parameter_types! { storage }` | Yes | Raw key `twox128(":NAME:")`, map by table. |
| `parameter_types! { static }` | No | Test-only. |
| Dynamic params (`frame-parameters`) | Yes | `Parameters::Parameters` with Blake2_128Concat key. |

---

## 7. OpenTelemetry, Jaeger, tracing-opentelemetry

- Grep of every `Cargo.toml` in the repo for `opentelemetry`, `jaeger`, `tracing-opentelemetry`, `tracing-jaeger`: zero hits.
- `Cargo.lock`: no `opentelemetry*` package.
- Remaining references: `SDK/polkadot/node/gum/src/lib.rs:108-120` ("jaeger dependency" comment, `TraceIdentifier = u128`, `hash_to_trace_identifier`), `SDK/polkadot/node/gum/README.md` (describes `mick-jaeger` stickiness). These are stale: the crate was removed.
- History from prdocs and `gh`:
  - `polkadot-node-jaeger` existed (prdoc `prdoc/1.13.0/pr_1644.prdoc` lists it as a crate that changed).
  - PR #5830 "Remove jaeger from approval-voting and approval-distribution" (merged 2024-09-27).
  - PR #5875 "Remove jaeger everywhere" (merged 2024-10-04). Its prdoc (`prdoc/stable2412/pr_5875.prdoc`): "Jaeger was remove from the codebase because it was not used by anyone and it did not help with the debugging."
  - `prdoc/stable2412/pr_5716.prdoc` and `pr_5830.prdoc` also touch it.
- Implication: no existing OTel exporter or span-to-trace-ID mapping to reuse. We need our own exporter. The `ProfilingLayer` + `TraceHandler` (section 2.2) is the natural attachment point, since it already carries `start_time`, `overall_time`, `parent_id`, and values for both native and wasm spans.

---

## 8. Runtime API and RPC state queries

### 8.1 Path summary

| RPC | Path | Wasm executed? | Spanned? | Per-key storage events? |
|---|---|---|---|---|
| `state_call` | `SDK/substrate/client/rpc/src/state/state_full.rs:198-212` -> `client.executor().call(...)` -> `SDK/substrate/client/service/src/client/call_executor.rs:98-135` `StateMachine::new(...)` -> `execute_aux` | Yes | Only if the called function contains spans (executive entry points do; generic runtime APIs do not). Host `Call`/`Return` events on target `state`. | Yes, `state` events (only visible if `state` target is enabled and a tracer is attached) |
| `state_getStorage` | `state_full.rs:253-260` -> `client.storage(block, key)` -> `Client::storage` in `SDK/substrate/client/service/src/client/client.rs:1461-1471` -> `state_at(hash).storage(key)` (trie/DB backend directly) | No | No | No (no `Ext`, no event) |
| `state_getStorageSize`, `state_getKeys`, `state_queryStorage` | Same family, direct backend | No | No | No |
| Block import / execution | `client.rs:853` -> `runtime_api.execute_block(...)` | Yes | Yes if the executive spans are compiled in | Yes |
| `state_traceBlock` | `state_full.rs:481-498` -> `BlockExecutor` (section 2.3) | Yes | Yes (block-tracing subscriber) | Yes (filtered by `storage_keys`) |
| `state_callRecorded` | `state_full.rs:500-515` -> `ParachainTracingExecuteBlock::call_recorded` | Yes | Not specifically | Yes, plus proof recording |

Key consequence: `state_getStorage` (the most common query) is invisible to all of the event machinery. To trace it, frame-trace needs its own backend wrapper, or it must use the trie `Recorder` (section 3.3) on that call path. The client has no hook here.

### 8.2 Runtime API calls

- Runtime API calls from the client go through `CallApiAt::call_api_at` (`SDK/substrate/client/service/src/client/client.rs:1677`) to `CallExecutor::contextual_call` (`call_executor.rs:136`). Each one is a `StateMachine` run, so it gets a `Call`/`Return` pair and its own `ext_id`.
- The `#[deprecated]` `trace_block` runtime API in revive (section 2.6) is executed through `state_call` and is the only in-tree runtime-side block tracer.

---

## Gaps for frame-trace (things nothing in the SDK provides)

1. **Timing per storage op or per extrinsic in the replay path.** `state_traceBlock` output has no time. Only the live `ProfilingLayer` / `SpanDatum` path has `overall_time`, and that needs a custom node build.
2. **Parent / nesting for wasm spans.** Parent links are explicit-only on both the wasm side and in `BlockSubscriber`. Extrinsic-scoped trees must be rebuilt from `:extrinsic_index` / `ExecutionPhase` markers and ordering.
3. **Cache hit vs DB read per key.** Not exposed. `cache_reads` is never populated. The trie cache metrics are process-global counters.
4. **Iteration (`next_key`) events.** Not traced.
5. **`state_getStorage` and other direct backend reads.** Not traced by any event.
6. **Constants.** Code-only. Need instrumentation or an explicit list.
7. **Per-key proof size.** Only per-recorder totals (`estimate_encoded_size`) and per-key `RecordedForKey` without size. Parachains only.
8. **Decoding.** Metadata has everything, but there is no in-tree decoder. Use `subxt`/`frame-decode`.
9. **Gating of `state_traceBlock`.** Appears ungated in the impl. Verify before exposing it.
10. **Span coverage gaps.** `on_idle` and inherent application have no spans; `on_initialize` / `on_finalize` do, but per-hook sub-steps do not.
11. **Test coverage.** No test asserts on span or event output of block tracing.

## Suggested use (for the design, not decided here)

- Storage order and key identity: `state` events, via `state_traceBlock` for archive replay (works without a custom node, no tracing wasm needed for events).
- Timing and spans: `ProfilingLayer` + custom `TraceHandler` in a custom node (logger hook, as Polkadot runtime-metrics does), with `--tracing-targets` set. Needs `with-tracing` runtimes for wasm spans.
- Decoding: metadata via `subxt` 0.50 / `frame-decode`, prefix table as in section 5.2, plus a manual table for `:NAME:` parameter keys and `:extrinsic_index`.
- Constants: list from metadata; instrument or mark the pallet functions if their access needs tracing.

## Verification still needed on a running node

- Exact string format of `value` / `result` fields in `state` events (Debug-wrapped `Option<HexDisplay>`).
- Whether `parent_id` is actually `None` for nested events in `state_traceBlock` output.
- Whether `state_traceBlock` is reachable on a default public RPC (the `DenyUnsafe` question).
- Whether `ProfilingLayer` timing is accurate for wasm spans (enter/exit happen across the host boundary).
- Which node path your target chains use for `state_callRecorded` (`ParachainTracingExecuteBlock` vs `None`).
