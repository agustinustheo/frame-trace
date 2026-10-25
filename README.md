# frame-trace

Replay a Polkadot SDK block and view its storage accesses in Jaeger or Tempo.
Self-replay uses safe RPC and authenticated parent-state proofs. The SDK executes
`Core_execute_block`, including signatures and the real storage-root assertion.

![A Utility.batch extrinsic in Jaeger: batch, three transfer_keep_alive calls and every storage read and write under them](docs/images/jaeger-batch.png)

Build with the pinned Rust toolchain. The first build compiles the SDK executor.

```sh
cargo +1.93.0 build --release --locked
cargo +1.93.0 test --workspace --locked
cargo +1.93.0 clippy --workspace --all-targets --locked -- -D warnings
```

Start Jaeger. Open http://127.0.0.1:16686 and select service `frame-trace`.

```sh
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest
```

Trace the current best Westend Asset Hub block. A decimal number or full hash also
works. Older blocks may need an archive endpoint.

```sh
target/release/frame-trace block \
  --rpc wss://westend-asset-hub-rpc.polkadot.io --block best \
  --save saved-block --otlp http://127.0.0.1:4318 --json block.json

target/release/frame-trace block --offline saved-block --otlp http://127.0.0.1:4318
target/release/frame-trace summary --offline saved-block
target/release/frame-trace summary \
  --rpc wss://westend-asset-hub-rpc.polkadot.io --block best

target/release/frame-trace follow \
  --rpc wss://westend-asset-hub-rpc.polkadot.io --otlp http://127.0.0.1:4318
```

Each replay prints the computed and header state roots, their comparison and RPC count.
Discovery batches up to 32 predicted storage keys per proof and uses eight concurrent
requests by default (`--proof-concurrency 1..64`). Metadata storage prefixes,
the last three successful replays' full keys and trie miss paths, and up to eight discarded speculative executions
provide the predictions. Branch misses also batch immediate sibling paths needed
by trie-root calculation and iteration. When a new 32-byte identifier appears in a
learned concat-hashed map, the same proof request also predicts related maps observed
in recent blocks (at most 31 extra keys). This shortens linked-list/account lookup
chains without assuming the guesses are correct. A strict authenticated discovery run follows; speculative
outputs are never exported or accepted. Progress is printed during proof acquisition
and runtime compilation/loading.
The measured run uses a warm cache and zero RPC. A missing proof is an error.
Offline replay never creates an RPC client. HTTP(S) endpoints also work.

`follow` polls finalized headers every two seconds. Use `--best` to follow reversible
best heads; reorgs and reverted traced blocks are logged. Two workers share authenticated
content-addressed nodes (bounded to 256 MiB), full-key hints and compiled runtimes
keyed by code hash. Before runtime loading, one bounded acquisition stage fetches predicted full paths
(including root-calculation/iteration branches). Each block walks the cache from
its own parent state root and skips fully covered paths (including proven absence).
Fresh overlays and recorders preserve proof sizes. `--cache-dir DIR` also persists
proof nodes and Wasmtime's compiled-module cache. Without it, compiled modules
persist under `$XDG_CACHE_HOME/frame-trace/wasm` (or `~/.cache/frame-trace/wasm`);
proof nodes remain in memory. Offline artifacts always supply their own proof nodes.
Corrupt content-addressed disk nodes are discarded and fetched again online.

`--cache-max-bytes N` bounds the combined proof-node and compiled Wasm disk cache
(default 1 GiB), including the default Wasm directory when `--cache-dir` is omitted.
An in-memory size/age index is initialized once; subsequent maintenance discovers
new Wasmtime entries and indexes proof writes incrementally. Oldest unpinned files
are evicted after each replay. Active compiled runtimes stay pinned (including
between repeated calls); a warning reports when they exceed the configured cap.
Compilation/persistence and pinned modules can exceed the cap. Saved artifacts are separate
and never evicted. Deleting cached files does not remove nodes or runtimes already
held by active workers.

An eight-block queue bounds pending work. Catch-up/reorg walks never precede the
first observed height or the latest eight heights, and schedule newest blocks first.
Queued blocks that fall outside that window are skipped before replay. Queued,
in-flight, done, retryable, terminal, reverted, skipped and cancelled counts are reported.
Expired or reverted queued jobs are skipped; Ctrl-C jobs are cancelled, without failure wording.
A reverted best-chain block becomes eligible for replay if it becomes canonical again. Transient RPC
failures retry at most four block attempts with capped backoff. Pruned state, invalid
blocks, root mismatches and job panics are terminal and logged. Entries are retained
in a bounded 256-block history. **Follow samples at replay speed when it cannot keep
up**; it does not guarantee every block.
RPC calls retry transient timeouts, dropped connections, HTTP 429/5xx and RPC rate
limits, up to four attempts with capped exponential backoff and jitter. DNS/TCP/
TLS/WebSocket setup has a ten-second deadline and the whole call, including pool
wait and retries, has a thirty-second deadline. HTTP and WS use the same WebPKI
root store. Ctrl-C cancels outstanding RPC immediately; a second Ctrl-C forces
exit. Shutdown does not wait for SDK compilation or collector I/O.

Times are measured on the replay machine, including tracing overhead. They are
not the original node's database latency or benchmark weights. Runtime spans
appear when the Wasm has `with-tracing`. Without them, ops sit under phase spans.
Backend iteration events describe attempts; the overlay may change the result.
Named phases retain their statistics and parent runtime dispatch/hook spans and
storage operations in one tree. The runtime's whole-block wrapper is represented
by the block span. Only traced `apply_extrinsic` spans carry
`frame.proof.marginal_bytes`, the recorder's growth during that span. These are
not standalone extrinsic proof sizes.

Names require metadata V14 or newer. Metadata for calls **and block names** comes
from local execution of the authenticated runtime against the selected state:
`Metadata_metadata_at_version(16)`, then 15, then legacy `Metadata_metadata` (V14).
The cache is indexed by authenticated code hash and captured storage dependency values.
Each new state root authenticates those values before reuse; an export with no
storage or root dependencies reuses solely by code hash. Metadata constants in the People runtime
do read storage, so treating metadata as code-only would be incorrect. A changed
value regenerates metadata. Iteration and whole-state/child-root dependencies conservatively retain the state
root. Cache validation reads full authenticated values, including external value nodes.
Root-bound entries also retain metadata execution proof nodes. Saved artifacts retain the current state’s metadata execution proofs, including
after a cache hit, for self-contained offline replay. RPC `state_getMetadata` is never
queried by default. If local metadata execution is impossible, replay uses unnamed
storage/hex results; `--args` requires API types. Explicit `--unauthenticated-metadata`
permits **unauthenticated** RPC metadata (or stored metadata offline) and labels
that fallback prominently. The three People block fixtures include metadata execution proofs and need no fallback.
The old `--rpc-metadata-fallback` spelling remains an alias. Metadata transport cancellation and
exhausted retries propagate as errors.
Caught replay-execution panics return clean errors without the default panic text;
panics outside that execution scope still use the normal panic hook.

Hook spans are named `on_initialize: Balances` (also `on_finalize`, `on_idle` and
`on_poll`). Metadata type paths resolve runtime pallet names. SDK metadata V14–V16
preserves the order used to construct `AllPalletsWithSystem`: legacy
`construct_runtime!` uses declaration order; the newer runtime macro sorts by
index unless legacy ordering is requested. We preserve the metadata list order.
`on_idle` rotates the complete tuple by block number before selecting instances.
Instances are assigned only for complete observed sequences; missing hook traces
or an unknown idle block number retain candidates such as `Assets | PoolAssets`.
Missing metadata mappings use the crate name in CamelCase with `pallet_` removed.
`frame.runtime.target` is retained.
Every storage op has `db.collection.name`: the decoded pallet (including prefixed
iteration), `well_known` for keys such as `:code`, `child_trie` for child operations,
`state` for whole-state roots/unprefixed iteration, or `unknown` for other keys.
Untimed measured phases carry `frame.phase.timing_available = false` and use the
block's enclosing interval; absent timing never widens a timed phase.

Recorded `state_traceBlock` JSON remains available as a secondary source. Its
timestamps are synthetic. Metadata is optional for this source.

```sh
target/release/frame-trace summary \
  --from-file fixtures/model/two-extrinsics.json \
  --metadata fixtures/metadata/westend-v16.scale
```

Replay supports SDK Blake2 headers with u32 block numbers. Unknown host imports,
missing extensions and root mismatches fail. Proof-size-sensitive execution is
accepted only if the real root check passes. Public acquisition can take many
proof round trips. No node-wide state scrape or unsafe RPC is used.

The executor uses the [stable2609 SDK release](https://github.com/paritytech/polkadot-sdk/releases/tag/polkadot-stable2609):
`sc-executor 0.51.0`, `sp-io/sp-runtime 49.0.0`, `sp-trie 47.0.0`,
`sp-state-machine 0.54.0`. Its host families match omni-node: Substrate, proof size,
statement store and RFC163 elliptic curves. Offline tests execute both E1's
stable2606-2 People runtimes and the public Westend runtime with that set.

See [SPEC.md](SPEC.md), [artifact format](crates/replay/ARTIFACT.md) and
[fixture provenance](fixtures/replay/README.md).

Artifact loading limits decompressed JSON to 64 MiB, proof nodes to 100,000 and
total decoded proof bytes to 24 MiB. Fixture blobs are limited to 16 MiB each;
errors identify the affected file. `--save` artifacts remain self-contained.

The workspace's `[profile.dev] debug = 0` omits debug symbols to keep the SDK /
Wasmtime development build within practical disk and linker-memory limits. It
does not change execution optimizations. Use release builds for throughput and
change the profile locally when native debugger symbols are needed.

[Replay fix-round measurements](crates/replay/PERFORMANCE.md) record cold/warm
replay, two-minute follow throughput, offline summary timings and exact test counts.

## Runtime API calls

`call` executes a runtime API against the selected block's **own state** (default
`best`; also `finalized`, a decimal number or a full hash). It authenticates runtime
code and every fetched storage value using the same proof backend as block replay.
**No block state-root check applies to a call.** Writes stay in a disposable overlay.
The optional safe `state_call` oracle compares bytes at that exact hash.

```sh
target/release/frame-trace call \
  --rpc wss://westend-asset-hub-rpc.polkadot.io --at finalized \
  --method AccountNonceApi_account_nonce \
  --data 0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d \
  --oracle --summary --json call.json --save saved-call

target/release/frame-trace call --offline saved-call --otlp http://127.0.0.1:4318

target/release/frame-trace call \
  --rpc wss://westend-asset-hub-rpc.polkadot.io --at finalized \
  --method TransactionPaymentApi_query_weight_to_fee \
  --args '{"weight":{"ref_time":1000000,"proof_size":0}}'
```

Use exactly one of `--data` (hex SCALE arguments; `0x` for none) or `--args`
(a positional JSON array or named object). V15/V16 runtime API metadata supplies
argument and return types; the CLI negotiates it by local authenticated execution.
JSON supports integers, booleans, strings, byte arrays, records and enum values
such as `{"$variant":"Some","$values":[16]}` (an object in `$values` supplies named
fields). Accounts use 32-byte arrays; use `--data` for SS58 inputs or unsupported
JSON shapes. Missing API types produce a hex result and require `--data` for inputs.
This also works for runtime-exposed view APIs with their usual SCALE arguments.

Each call exports one trace rooted at `call <method> at #N`, with ordered storage
operations, runtime spans when available, and measured local timings. `--summary`
prints the storage tables; JSON adds method/input/result, decoded output, proof
bytes, state root and optional oracle comparison to the existing trace model.
Byte vectors render as hex. Terminal result previews are limited to 256 characters;
`--json FILE` keeps the complete result. `--json -` writes JSON to stdout and sends
`--summary` tables to stderr. Exit codes: 0 success, 1 execution/input/export error,
2 command-line usage error, **3 oracle mismatch** (JSON/save/export still complete).
The proof-byte count is the backend recorder's proof, excluding code/metadata loading.
Calls supply the node's runtime-version reader, and omit `ProofSizeExt` as ordinary
`state_call` does; proof reporting does not change the runtime's observed proof size.
Call artifacts have `kind = "call"`, record the output and optional oracle bytes,
and check the recorded output again during offline execution. Offline oracle
comparisons are labelled **recorded (unverified)**: artifact oracle bytes are
editable auxiliary input and have no cryptographic authenticity. They make no network requests.

A separate `storage` command is deferred: these calls already expose authenticated
storage reads and their execution proof sizes. A direct storage query's extra value
would be per-key trie depth/node accounting, which is not yet exposed by the shared
recorder. Raw storage values and a standalone trie-depth view remain future work.


## Read origins, proof accounting, and saved summaries

```sh
target/release/frame-trace block --offline fixtures/replay/people-traced --json batch.json
target/release/frame-trace summary --from-json batch.json --top 10
```

Every measured `get`, `exists`, and `hash` (including child reads) has an `origin`:
`overlay` means the SDK externalities returned without invoking the trie backend;
`backend_warm` means a backend call used only trie nodes already recorded during
this execution; `backend_cold` means it introduced new nodes. Backend markers are
matched to the immediately following synchronous externalities read, within its
observed host span. This observes backend access, rather than inferring it from a
repeated key or zero proof growth. Recorder node sets establish warmth, including
across recorder transaction rollback. It says nothing about physical disk I/O,
OS/database caches, RPC acquisition, or whether removing a read would be safe.
A fresh measured recorder excludes earlier runtime loading and metadata execution.
Legacy `state_traceBlock` JSON lacks this evidence and reports unknown origins.

`proof_bytes_added` is positive growth of the SDK recorder's encoded-size estimate
at ordered storage-operation events. Append backend growth is assigned to the
Append that owns the access, including when its SDK event precedes the read. The first operation to introduce shared trie
nodes gets their bytes; later users get zero. Backend iteration events can introduce
nodes inside an enclosing prefix clear, so those events receive the bytes and the
outer event gets only subsequent growth. No nested double counting is added.
Recorder shrinkage at transaction boundaries resets the growth baseline: a path
reintroduced after rollback can be warm yet add proof bytes again. Therefore sums
in rollback-heavy blocks can exceed the final execution proof. Internal work without
a storage event is charged to the next event, or reported as unattributed if it
happens after the last event. These are marginal estimates, not standalone item or
extrinsic proofs, PoV size, or network bytes. `execution` in block JSON records the
root-check result, final proof bytes, RPC calls, and acquisition rounds separately.

Summary splits duplicate reads by origin and reports real backend calls alongside
overlay hits. It aggregates marginal proof bytes by storage item, phase, and
extrinsic, ranks items by proof bytes, and counts `next_key`/backend iterator
attempts in an **iteration** column. Attempts are neither writes nor proof of a
returned or deleted key. Byte columns sum known value lengths; an overlay read
still contributes logical bytes. Duplicate scope remains the same full key within
a phase, including repeats after writes.

Each phase exports `wall_time` with `start_ns`, `end_ns`, `duration_ns`, and `source`.
`runtime_span` bounds enclose the measured executive/hook spans defining that phase;
`storage_envelope` bounds enclose only observed storage calls when runtime spans
are unavailable. The latter excludes unobserved compute before/after those calls.
Untimed input has null bounds. Parent extrinsic bounds can include nested
`post_inherents`; its operations, proof bytes, and storage time are counted separately. Storage
time is the union of operation intervals, including nested backend intervals once.
These timings include instrumentation on the replay machine, not node latency.

## Reports across blocks

```sh
target/release/frame-trace report blocks/*.json --top 10 --json report.json
target/release/frame-trace report --rpc wss://westend-asset-hub-rpc.polkadot.io \
  --from 18563569 --to 18563608 --cache-dir .trace-cache --json range.json
target/release/frame-trace report --rpc wss://westend-asset-hub-rpc.polkadot.io \
  --last 20 --concurrency 2 --proof-concurrency 4
```

`report` accepts saved `block --json` files (also repeatable `--from-json FILE`)
or a non-genesis, finalized inclusive range of at most 10,000 blocks. `--last K`
anchors once to the finalized head. Live requests pin each block hash and reuse
proof, compiled-runtime, and metadata caches. Two block workers run by default
(configurable 1–4); proof RPC concurrency is a separate shared bound. Historical
ranges may require archive state. Each completed block is folded immediately into
statistics; traces are released. Failed blocks/files, duplicate input hashes and call
traces are listed in output while remaining inputs continue. Any failure exits non-zero. Saved inputs do no replay and require no RPC client.

Short terminal tables and complete JSON show phase shares of operations, unioned
storage time and attributed proof bytes; per-item origins, writes, iteration,
logical bytes, proof bytes and duplicates; observed hook/pallet totals; and
extrinsic-type count, median and nearest-rank p95 operations, marginal proof bytes,
and storage time. `--top` limits ranked terminal rows and reports omitted counts;
extrinsics rank by total proof bytes (`--sort-by ops|time` selects another cost).
Distributions are exact through 1,024 samples; thereafter a bounded logarithmic
sketch has at most 1.5625% value error (not a rank-error bound). JSON additionally includes observed extrinsic wall-time
statistics when runtime span bounds exist. Inherents and signed extrinsics have
separate phase groups when the extrinsic bytes identify them. Missing origins are counted as unknown; `proof_ops_observed` distinguishes
missing proof evidence from a measured zero. Runtime wall-time distributions are
null without defining runtime spans. Old saved output is never assigned an inferred
origin or proof measurement. Avoid combining different chains with
the same pallet/call names unless that grouping is intentional.

The [JSON format](docs/json-schema.md) documents schema versions, legacy missing
evidence, report errors, quantile bounds and ranking.
