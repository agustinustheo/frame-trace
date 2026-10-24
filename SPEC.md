# frame-trace: spec

frame-trace shows what a Polkadot SDK chain did, step by step, as OpenTelemetry traces: for each block and each extrinsic, which storage items it read and wrote, in what order, how big the values were, and (later) how long each step took.

Research notes behind every decision here are in [`research/`](research/).

## Goals

1. **Works on existing nodes, with safe RPC only.** frame-trace re-executes a block itself, outside the node, with the Polkadot SDK's own executor. It fetches the parent state on demand as authenticated trie proofs (`state_getReadProof`), so it works against any node, including public RPC endpoints, without `--rpc-methods unsafe`. The block's real `Core_execute_block` runs, including the storage-root check, so the replay is provably faithful. (Experiment E2.)
2. **Readable.** Raw keys become `System::Account(5Grw…)`. Extrinsics are named `Balances.transfer_keep_alive`. A trace opens in Jaeger, Tempo or any OTLP viewer.
3. **Complete and real.** Every storage access, including deletions by prefix, child tries and iteration, in order, with measured timings, and nested under runtime spans when the runtime was built with `with-tracing`.
4. **Reuse, don't rebuild.** sc-executor, sp-state-machine, sp-trie and sp-io do the execution. subxt and frame-decode decode metadata and keys. The OpenTelemetry SDK exports OTLP. frame-trace is the glue.
5. **Reproducible.** A replay can be saved as an artifact (block + proof nodes) and replayed offline later, with no network.

## Non-goals (for now)

- Setting weights. Benchmarks stay the source of truth.
- A custom UI. Use Jaeger/Tempo/Perfetto first.
- Changing the Polkadot SDK. Upstream proposals (drafts in `upstream/`) come after we have numbers.

## Why not `state_traceBlock`

E1 showed the node's block-trace RPC is not enough on its own: it needs unsafe RPC, drops every storage event without a key (prefix deletions, child-trie kills, storage roots), never traces iteration, returns no timing and no span nesting, and in omni-node dev mode cannot replay signed parachain blocks at all. frame-trace keeps it as a secondary source (`--source trace-block`) and as the test oracle.

## What we reuse

| Need | Reuse | Notes |
| --- | --- | --- |
| Execute a block | `sc-executor` `WasmExecutor`, `sp-state-machine`, `sp-io` + the same extra host functions as omni-node (storage proof size, statement store, elliptic curves) | Runtime code comes from the parent state, authenticated by proof |
| Parent state | `state_getReadProof` at the parent hash, fetched per missing trie node, cached; `sp-trie` `TrieBackend` | Safe RPC only. The root check proves the state was right |
| Proof size (parachains) | a fresh `sp_trie` recorder + `ProofSizeExt` per execution | Also gives proof size per extrinsic |
| Every storage op, in order, timed | a `tracing` subscriber on target `state`, plus backend instrumentation for `next_key` | Captures what `state_traceBlock` drops |
| Runtime spans | `sp-tracing` `with-tracing` | Optional; parents reconstructed from the entered-span stack |
| Extrinsic boundaries | `System::ExecutionPhase`, `:extrinsic_index`, `System::ExtrinsicData` writes | Rules in `crates/model/src/phase.rs` |
| Key → `Pallet::Item` + map keys | `frame-decode` / `frame-metadata` (V14–V16) | |
| OTLP export | `opentelemetry` / `opentelemetry_sdk` / `opentelemetry-otlp` 0.33 | Used directly, not through `tracing-opentelemetry` |
| Span attribute names | OTel database semantic conventions | `db.system.name = "substrate"`, `db.operation.name`, `db.collection.name` = pallet, plus `frame.storage.*` |
| Viewer | Jaeger v2 all-in-one (4317/4318/16686) | |

Inspiration only: try-runtime-cli (relaxes the root and signature checks; we don't), Chopsticks' lazy storage layer, smoldot's resumable executor, substrate-api-sidecar's `Trace.ts` (GPL, do not copy), nearcore's `io_tracer`, geth's prestate/call tracers, pallet-revive's tracers.

## Trace model

One trace per block:

```
block #1234 (0xabc…)                       span
├─ initialization (on_initialize hooks)    span
│  ├─ get System::Number                   span (storage op)
│  └─ …
├─ extrinsic 1: Balances.transfer_keep_alive   span
│  ├─ get System::Account(Alice)           span
│  ├─ put System::Account(Alice)           span
│  └─ …
└─ finalization (on_finalize hooks)        span
```

Storage op spans carry: operation (`get`/`put`/`clear`/`clear_prefix`/`append`/`next_key`, child-trie flag), pallet, item, decoded map keys (when decodable), raw key, value length, a `duplicate_read` flag when the same key was already read in this extrinsic, and the extrinsic index. Each extrinsic span carries totals: reads, writes, distinct keys, bytes read and written, duplicate reads.

**Timing.** Self-replay measures every storage op and span (`frame.timing = "measured"`). These are replay timings on the tracing machine, with tracing overhead, not the original node's database latency: good for "where does the time go", not for weights. The `trace-block` source has no timing, so it uses synthetic, evenly spaced timestamps (`frame.timing = "synthetic"`).

Raw keys and values can identify accounts. Keys are on by default (needed for debugging); values are off by default, length only.

## Command line

```sh
frame-trace block   --rpc wss://… --block <number|hash> [--otlp http://127.0.0.1:4318] [--json out.json] [--save <artifact-dir>]
frame-trace block   --offline <artifact-dir> [--otlp …]          # replay a saved artifact, no network
frame-trace call    --rpc wss://… [--at finalized] --method AccountNonceApi_account_nonce --data <hex> [--oracle] [--save <artifact-dir>]
frame-trace call    --offline <artifact-dir> [--summary] [--otlp …]
frame-trace follow  --rpc wss://… [--otlp …]                    # trace each new block
frame-trace summary --rpc … --block …                            # terminal tables: top items, duplicate reads, per extrinsic
```

## Phases

1. **Replay exporter (now).** Self-replay source, `block`, `follow`, `summary`, offline artifacts. Tested offline against saved artifacts from a local People chain and public Westend Asset Hub.
2. **Runtime spans.** Nest storage ops under dispatch and hook spans when the runtime has `with-tracing`; build such runtimes on demand for debugging.
3. **State queries.** `call` executes runtime APIs at a selected state, using authenticated proofs, measured storage/runtime spans, metadata argument/result decoding, optional safe `state_call` comparison, and offline artifacts. Calls have no block root assertion. A standalone storage query/trie-depth view is deferred; see README.
4. **Constants.** Plain `#[pallet::constant]` values are compiled in and never touch storage. Tracing them needs an opt-in macro or instrumented runtime. Only `parameter_types! { storage … }` and dynamic parameters are visible as storage today.
5. **PolkaMeter integration.** Attach a block trace to each load-test step, link spans by extrinsic hash.

## Repository layout

```
crates/
  model/     trace model: blocks, extrinsic segments, storage ops, span tree (no I/O)
  decode/    metadata-based key and extrinsic decoding
  replay/    self-replay: safe-RPC proof fetching, executor, capture
  source/    (optional) state_traceBlock client, used as the test oracle
  otel/      model → OpenTelemetry spans and OTLP export
  cli/       the frame-trace binary
fixtures/    recorded state_traceBlock responses + metadata for tests
experiments/ notes and scripts from experiments
research/    background research
```
