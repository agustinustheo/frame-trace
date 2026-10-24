# frame-trace: spec

frame-trace shows what a Polkadot SDK chain did, step by step, as OpenTelemetry traces: for each block and each extrinsic, which storage items it read and wrote, in what order, how big the values were, and (later) how long each step took.

Research notes behind every decision here are in [`research/`](research/).

## Goals

1. **Works on existing nodes.** The first version needs no node or runtime changes, only RPC access to a node started with `--rpc-methods unsafe` that still has the block's parent state (an archive node, or a recent block).
2. **Readable.** Raw keys become `System::Account(5Grw…)`. Extrinsics are named `Balances.transfer_keep_alive`. A trace opens in Jaeger, Tempo or any OTLP viewer.
3. **Reuse, don't rebuild.** Polkadot SDK already replays blocks and logs every storage access. subxt and frame-decode already decode metadata and keys. The OpenTelemetry SDK already exports OTLP. frame-trace is the glue.
4. **Cheap when off, efficient when on**, so it can be pointed at a production node to debug a real block.

## Non-goals (for now)

- Setting weights. Benchmarks stay the source of truth.
- A custom UI. Use Jaeger/Tempo/Perfetto first.
- Changing the Polkadot SDK. Upstream proposals come after we have numbers.

## What we reuse

| Need | Reuse | Notes |
| --- | --- | --- |
| Every storage op of a block, in order | `state_traceBlock` RPC (`sc-tracing`), target `state` | Host-side events: `Get`, `Put`, `ClearPrefix`, `Append`, child-trie variants. No runtime change needed. Gated as unsafe. |
| Extrinsic boundaries | `:extrinsic_index` and `System::ExecutionPhase` writes in the same event stream | |
| Runtime spans (dispatch, hooks) | `sp-tracing` `with-tracing` runtime feature | Optional. Only if the runtime was built with it. |
| Key → `Pallet::Item` + map keys | `frame-decode` / subxt metadata (V14–V16) | `twox128(pallet) ++ twox128(item)` prefix, then hashers. |
| Extrinsic names | block body + metadata via subxt | |
| OTLP export | `opentelemetry` / `opentelemetry_sdk` / `opentelemetry-otlp` 0.33 | Used directly, not through `tracing-opentelemetry`, so we never touch the node's pinned `tracing-subscriber`. |
| Span attribute names | OTel database semantic conventions | `db.system.name = "substrate"`, `db.operation.name`, `db.collection.name` = pallet, plus `frame.storage.*`. |
| Viewer | Jaeger v2 all-in-one (4317/4318/16686) | |

Inspiration only: substrate-api-sidecar's `Trace.ts` (GPL, do not copy), nearcore's `io_tracer`, geth's prestate/call tracers, pallet-revive's tracers.

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

**Timing.** `state_traceBlock` returns order but no timestamps. Replay mode lays ops out in order with synthetic, evenly spaced timestamps and marks the trace `frame.timing = "synthetic"`, so nobody mistakes the widths for real durations. Real timing comes from live mode (phase 3).

Raw keys and values can identify accounts. Keys are on by default (needed for debugging); values are off by default, length only.

## Command line

```sh
frame-trace block  --rpc ws://127.0.0.1:9944 --block <number|hash> [--otlp http://127.0.0.1:4318] [--json out.json]
frame-trace follow --rpc ws://127.0.0.1:9944 [--otlp …]      # trace each new block
frame-trace summary --rpc … --block …                          # terminal table: top items by reads/writes, duplicate reads
```

## Phases

1. **Replay exporter (now).** `block`, `follow`, `summary` on top of `state_traceBlock`. Tested against recorded fixtures and a local individuality-community chain.
2. **Runtime spans.** When the runtime has `with-tracing`, nest storage ops under the real dispatch and hook spans.
3. **Live mode with real timing.** A `sc-tracing` `TraceHandler` / layer a node can add, exporting OTLP with real durations, plus cache-hit vs database information where the node exposes it. Also covers runtime API calls and RPC state queries, which block replay cannot see.
4. **Constants.** Plain `#[pallet::constant]` values are compiled in and never touch storage. Tracing them needs an opt-in macro or a runtime build with instrumentation. Only `parameter_types! { storage … }` and dynamic parameters are visible as storage today.
5. **PolkaMeter integration.** Attach a block trace to each load-test step, link spans by extrinsic hash.

## Repository layout

```
crates/
  model/     trace model: blocks, extrinsic segments, storage ops, span tree (no I/O)
  decode/    metadata-based key and extrinsic decoding
  source/    RPC client: state_traceBlock, block body, metadata
  otel/      model → OpenTelemetry spans and OTLP export
  cli/       the frame-trace binary
fixtures/    recorded state_traceBlock responses + metadata for tests
experiments/ notes and scripts from experiments
research/    background research
```
