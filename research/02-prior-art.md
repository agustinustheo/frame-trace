# Prior art: per-extrinsic storage and call tracing for Substrate (frame-trace)

Research date: 2026-10-25. Repo metadata is from the GitHub API on that date ("pushed" = last push to default branch). Stars are approximate.

## Method and limits

- GitHub via `gh` (repo view, code/issue search, contents API). The search API hit its rate limit once, so some code searches are missing or empty and should be re-run before relying on a negative.
- WebSearch/WebFetch for docs, blog posts and proposals.
- Not covered: forum.polkadot.network was not searched directly (the one web search returned nothing from it). Subwasm and subweight could not be located on GitHub under the names tried. try-runtime and polkadot-introspector were checked for metadata only, not for their feature sets.
- Items marked "unverified" are from memory or a single secondary source.

## Summary

- Substrate already exposes the raw material. The `state_traceBlock` RPC re-executes one block and returns tracing spans and events from the client and the WASM runtime. Storage Get/Put events carry the hex key. Empty `storage_keys` means no key filtering.
- The only existing consumer that turns that into per-extrinsic, decoded storage access is `substrate-api-sidecar` (GPL-3.0). It returns JSON, not OpenTelemetry, and by default filters down to two keys (`:extrinsic_index` and `System::Account`) to build balance operations.
- Chopsticks (Apache-2.0) has the best replay and storage-diff tooling, plus an EVM call/VM tracer that works only for Acala/Karura. It is the strongest candidate for an offline replay harness.
- Nobody has, as far as I found, built OpenTelemetry spans for storage reads and writes per extrinsic on Substrate. The old Polkadot Jaeger integration was removed from polkadot-sdk.

## A. Polkadot / Substrate ecosystem

### A1. `state_traceBlock` (polkadot-sdk core)

- **What:** Node RPC that re-executes a block and returns spans and events. Targets are prefix-matched, and storage events can be filtered by hex key prefix.
- **Mechanism:** `substrate/client/rpc/src/state/mod.rs` (RPC), `substrate/client/tracing/src/block/mod.rs` (`BlockExecutor` with `BlockSubscriber`, a custom `tracing` subscriber), `substrate/primitives/rpc/src/tracing.rs` (`BlockTrace`, `Span`, `Event`, `TraceBlockResponse`). Default targets are `pallet,frame,state`. Span names include `execute_block` and `apply_extrinsic` under target `frame_executive`. Each event has `target`, `data` and `parent_id`, so parent/child structure is available.
- **Key detail:** In `block/mod.rs`, `storage_keys` is a hex prefix filter, and an empty string means nothing is filtered out. So `targets="state"` with no key filter should yield every storage Get/Put in the block, which is the raw signal frame-trace needs. (Only the sidecar code path I read uses a filter.)
- **Requirements (from docs and the RPC doc comment):** an archive node that is not doing a major sync, and tracing-enabled WASM runtimes (built with a `with-tracing` feature and loaded via `--wasm-runtime-overrides`) for the runtime versions to be traced. The sidecar docs also say the node needs `--unsafe-rpc-external`. Whether host-side `state` storage events appear without a tracing WASM build is **unverified**, and it is the first thing to test.
- **Status:** Maintained in polkadot-sdk (pushed 2026-10-24). Open issues: polkadot-sdk #7270 (Westend coretime, `Cannot resolve a block range` for some blocks, 2025-01). subquery/subql #1890 (asks for `state_traceBlock` support, open since 2023).
- **License:** Apache-2.0 (polkadot-sdk).
- **Reuse:** Depend on it. It is the foundation for any option below.

### A2. substrate-api-sidecar experimental trace endpoints

- **What:** REST service. Experimental routes: `/experimental/blocks/{blockId}/traces`, `/experimental/blocks/head/traces`, and `.../traces/operations` (with `?actions=` to include action traces). The RC variants `/experimental/rc/blocks/...` are also in the OpenAPI spec.
- **Mechanism:** `src/services/blocks/BlocksTraceService.ts` calls `api.rpc.state.traceBlock(hash, DEFAULT_TARGETS, DEFAULT_KEYS, null)` with `DEFAULT_TARGETS = 'pallet,frame,state'` and `DEFAULT_KEYS` set to the `:extrinsic_index` key and the `System::Account` prefix (`26aa394e…` + `b99d880e…`). `src/services/blocks/trace/Trace.ts` builds a key-to-name map from `api.query` metadata (so storage keys decode to `Pallet::Item`), walks the `frame_executive` `apply_extrinsic` spans, and emits operations (balance changes). Output is JSON, not OTel.
- **Limits:** The filter means it only reports events for those two keys. It is built for balance operations, not general storage access. Output is not per-extrinsic storage access in general.
- **Status:** Repo pushed 2026-10-21; latest release v20.14.1 (2026-04-28). The trace service was last touched 2025-06-10 (AHM staking work). 268 stars.
- **License:** GPL-3.0.
- **Reuse:** Fork-ready as a reference for decoding (key to Pallet::Item, `apply_extrinsic` walking, `Get`/`Put` event parsing via `stringValues.method` and `key`). Forking would make frame-trace GPL-3.0. Prefer reading it and reimplementing the decode logic in Rust or TS against subxt/PAPI metadata.

### A3. Chopsticks (AcalaNetwork/chopsticks)

- **What:** Forks live chains into a local sandbox. Commands include the default fork, `run-block`, `dry-run`, `xcm`, and a `trace-transaction` plugin.
- **Mechanism:**
  - `run-block`: replays a block (default latest) and reports changed storage. Flags: `--block`, `--output-path`, `--html` (storage-diff HTML, generated by `packages/chopsticks/src/utils/generate-html-diff.ts`), `--open`. `dry-run` does the same for an extrinsic (`packages/chopsticks/src/plugins/dry-run/`).
  - `trace-transaction` (`packages/chopsticks/src/plugins/trace-transaction/`): Acala/Karura only. It fetches the tx from the live chain, looks up a tracing-enabled runtime from GitHub Releases (`<spec>_runtime_tracing_<ver>.compact.compressed.wasm`), and calls the runtime API `EVMTraceApi_trace_extrinsic` with a `CallTracer` or `OpcodeTracer` config. Output is JSON. `--vm` gives opcode steps, paginated.
  - The executor is a Rust crate (`executor/`) compiled to WASM via `wasm_bindgen`. It uses smoldot's runtime/trie code, not a full node. The public docs call it a "Smoldot light client". The code is more accurately a smoldot-based WASM runtime executor.
- **Status:** Repo pushed 2026-10-22; latest release 1.5.0 (2026-06-26). trace-transaction plugin: last functional change 2025-07-11. 171 stars.
- **License:** Apache-2.0.
- **Gaps for frame-trace:** No span or event output. No storage-level trace. The EVM tracer depends on an Acala-specific pallet API. The storage diff is between blocks, not per extrinsic.
- **Reuse:** Fork or depend on it for an offline replay harness: fork a live chain, replay a block or extrinsic with `dry-run`, and get the executor and HTML diff. The trace-transaction plugin is the best reference for "fetch tx, find block, run runtime API with a tracer config" and for the tracing-runtime-from-release pattern.

### A4. try-runtime-cli and frame-try-runtime

- **What:** Testing framework for runtime upgrades and block execution against live state (`try-runtime`). Unverified: its subcommands include `execute-block`, which runs a block against remote state.
- **Mechanism:** Runs a runtime against a state snapshot or live RPC state. The `frame-try-runtime` support crate lives in polkadot-sdk. Not a tracer.
- **Status:** try-runtime-cli pushed 2026-08-10; latest v0.10.1 (2026-01-23). 25 stars.
- **License:** GPL-3.0 (try-runtime-cli). frame-try-runtime is Apache-2.0 (in polkadot-sdk).
- **Reuse:** Inspiration for "run this block against live state" and for the remote-state/snapshot plumbing. Not a tracer. Do not fork for tracing.

### A5. polkadot-introspector

- **What:** Collection of CLI tools (`polkadot-introspector`, `subxt`-based) for monitoring relay chain and parachain progress.
- **Status:** Pushed 2026-10-23; 48 stars.
- **License:** GPL-3.0.
- **Mechanism / relevance:** Monitoring and progress tracking, not storage or call tracing. Unverified whether it has any OTel export.
- **Reuse:** Inspiration only for the operator-facing UX.

### A6. subwasm, frame-omni-bencher, subweight

- **subwasm** (chevdor/subwasm): MIT. Inspects and diffs runtime WASM metadata. Last pushed 2025-03-13. Not tracing. Possibly useful to map runtime version to metadata/tracing-build pairs.
- **frame-omni-bencher**: lives in polkadot-sdk at `substrate/utils/frame/omni-bencher`. Runs FRAME benchmarks and produces weight files that include read/write counts. Relevant only as a source of expected per-extrinsic read/write weights to compare against observed traces. Apache-2.0.
- **subweight**: not found on GitHub under `paritytech/subweight`, `open-web3-stack/subweight`, or by search (search returned unrelated repos). Treat as not located.

### A7. Subscan, OnFinality Trace API, Subsquid, substrate-etl

- **OnFinality Trace API** (blog dated 2023-05-17; docs at documentation.onfinality.io/support/trace-api): hosted wrapper around `state_traceBlock` across about 40 Substrate chains. Access by request. Colorful Notion (Polkaholic) reported running about a dozen nodes off it. Closed source.
- **Subscan Trace** (polkadot.subsquare.io/referenda/1906, 2026 proposal): shows storage changes and custom state queries from block or extrinsic pages. Usage fell from about 12.8k requests/day (Sep 2025) to 3.2k/day (May 2026). Hosted, closed.
- **subscan-essentials** (itering/subscan-essentials): open-source explorer backend that indexes blocks, extrinsics and events. No storage-trace feature confirmed. License unverified.
- **Subsquid**: indexing SDK with a call-based Substrate processor. Not a tracer. Unverified whether it exposes storage access.
- **substrate-etl** (Colorful Notion): linked from the OnFinality post, uses trace data for indexing. Not checked in depth.
- **Reuse:** Hosted services are not forkable for frame-trace. They are useful as a demand signal (Subscan and OnFinality both productised `state_traceBlock`) and as a comparison for UX.

### A8. polkadot-node-jaeger (old Jaeger tracing)

- **What:** Polkadot node-side Jaeger primitives (`polkadot_node_jaeger`). docs.rs v11.0.0 says it works with Grafana/Tempo and gives a local Jaeger all-in-one setup.
- **Status:** Removed. Commit "Remove jaeger everywhere (#5875)" in polkadot-sdk, which the commit message says went mostly unused and caused CPU and memory costs. The `polkadot/node` directory on master no longer has a jaeger crate (checked 2026-10-25).
- **Reuse:** Not reusable. It shows the project already tried OTel-style tracing in the node and dropped it for cost. Any proposal should target a light, opt-in, off-by-default path and say how it avoids that cost.

### A9. pallet-revive / Polkadot Hub EVM tracing

- **What:** `debug_traceTransaction`, `debug_traceBlockByNumber`, `debug_traceCall` in the ETH-RPC proxy (`substrate/frame/revive/rpc/src/apis/debug_apis.rs`). Tracer types: `CallTracer`, `PrestateTracer`, `ExecutionTracer` (opcode-level), with configs in `substrate/frame/revive/types/src/runtime_api/types/tracer.rs`.
- **Mechanism:** The on-chain side has a composite tracer in `substrate/frame/revive/src/evm/tracing.rs` (call tracer, prestate tracer, opcode/syscall tracer) and a call tracer in `src/evm/tracing/call_tracing.rs`. The RPC proxy wraps each runtime-API tracing call with a timeout (`with_timeout`). Replay runs inside the runtime, not in the node's RPC layer. The layout suggests this, but the exact runtime-API dispatch was not read line by line.
- **Recent work:** prdoc 11581 (state overrides in `debug_traceCall`, stable2606), prdoc 12386 (omit empty traces uniformly, stable2609).
- **Status:** Active (polkadot-sdk pushed 2026-10-24). `paritytech/revive` (Solidity to PolkaVM compiler) is separate, pushed 2026-10-23, Apache-2.0.
- **License:** Apache-2.0.
- **Reuse:** This is the closest in-tree design to what frame-trace needs for the EVM side: tracer configs passed to a runtime API, results returned as call trees, prestate diffs, or opcode steps. Two lessons: (1) the tracer runs in the runtime, so it works against existing state; (2) "empty trace" semantics needed normalising (#12386). For Substrate storage, the analogue is a `TracerConfig` for "storage access" passed to `apply_extrinsic`-level re-execution.
- **Issues:** polkadot-sdk #9574 (EVM warm/cold storage distinction, open 2026-04). #12236 (excessive proof size for a contract call, open 2026-07). Both show interest in per-call storage accounting.

### A10. Parity issues and other mentions

- Searches for "opentelemetry" in polkadot-sdk issues returned only #803 (staking-miner `/metrics`, closed). Searches for "runtime profiling storage" and "storage access" returned storage-weight and benchmarking issues, not tracing proposals. polkadot-sdk #804 "Fine grained CPU profiling" (closed 2024-05-31) is the closest proposal found.
- polkadot-sdk #5223 "FRAME: why not refund the weight used for storage access accurately with a new DataBaseAccessExt" (open 2024-08). Relevant to any storage-access accounting.
- No GitHub repos found with "substrate opentelemetry", "polkadot tracing", "substrate jaeger" or "storage access profiler" in their name or description. (Search rate-limited on some queries; re-run before final.)

## B. Other chains (UX and design inspiration)

### B1. Ethereum clients

- **geth** (ethereum/go-ethereum, LGPL-3.0, pushed 2026-10-23). `eth/tracers/native/` has `call.go` (callTracer), `call_flat.go` (flatCallTracer), `prestate.go` (prestateTracer, supports `diffMode: true` with `pre`/`post`), `4byte.go`, `mux.go` (multiplexer), `noop.go`, `erc7562.go`, `opcode_counter.go`. JS tracers under `eth/tracers/js/`. The struct/opcode logger is the default. Design lesson: a "what state was touched" tracer (prestate with diffMode) is more useful for storage-level questions than opcode logs.
- **reth** (paradigmxyz/reth, Apache-2.0, pushed 2026-10-24). `crates/rpc/rpc/src/debug.rs` (`debug_trace_*`), `crates/rpc/rpc/src/trace.rs` (Erigon-style `trace_*`), and `crates/tracing-otlp/` (OTLP export of node tracing spans, not per-transaction storage). Reference for "node-level OTLP plus per-tx debug RPC" as two separate concerns.
- **Foundry `cast run`** (foundry-rs/foundry, Apache-2.0, pushed 2026-10-24). Replays an on-chain tx locally with `-t/--trace-printer` (opcode trace), and `--debug-trace-transaction` fetches a node's `debug_traceTransaction` (callTracer) and renders it. Good CLI UX model: `cast run <hash>` gives a call tree plus a verbosity flag.
- **Tenderly** (proprietary, docs at docs.tenderly.co). Debugger steps through a tx; state changes decoded by default with old/new values. Exposed via `tenderly_trace`, `tenderly_simulate`, `tenderly_simulateBundle`. Closed. Best-in-class UX reference for "state diff per tx with decoded names".
- **OpenTelemetry for EVM**: no client found emitting per-SLOAD OTel spans. Closest is the EIP-7928 (Block-Level Access Lists) OTel tracing draft from ethpandaops (notes.ethereum.org/@ethpandaops/bal-otel). It tracks storage slot reads and writes per block and proposes OTel metrics. A draft, not shipped.

### B2. NEAR

- **nearcore** (near/nearcore, GPL-3.0, pushed 2026-10-24).
  - `core/o11y/src/opentelemetry.rs`: OTLP span export to a collector.
  - `core/o11y/src/io_tracer.rs` (feature `io_trace`, last changed 2025-02-21): a `tracing` Layer that emits storage ops (`StorageOp`: Read, Write, Remove, Exists, Other) and DB ops (`DbOp`: Get, Insert, Set, ...) as events in a span hierarchy, written to a file. Its stated goal is to show "what requests are sent to the DB and for what reason", including cache hits and misses. It is also replayable by the estimator.
- **Relevance:** This is the closest existing design to frame-trace's storage-access layer: a feature-gated `tracing` layer that turns storage ops into events under the extrinsic/receipt span. The difference is that it writes its own format, not OTel. Reuse as inspiration only (GPL-3.0).

### B3. Cosmos SDK

- **cosmos-sdk** (Apache-2.0, pushed 2026-10-24). `telemetry/README.md` documents OpenTelemetry metrics and traces exported over OTLP. The quick start runs Grafana LGTM (`grafana/otel-lgtm`) on ports 4317/4318. Tracing is manual: modules call `otel.Tracer()` and must thread `context.Context` for spans. Storage reads are not traced per key by default (not verified).
- **Relevance:** The closest "OTel out of the box for a blockchain SDK" example, and a model for the local dev setup (LGTM docker). Its instrumentation is manual, so it does not solve the per-key problem.

### B4. Solana and CometBFT

- **agave** (anza-xyz/agave, Apache-2.0, pushed 2026-10-24): code search for "opentelemetry" returned no hits. Metrics go through the `solana-metrics` crate. No per-account storage trace found.
- **cometbft** (Apache-2.0, pushed 2026-10-24): code search for "opentelemetry" returned no hits.

### B5. Other

- **rippled / XRP Ledger** (archived plan at git.inftf.org, OpenTelemetryPlan): proposes transaction-level spans (`tx.relay`, `txq.enqueue`) over OTLP/gRPC, with telemetry limited to metadata and account hashing for redaction. Useful for the privacy and cardinality rules a Substrate tool would also need.

## C. Conclusions

### Has anyone built this?

No, as far as I could find. Specifically missing:

- An OTel exporter for `state_traceBlock` output. Substrate's `sc-tracing` has no OTLP path in the code I read.
- Per-extrinsic storage access spans (`Pallet::Item` with read/write, value size, order) exported to Jaeger/Tempo. The only consumer (sidecar) returns JSON and filters to balance keys.
- Constant-access tracing. Constants are metadata, read through `Get<T>` impls compiled into the runtime, so nothing in the existing traces shows them. This needs runtime-side instrumentation (for example a `#[pallet::constant]` wrapper or a macro in the runtime) and is unexplored.
- A path that works on existing live nodes without a tracing WASM build. The `with-tracing` requirement is the main blocker, and whether host-side storage events need it is unverified.
- The jaeger path was removed in polkadot-sdk, so any new approach must be opt-in and cheap.

Closest things:

1. **sidecar's `Trace.ts`** for decoding storage keys to `Pallet::Item` and for `apply_extrinsic` walking. Read only (GPL-3.0).
2. **`state_traceBlock`** with `targets=state` and empty key filter. Raw capture, already in every archive node.
3. **nearcore `io_tracer.rs`** for the design of a storage-op tracing layer. Inspiration only (GPL-3.0).
4. **pallet-revive tracer configs** for the pattern "tracer config passed to a runtime API, returns structured trace". Inspiration.
5. **Cosmos SDK telemetry** for the OTel and local LGTM setup. Inspiration.

### Candidates for an experiment

1. **Thin exporter over `state_traceBlock` (depend on the RPC, reimplement the decode, output OTLP).** Run against an archive Westend or Paseo node with a tracing runtime. Check first: (a) whether `targets=state` with empty keys returns Get/Put events without a tracing WASM build; (b) event volume and latency for a busy block; (c) that `apply_extrinsic` spans map cleanly to extrinsic indexes. Output: one trace per block, span per extrinsic, child spans per storage op, with decoded names from metadata. Reuse: read sidecar's `Trace.ts` for logic; do not fork it (GPL-3.0 licence would propagate).
2. **Chopsticks fork as an offline replay harness.** Apache-2.0, so forkable. Use `run-block` and `dry-run` on a forked chain to produce the same per-extrinsic storage trace locally, without an archive node, and use it as a development and test oracle for candidate 1. Fork or depend on it, depending on how much of the executor needs to change.
3. **Runtime-side tracer behind a flag (pallet-revive pattern).** Riskier and longer. It gives constant-access and warm/cold information that the RPC approach cannot. It needs runtime upgrades, so it is an upstream conversation, not an experiment to run in a sidecar.

Inspiration only: nearcore `io_tracer` (design), Tenderly (UX), geth prestate `diffMode` (output shape), Foundry `cast run` (CLI shape), Subscan Trace (demand signal).

### Open questions for the next step

- Does `state_traceBlock` emit host-side storage events on a standard (non-tracing) runtime? If not, candidate 1 requires operator-built runtimes, which rules out "existing production nodes" in most cases.
- Does `sc-tracing` have an OTLP path? I read only the `TracingReceiver` enum and the `LogTraceHandler` dispatch in `substrate/client/tracing/src/lib.rs`, and found no OTLP handler there.
- Re-run the GitHub code searches for "opentelemetry", "tracing_opentelemetry" and "otlp" in polkadot-sdk and sidecar once the rate limit resets, and check forum.polkadot.network directly.
