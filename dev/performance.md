# Replay fix-round measurements

Measured on 2026-10-25, macOS arm64, with Rust 1.93.0 development builds
(`debug = 0`, default development optimization level). Endpoint:
`wss://westend-asset-hub-rpc.polkadot.io`. Runs were sequential; compilation/tests
were stopped before the final measurements. The cold comparison uses exactly the
same block and parent state. Follow runs cover different recent intervals, so
throughput remains sensitive to endpoint conditions and block contents.

The baseline is 7fc9ef2. Its follow executable adds only an elapsed-time log around
replay/export, so warm replay time can be distinguished from polling/queue wait.
The cold baseline uses the unmodified executable. Both cold block commands include
`--save`; the optimized command also uses a new empty `--cache-dir`.

- Cold baseline: block #18560214, hash
  `0x2f17b591347f2853c3ddd787771ea73a97f8c11237553109e0c1be21695c5144`,
  2 extrinsics: **51.606 seconds, 133 RPC calls**. The parent state was
  still available for this same-block comparison; Core passed with the identical
  92,603-byte measured execution proof in both versions.
- Cold optimized: block #18560214, hash
  `0x2f17b591347f2853c3ddd787771ea73a97f8c11237553109e0c1be21695c5144`,
  2 extrinsics: **31.605 seconds, 51 RPC calls**.
- Follow, next completed warm replay after the two startup workers:
  baseline #18559973 **39.862 seconds**; optimized #18560231 **13.102 seconds**.
  Baseline's other first warm worker took 43.390 seconds. Across all
  11 optimized warm completions, the median was **15.114 seconds**
  (range 12.764–16.784 seconds).
- Follow, exactly 120 seconds per version: **4 → 13 completed blocks**,
  **2.0 → 6.5 blocks/minute**. Work finishing after the
  window is excluded. The optimized run used two workers, eight pooled RPC slots,
  32-key batches, shared nodes/runtimes, and a fresh persistent cache directory.
- Ctrl-C shutdown: baseline had not exited after three seconds and was killed by
  the benchmark harness. Optimized shutdown took **0.042 seconds** and exited successfully.
- `summary --offline` on the same self-contained baseline public artifact:
  **13.135 seconds → 1.989 seconds**;
  optimized repeat **1.944 seconds**. The optimized compiled-module
  cache was warm from the cold-block command. All summaries used zero RPC.

Every completed replay passed Core's real root assertion. Optimized output also
reported matching computed/header roots and zero measured RPC calls. Follow
still cannot keep up with the endpoint's observed block rate; queue saturation
is reported and the README documents sampling at replay speed. No live node was
started. All replay clients and mock servers were stopped.

## Verification

`cargo +1.93.0 test --workspace --locked --no-fail-fast`: **79 passed, 0 failed,
1 ignored** (the opt-in public-network test); **0 doctests**. Breakdown: CLI 13,
decode 12, model 19, OTel 6, replay 29 (19 unit + 10 integration), plus the ignored
replay integration case. This includes the subprocess panic-handler regression;
the child re-executes that case and is not an additional workspace test case.
The full local ordered-oracle/phase comparisons and public reference comparison
remain enabled. `cargo +1.93.0 fmt --all --check` and
`cargo +1.93.0 clippy --workspace --all-targets --locked -- -D warnings` passed.

The ten compressed replay fixture files total **11,195,050 bytes**, down from
**15,882,348 bytes**. Two shared content-addressed blobs contain the identical
People Wasm node and metadata. Saved user artifacts remain self-contained.


## Round 2 (working tree based on f618fbe)

Measured on 2026-10-25 on the same macOS arm64 host, with Rust 1.93.0 development
builds. The before executable was preserved from the clean f618fbe checkout (its
replay implementation is 63c98a5); after uses these uncommitted fixes. Endpoint:
`wss://westend-asset-hub-rpc.polkadot.io`. All runs are sequential, after compilation,
tests, fmt and clippy completed. Every command uses a separate initially empty
`--cache-dir`, eight RPC slots and the default 32-key batches. Cold commands write
`--json` and use the identical block hash; they do not save an artifact.

- Cold public block **#18561326**, hash
  `0x67ce1c26456c191e9fc37c722f655429e57754393b344d0952cb31281f2607d8`:
  **51 → 51 RPC calls**, **31.087 → 29.582 seconds**. Both executions have 575
  operations, the same **88,203-byte execution proof**, and computed/header root
  `0xb25cbc55096c87bee7f3fba4dce76be5043b0368e9a3cb80104cbf9a50423219`.
  This cold block shows no RPC-count reduction; the odd-nibble correction is also
  checked directly by the trie sibling-path regression.

- First completed warm follow replay after the two startup jobs: before
  **#18561358**, hash
  `0xcc7e953def65919fc22ea1e21960a4572fde2e409111cccfed0c4b77c814b311`,
  **43 RPC calls, 10.398 seconds**; after **#18561408**, hash
  `0xd8cb4c5bd952c598592581c863c86b9ae80a0ee43961f70e5108172692aa13de`,
  **44 RPC calls, 12.053 seconds**. These are different public blocks/intervals;
  the first warm example does not show a reduction.
- Across all **11 warm completions per window**, median RPC calls were
  **61 → 42** (ranges **43–71 → 36–47**). Median replay time was
  **14.785 → 15.209 seconds** (ranges **10.398–17.509 → 12.053–18.533**).
  Cache coverage reduces the repeated prediction requests in this aggregate;
  fewer RPCs did not translate into lower wall-clock replay time in these windows.
- Follow, **120 seconds per executable**: **13 → 13 completed blocks**,
  **6.5 → 6.5 blocks/minute**. The baseline follows best heads; the fixed run uses
  the new finalized-head default. Both use two workers, fresh persistent caches
  and the same endpoint. Different intervals, head policies and block contents
  limit direct throughput/latency comparisons. The fixed run skips expired queued
  samples near the tip instead of tracing the old backlog; these skips are logged
  as terminal entries and are not counted as completed replays.
- Both follow commands exited successfully on Ctrl-C: **0.144 → 0.143 seconds**.
  The fixed follow log contains no default panic text, no root/proof failures and
  no ancestry-limit warnings. All **26 completed follow replays** and both cold
  replays passed the real root check and reported **zero measured RPC calls**.

Raw cold/follow logs, decoded cold JSON, completion timestamps and per-block
RPC/proof/time rows are retained in `/tmp/frame-trace-round2-bench/` on this host;
`results.json` records the exact commands, exit status and 120-second windows.
All benchmark clients, test subprocesses and mock servers have stopped. No node
or collector was started, and no commit or push was made.

### Round 2 verification

`cargo +1.93.0 test --workspace --locked --no-fail-fast`: **96 passed, 0 failed,
1 ignored**, **0 doctests**. Breakdown: CLI **18** (5 unit + 13 integration),
decode **12** (1 unit + 11 integration), model **19** (11 unit + 8 integration),
OTel **9**, replay **38** (27 unit + 11 integration), plus the opt-in ignored
public-network integration test. Subprocess re-executions are part of their parent
test cases and are not counted again. `cargo +1.93.0 fmt --all --check`,
`cargo +1.93.0 clippy --workspace --all-targets --locked -- -D warnings`, and
`cargo +1.93.0 build --locked` all pass.

Regressions cover reorg/start-height/catch-up bounds and head retreat, terminal
errors and the four-attempt cap, whole-job panic recovery, quiet scoped execution
panics with genuine/other-thread hooks preserved, metadata error propagation,
authenticated cache coverage and absence, changed runtime code, odd-nibble sibling
paths, child pending namespaces, corrupt cache healing, combined disk eviction
(including failed replays/abandoned writes), concurrent artifact saves, empty timed
phases, pallet-labelled hooks, collections on well-known/root/iteration operations,
RPC EOF, timeouts, and event-based cancellation. Existing ordered oracle/phase
comparisons and authenticated public fixture replay remain enabled.

## State queries and follow prefetch (11d84cc working tree)

Measured on 2026-10-25, macOS arm64, Rust **1.93.0**, development builds
(`debug = 0`, default optimization). Endpoint:
`wss://westend-asset-hub-rpc.polkadot.io`. The before executable is 11d84cc plus
proof-round counters and a per-block counter log; its acquisition behavior is
unchanged. After is this uncommitted implementation. Each follows **finalized**
heads with two workers, eight proof slots, 32-key batches and a separate initially
empty persistent cache. Compilation and tests were stopped during both reported
windows. No collector or local node was started.

Exact commands (each interrupted with SIGINT at 120 seconds):

```sh
/tmp/frame-trace-state-bench/before follow \
  --rpc wss://westend-asset-hub-rpc.polkadot.io \
  --cache-dir /tmp/frame-trace-state-bench/before-cache

target/debug/frame-trace follow \
  --rpc wss://westend-asset-hub-rpc.polkadot.io \
  --cache-dir /tmp/frame-trace-state-bench/after2-cache
```

Warm statistics exclude the first two completed startup jobs in each window:
**12 before / 16 after** warm completions. Counts include context RPC; measured
execution itself uses zero RPC.

| Metric | Before | After |
| --- | ---: | ---: |
| Warm RPC calls/block, median | 41 | 33.5 |
| Warm RPC range | 38–45 | 30–49 |
| Sequential proof rounds/block, median | 28 | 15.5 |
| Proof-round range | 26–30 | 13–17 |
| Warm seconds/block, median | 12.0255 | 10.034 |
| Warm seconds range | 10.588–14.808 | 8.117–13.677 |
| Completed in 120 seconds, including startup | 14 | 18 |
| Throughput, blocks/minute | 7.0 | 9.0 |

A proof round is one strict demand request or one bounded parallel group of
prefetch requests that actually makes RPC. Cache-only groups do not count.
This counts acquisition dependencies, **not each parallel request**; retries are
inside their original round. Four sequential context RPC calls precede acquisition
for these hash-selected follow jobs, giving median **32 → 19.5** logical
sequential stages including context. RPC counts include individual retry attempts.

The first warm before block is **#18561570**,
`0xe7c0d5c3e2405c42310460bfd74f46032b0f16d579969103e3696de2f8e67670`:
44 RPC, 27 proof rounds, 14.808 seconds. The first warm after block is
**#18562313**,
`0x3589f24a84a4489a1911deed6d61399473bb0beb0cb910a7cf54579c8b248b50`:
49 RPC, 15 proof rounds, 10.692 seconds. Thus speculative overfetch can increase
request count on an individual block; it is not an unconditional RPC reduction.

The measured windows contain different public blocks and election-snapshot work:
before warm traces have roughly 567–587 operations; after has 550–552. Endpoint
conditions also vary. These figures are an observed comparison, not an isolated
same-block speedup. All **32** completed follow replays passed the real computed /
header root check and reported **zero measured RPC calls**. Follow still samples
near the tip when its queue fills; 9 blocks/minute does not keep up with this chain.
Both clients exited successfully after SIGINT, with no forced kill.

### Acquisition changes and remaining dependencies

Before loading runtime code, one bounded acquisition stage predicts full storage
keys and trie miss paths from the last three successful replays. Every lookup
walks from the new parent root and skips paths already covered by authenticated
cache nodes, including proven absence. Missing iteration/root-calculation paths
are remembered along with ordinary reads. Up to eight existing discarded dry runs
can collect another set of pending keys; their output is never exported.

A preliminary experiment retaining only previous paths did not materially shorten
the miss chain. Diagnostic key decoding identified sequential
`VoterList::ListNodes` traversal followed by new `Staking::Bonded`, `Ledger` and
`Nominators` lookups: the next block visits different accounts. Consequently, a
new 32-byte identifier in a checked concat-hashed key now also predicts related
map layouts observed in recent blocks. At most **31** extra keys join its proof
request, including pending requests from dry runs. Incorrect guesses cost
bandwidth; only traversal from the current authenticated root can supply values.
This is generic layout inference, with no hard-coded staking pallet keys.

Previously unseen identifiers, other key layouts, linked-list dependencies,
child-trie requests and context RPC still cause sequential work. The final run
cuts those waits substantially but does not eliminate discovery. Raw benchmark
logs, per-block rows, exact commands/window/exit records and summaries are in
`/tmp/frame-trace-state-bench/`: `before.log`, `after2.log`, `results.json`,
`before-run.json`, `after2-run.json`. Preliminary/debug logs are separate and are
not used for the table. All benchmark/debug clients and test servers have stopped.

### Call observation and verification

A live finalized-state call at Westend Asset Hub **#18,561,759**,
`0x60c39d38f0135762de46c998a9628e606483e52d4b838ea6d4c5da383b940502`,
executed `AccountNonceApi_account_nonce` for Alice. JSON arguments were encoded
using the block's V16 runtime API types. Local result **520** (`0x08020000`)
matched the node's pinned safe `state_call` oracle. One timed `System::Account`
read used a **3,713-byte execution proof**, with **8 RPC calls**, **2 proof rounds**,
and **0 measured RPC calls**. Runtime code loading is excluded from that proof
size. A call authenticates reads against its state header; **no block root check
applies**. The small public fixture retains the independent oracle bytes and is
re-executed offline. Three People calls cover nonce, transaction-payment info and
metadata versions; their V14 metadata intentionally exercises hex fallback.

Final commands passed:

```sh
cargo +1.93.0 fmt --all --check
cargo +1.93.0 clippy --workspace --all-targets --locked -- -D warnings
cargo +1.93.0 test --workspace --locked --no-fail-fast
cargo +1.93.0 build --locked
```

Exact results: **110 passed, 0 failed, 1 ignored; 0 doctests**. Breakdown:
CLI **21** (5 unit + 16 integration), decode **13** (1 unit + 12 integration),
model **19** (11 unit + 8 integration), OTel **10**, replay **47**
(31 unit + 5 call integration + 11 block integration), plus the ignored public
network test. Subprocess re-executions are part of their parent cases and are not
counted again. Tests cover call input validation, selected-state pinning, local /
oracle match and mismatch, recorded oracle labeling, output/state/proof tampering,
metadata argument/result types, empty call traces, exports, offline save/load,
recent-path expiry, bounded related-key inference, and the existing block oracles.
The sixteen compressed replay fixture files total **11,144,851 bytes**, sharing
People and Westend Wasm nodes and metadata. No commit or push was made.


## Round 3 validation (2026-10-25)

Rust 1.93.0 `fmt --all -- --check`, `clippy --workspace --all-targets --locked --
-D warnings`, and `test --workspace --locked` passed. Exact distinct test counts:
**128 passed, 0 failed, 1 ignored, 0 doctests**. CLI: **28** (6 unit + 22
integration); decode: **16** (3 unit + 13 integration); model: **19** (11 unit + 8
integration); OTel: **10**; replay: **55** (37 unit + 7 call integration + 11 block
integration). The existing public-network block test remains ignored. The final
follow job-generation extraction was additionally checked with all **6** CLI unit
tests; repeated runs and subprocess executions are not counted as new test cases.

Live `DryRunApi_dry_run_call` on Westend Asset Hub at #18,563,012, with input
`0x000000020005000000` (root System.set_code([])), returned the same bytes as
`state_call`, without an extension trap: **24 storage operations**, **3,047
execution-proof bytes**, **zero measured RPC calls**. The saved
`call-westend-dryrun` fixture repeats the regression offline; its stored oracle is
labelled recorded (unverified). The nonce fixture was refreshed to include locally
executed metadata dependencies. Original People/block artifacts still need explicit
unauthenticated metadata fallback for their historical names because they did not
record those additional metadata-only proof paths.

## Round 4 (5a1963e working tree)

Rust **1.93.0** `fmt --all -- --check`, `clippy --workspace --all-targets
--locked -- -D warnings`, and `test --workspace --locked --no-fail-fast` passed.
Exact distinct counts: **139 passed, 0 failed, 1 ignored; 0 doctests**.
CLI **33** (9 unit + 24 integration), decode **17** (4 unit + 13 integration),
model **19** (11 unit + 8 integration), OTel **11**, replay **59** (41 unit +
7 call integration + 11 block integration). The public-network block test remains
ignored. Subprocess executions are included only in their parent test case.

Metadata execution was captured rather than assumed pure. The People fixture's
V16 export makes **11 backend reads of 7 distinct keys**, including constants
backed by storage. Cache entries retain code hash and those dependency values;
each new root authenticates the values, and changes regenerate metadata. A pure
export has no dependencies and reuses by code hash alone. Iteration conservatively
requires the same state root. Tests exercise changed values, root-independent
reuse, two real People parent states, and fresh offline metadata execution after
exporting dependencies from a cache hit.

Three People block artifacts were extended with authenticated metadata-call
nodes from a local E1 chain. Original hashes and independently recorded ordered
oracles remain unchanged. All three now produce names without a fallback or RPC.
Their total compressed growth is **2,490 bytes**; all 17 compressed replay files
now total **11,153,408 bytes**. The temporary local node exited through the E1
wrapper's cleanup path.

The final batch JSON/summary has **518 ops**, **312 reads**: **231 overlay**,
**47 backend warm**, **34 backend cold**. Its **203 duplicates** split into
**177 overlay / 26 warm / 0 cold**. Alice's **48 batch reads** split **42 / 5 / 1**;
her 47 duplicates split 42 overlay / 5 warm. The batch's 146 reads split
115 overlay / 26 warm / 5 cold. The execution proof and sum of per-op attribution
both equal **6,586 bytes**, with zero unattributed bytes. Per-phase bytes are
initialization **3,490**, inherents **634**, batch's own work **827**, nested
post-inherents **686**, finalization **949**. Thus the containing apply-extrinsic
span's 1,513 bytes includes 686 bytes attributed separately to post-inherents.
`System::Account` contributes **332 proof bytes** across the block. Iteration is
counted separately from writes. A recorder-rollback regression retains block-wide
warmth while allowing positive marginal growth when a proof path is reintroduced.

Raw validation logs, saved batch JSON, replay-free summary/report output, and live
measurement logs are retained in `/tmp/frame-trace-round4/` on this host.

### Before follow window

Command: `/tmp/frame-trace-round4/before follow --rpc wss://westend-asset-hub-rpc.polkadot.io --cache-dir /tmp/frame-trace-round4/before-cache`. Window **120.028 seconds**;
**18 completed blocks**, 9.0 blocks/minute;
heights **18564661–18564709**
(sampling can skip heights). Excluding the first two startup completions:
**16 warm samples**, median **9.336 seconds/block**,
range **8.061–11.568 seconds**. Median RPC
calls **37**, sequential proof rounds **16**;
warm operation range **550–696**.
First warm example: **#18564669**, `0x714c27abe54816a11a80b5feaad9c4c7bdb5eb99eabd8dca64821a5fa3cae7f4`,
**9.010 seconds**. Exit **0**, SIGINT shutdown **0.150 seconds**.

### After follow window

Command: `/Users/theo/Projects/parity/frame-trace/target/debug/frame-trace follow --rpc wss://westend-asset-hub-rpc.polkadot.io --cache-dir /tmp/frame-trace-round4/after-cache`. Window **120.060 seconds**;
**17 completed blocks**, 8.5 blocks/minute;
heights **18564718–18564742**
(sampling can skip heights). Excluding the first two startup completions:
**15 warm samples**, median **9.439 seconds/block**,
range **5.641–11.757 seconds**. Median RPC
calls **38**, sequential proof rounds **16**;
warm operation range **550–768**.
First warm example: **#18564720**, `0x761454b7f5a678cb71e010867c1e3ecacc41d60d7835b32bba19e46ce970c324`,
**8.453 seconds**. Exit **0**, SIGINT shutdown **0.200 seconds**.

Both runs used the macOS arm64 host, Rust 1.93.0 development executables, two
workers, eight proof slots, finalized heads, and separate initially empty cache
directories. The baseline executable was preserved from the clean 5a1963e checkout
before edits. Windows ran sequentially with compilation, tests, and fixture replay
stopped. All **35 completed blocks** passed the real root check and made **zero
measured RPC calls**. Finalized-head availability and block contents differed.
The observed **9.336 → 9.439 seconds/block** and **18 → 17 completions** do **not**
demonstrate an end-to-end speedup. Metadata reuse removes repeated runtime
metadata generation, but still authenticates storage dependencies and does not
remove other sequential proof fetches. New accounting also adds tracing overhead.

Both clients exited normally on SIGINT. The local node, all test/mock processes,
and benchmark clients have stopped. No commit or push was made.

The final live `report --last 2 --proof-concurrency 2` smoke check replays
two finalized Westend blocks concurrently using the warmed after-cache: **1,102
operations**, **183,374 attributed proof bytes**, **0 unattributed bytes**. Both
roots pass with zero measured RPC calls. Files: `live-report.json`,
`live-report.txt`, `live-report.log`. Workers fork the shared RPC transport to
isolate their measured-run counters; the CLI regression also schedules two
authenticated fixture replays concurrently and checks both zero-RPC guards.
The complete workspace suite was rerun after this correction: the exact count
remains **139 passed, 0 failed, 1 ignored; 0 doctests**.

## Round 5 (e86f740 working tree, 2026-10-25)

Rust **1.93.0** `fmt --all -- --check`, `clippy --workspace --all-targets
--locked -- -D warnings`, and `test --workspace --locked --no-fail-fast` passed.
Exact distinct counts: **154 passed, 0 failed, 1 ignored, 0 doctests**.
CLI **42** (16 unit + 26 integration), decode **18** (5 unit + 13 integration),
model **19** (11 unit + 8 integration), OTel **11**, replay **64** (46 unit +
7 call integration + 11 block integration). The existing public-network test is
ignored. Repeated probe runs and child-process executions are not additional cases.

All eleven review findings are addressed. The Append regression records
**Append = 148 B; unrelated overlay Get = 0 B**. Additional probes cover an
Append served by the overlay followed by a backend read, iteration-owned growth,
and recorder warmth after iteration/rollback. Capture observes hashes through a
wrapper around the SDK recorder and keeps no duplicate node bytes; it no longer
copies the full growing proof to establish read warmth.

Metadata validation reads complete authenticated top-level and child values,
including V1 external value nodes. Root-bound cache entries retain metadata proof
nodes for iteration exports. Tests save after cache hits and execute metadata in
**fresh processes** for top-level reads, child reads, iteration, storage roots and
child roots. People reuse checks exact equality with the first execution.
StorageRoot/ChildStorageRoot dependencies invalidate on a changed state root.

SDK source finding: legacy `construct_runtime!` passes declaration order to both
`decl_all_pallets` and `expand_runtime_metadata`; metadata IR V14/V15/V16 conversion
maps that vector without sorting. The newer runtime macro may sort both by index
before expansion (unless legacy ordering is enabled). Preserve metadata's tuple
order, rather than independently sorting indices. OnIdle rotates the **complete**
tuple by block number before filtering instances. A synthetic V16 fixture uses
instance indices 90 then 1, with System between them, and tests declaration order,
rotation, unknown block numbers and incomplete tracing. Ambiguous cases retain
`DeclaredFirst | DeclaredSecond`.

Report folds each completed worker result immediately and releases its trace.
It retains aggregate statistics, a duplicate-hash set, errors and bounded quantile
data. Samples are exact through 1,024 observations per metric/type, then use at
most 4,097 logarithmic buckets with **1.5625% maximum value error** (no rank-error
claim). Default behavior continues after block/file errors, emits them in JSON and
tables, and exits 1 if any input failed. Cost ranking, omitted-row counts, phase
order, legacy missing evidence, saturating sums, range validation and stable
summary/report tables are covered by tests. The genuine legacy JSON fixture was
built from 5a1963e in a temporary detached worktree, which has been removed.

### Isolated before/after measurements

macOS arm64, Rust 1.93.0 development executables; baseline copied from clean
e86f740 before edits. Tests/compilation had stopped. Westend Asset Hub fixture
**#18,558,320**: one warmup per executable, then three separate timed processes,
with warm compiled Wasm, fresh overlays/recorders and JSON export. All six runs
have **549 ops**, **92,458 proof bytes**, matching attributed sums, passing state
roots and zero measured RPC calls. Read origins remain **142 overlay / 110 warm /
89 cold**.

Median measured capture duration: **68.866458 → 36.488417 ms**. End-to-end median:
**2.19 → 2.14 seconds**. Median process peak RSS: **424,099,840 → 418,316,288 bytes**.
These are local fixture measurements including tracing overhead, not node timings.

Report memory uses **50 repetitions** of that saved Westend JSON with distinct
synthetic input hashes (aggregation only, no replay), **27,450 ops** and
**4,622,900 attributed proof bytes**, using `report … --top 3 --json FILE`.
`/usr/bin/time -l` peak RSS: **34,275,328 → 12,730,368 bytes**
(**32.6875 → 12.140625 MiB**, **62.86% lower**). This also includes buffered file
parsing and terminal/JSON rendering. Raw commands, six block JSON files,
measurements, full validation and Append probe logs are in
`/tmp/frame-trace-round5/`; inputs are in `/tmp/frame-r5-memory/`.

All processes started for this round have exited. HEAD remains e86f740; no commit
or push was made.


## Follow workers and memory (2026-10-26)

The default now traces consecutive Westend Asset Hub heights after startup, rather
than continually dropping queued blocks. The two 120-second default windows
completed **49 and 38 blocks**, with **no expiry skips**. The chain advanced
**56 and 46 best-head heights** in the respective windows. In their final
60 seconds, the runs completed **40 and 37 blocks**, clearing startup backlog;
the repeat ended with **zero queued and six in-flight** blocks. Every completed
replay passed the storage-root check and reported **zero RPC in timed replay**.
This demonstrates keeping up after startup in these windows, not a guarantee for
every endpoint or block. Cold startup and unfinished jobs lower whole-window
completion rates below the chain's production rate.

All binaries are Rust **1.93.0 release builds**, on macOS arm64. Before is clean
**3802322**, built from the detached worktree `/tmp/ft-perf/before`, with its own
`CARGO_TARGET_DIR=/tmp/ft-perf/before-target`. After is the uncommitted working
tree. Endpoint: `wss://westend-asset-hub-rpc.polkadot.io`. Each run uses a separate,
initially empty `--cache-dir`. OTLP export is disabled. Follow defaults to finalized
heads; chain rate uses best-head header numbers immediately before and after each
window. Those bracketing HTTP calls add a small timing difference to the chain
rate. These are different blocks and intervals, not a same-block latency comparison.
The chain ran at **23–30.5 blocks/minute** here, rather than the earlier observed
17 blocks/minute.

Unrelated PolkaMeter Rust builds were running on the host throughout these tests.
They were **not paused, signalled or killed**. At the user's direction, runs were
sequential despite that load. Initial workspace verification was also finishing
in the background during the early windows. Each boundary has an `uptime` capture
and the full list of cargo/rustc processes in `/tmp/ft-perf/*-host-{start,end}.txt`.
No frame-trace build overlapped a timed window. Load and endpoint conditions limit
attribution: the before repeat's warm median rose from 8.952 to 11.745 seconds
while user CPU stayed similar and host load fell. Both windows are reported.
The chosen default was repeated too; its warm medians were 12.194 and 12.178 seconds.

### Release windows

Every window is **120 seconds**. Completions after its boundary are excluded.
"Replay / chain" is completed blocks/minute versus the bracketing chain rate.
Warm medians omit the first N completions, where N is the worker count. Startup
jobs can overlap later jobs, so this convention is an approximation of a warm
session. RSS is `/usr/bin/time -l` maximum resident set size in decimal **MB**;
user CPU is process CPU **seconds**, not replay wall time.

| Run | Workers / RPC slots | Completed blocks | Replay / chain, blocks/min | Warm median, seconds/block | Peak RSS, MB | User CPU, seconds | Heights skipped |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| Before | 2 / 8 | 20 | 10.0 / 30.5 | 8.952 | 738.8 | 11.31 | 24 expiry skips |
| Trial: 6 workers | 6 / 8 | 31 | 15.5 / 30.5 | 13.926 | 805.7 | 13.54 | 1 expiry skips |
| Trial: 8 workers | 8 / 8 | 37 | 18.5 / 30.0 | 17.324 | 797.7 | 14.29 | None |
| After default | 8 / 16 | 49 | 24.5 / 28.0 | 12.194 | 752.0 | 16.08 | None |
| After higher workers | 12 / 16 | 46 | 23.0 / 26.5 | 11.287 | 779.6 | 14.77 | None; 8 cancelled at shutdown, including #18615501 |
| Before repeat | 2 / 8 | 18 | 9.0 / 30.0 | 11.745 | 819.3 | 10.90 | 22 expiry skips |
| After default repeat | 8 / 16 | 38 | 19.0 / 23.0 | 12.178 | 698.3 | 14.49 | None |

The six/eight-worker trials retained eight shared RPC slots and let all startup
jobs run cold together. Increasing workers alone improved throughput but did not
keep up. The final configuration uses **eight workers and 16 shared slots**,
with one cold replay warming the session first. The 12-worker run did not improve
whole-window completions; its chain interval was slower too. It is not evidence
that 12 workers are always worse. The default of eight is a judgement, not a
measured optimum. The six- and eight-worker trials ran before the cold-start gate
and the 16 slots. The final configuration ran once at 12 workers and twice at
eight, on a loaded host. Those windows cannot separate the worker count from
endpoint and host variance.

Default completion ranges were **#18615397–#18615445** and
**#18615607–#18615644**, both consecutive. The 12-worker run completed
**#18615456–#18615502** except **#18615501**, which was still in flight at Ctrl-C.
That run's shutdown line was `follow shutdown: skipped=0 cancelled=8`: #18615501
and seven other queued or in-flight heights were cancelled, not expired or rejected. Before windows had both explicit expiry
skips and missing completed heights. Shutdown cancellations are not counted as
successful replays. All seven clients exited successfully on one Ctrl-C.

### Host load at each boundary

Each cell lists the **1/5/15-minute load averages**, then the numbers of
**cargo/rustc/cargo-clippy** processes. The raw files retain their PIDs, commands,
CPU use and elapsed times. Load averages are host run-queue measurements, not
frame-trace CPU percentages.

| Run | Start: load averages (process counts) | End: load averages (process counts) |
| --- | --- | --- |
| Before | 133.47 105.26 72.04 (7/64/2) | 112.16 110.75 79.06 (8/30/2) |
| Trial: 6 workers | 112.16 110.75 79.06 (8/30/2) | 78.07 99.99 79.07 (3/19/0) |
| Trial: 8 workers | 78.07 99.99 79.07 (3/18/0) | 86.60 93.15 79.02 (2/4/0) |
| After default | 65.19 85.49 77.26 (2/8/0) | 44.39 71.88 72.99 (1/0/0) |
| After higher workers | 44.39 71.88 72.99 (1/0/0) | 39.79 61.31 68.66 (4/14/1) |
| Before repeat | 39.79 61.31 68.66 (4/12/1) | 43.14 55.49 65.37 (1/11/1) |
| After default repeat | 43.31 52.36 63.32 (3/34/2) | 57.33 54.78 62.85 (3/18/2) |

### What changed and why

- `follow --workers` accepts **1–16**, default **8**. The waiting queue holds four
  blocks per worker, at least eight. Its recent-height floor uses that same bound.
  Thus the default can absorb a cold first replay without expiring nearby heights.
  Startup still begins at the current finalized tip; there is no pre-start backfill.
- `Rpc::fork` already shares its semaphore and connection pool, while resetting
  each worker's request counter. That isolation is essential to zero-RPC checking.
  The follow default for `--proof-concurrency` is now **16**, shared globally,
  not multiplied by workers. Other commands retain their existing defaults.
  The eight-slot trials show why more workers need enough shared transport capacity.
- In `follow`, one cold replay runs first. Other workers reuse its
  authenticated node cache and learned paths, then run concurrently. Failure,
  panic or cancellation releases the startup gate so another job can try. The
  gate is opt-in (`ReplaySession::cold_start`); `report` and other callers do
  not use it. No worker accepts another
  block's values without traversing its own parent root.
- Parent header and runtime version are fetched concurrently, at the same parent
  hash. Both requests are joined before replay. The block body must arrive first
  to identify that parent; selector resolution remains a dependency too.
- Immutable proof-node bytes now use `Arc<[u8]>` in the session and active block
  databases. SDK trie reads still return owned bytes as required by its interface.
  Per-block proof coverage, overlays, recorder state and failures remain separate.
  Offline artifacts cannot obtain missing nodes from an online session.
- Compiled executors and raw authenticated metadata were already shared by code
  hash. The CLI now limits Wasmtime's Rayon compilation pool to **two threads**
  for follow; the replay library does not change an embedding application's pool.
  This reuses the existing Rayon dependency; Cargo.lock includes its direct CLI edge.

### Where the memory went

Separate exploratory probes used `vmmap -summary` and `heap -s`; they are not
throughput windows because inspection can suspend the target. In the baseline's
cold profile, live malloc allocations were about **255 MiB**. After compilation,
`heap` counted **54,039,872 live allocated bytes**. A later `vmmap` snapshot showed
about **591 MiB resident in malloc regions** but only **52 MiB allocated** there.
This identifies transient compilation and retained allocator pages as major
contributors, rather than two independent compiled runtimes. The SDK executor
and its compiled-module cache were already shared. This is not a complete byte
accounting for every subsystem.

The SDK's pool also reserved about **258 GiB of virtual address space** in that
profile, with roughly **50 MiB resident** during the cold snapshot. Virtual
reservations are not laptop RAM consumption. Proof nodes and metadata are much
smaller than the cold compilation peak; avoiding their duplicate allocations
still matters as worker count grows.

The baseline memory probe peaked at **807.3 MB RSS** with two workers. A four-worker
probe with `RAYON_NUM_THREADS=2` and shared nodes peaked at **681.4 MB**. Different
blocks, host load and inspection mean that is supporting evidence, not an isolated
Rayon comparison. The actual final windows are stronger operational evidence:
eight workers used **698.3–752.0 MB**, versus **738.8–819.3 MB** in the two-worker
before windows. The chosen default does not multiply baseline RSS by four.

### Exact commands and retained evidence

Run from `/Users/theo/Projects/parity/frame-trace` unless a subshell changes it:

```sh
git worktree add --detach /tmp/ft-perf/before 3802322
(cd /tmp/ft-perf/before && CARGO_TARGET_DIR=/tmp/ft-perf/before-target cargo +1.93.0 build --release --locked)
cargo +1.93.0 build --release --locked
```

The baseline build ended with:

```text
    Finished `release` profile [optimized] target(s) in 40.23s
```

The benchmark harness and its complete source are retained at
`/tmp/ft-perf/bench.py`. It wraps the binary in `/usr/bin/time -l`, timestamps each
output line, samples RSS, reads chain headers, and sends SIGINT to the client at
120 seconds. The OS time wrapper runs slightly longer/shorter than the harness
window because of process startup/shutdown. Commands, in order:

```sh
python3 /tmp/ft-perf/bench.py before-final /tmp/ft-perf/before-target/release/frame-trace
python3 /tmp/ft-perf/bench.py after-six /Users/theo/Projects/parity/frame-trace/target/release/frame-trace --workers 6
python3 /tmp/ft-perf/bench.py after-eight /Users/theo/Projects/parity/frame-trace/target/release/frame-trace --workers 8 -- --workers 8
# Build the final cold-start/16-slot configuration after those trials.
cargo +1.93.0 build --release --locked
python3 /tmp/ft-perf/bench.py after-default /Users/theo/Projects/parity/frame-trace/target/release/frame-trace --workers 8
python3 /tmp/ft-perf/bench.py after-high /Users/theo/Projects/parity/frame-trace/target/release/frame-trace --workers 12 -- --workers 12
python3 /tmp/ft-perf/bench.py before-repeat /tmp/ft-perf/before-target/release/frame-trace
python3 /tmp/ft-perf/bench.py after-default-repeat /Users/theo/Projects/parity/frame-trace/target/release/frame-trace --workers 8
```

The first three windows use the first implementation: default six workers,
eight RPC slots, shared node bytes, concurrent context requests, and two compiler
threads. The final four use the final configuration (or the unchanged baseline).
`--workers` before `--` is a harness parameter for the warm-sample exclusion; flags
after `--` are passed to follow. Default windows pass no follow flags except the
endpoint and a fresh cache directory. Each JSON file records the full actual command.

Real final-default output includes:

```text
root check: PASS; RPC calls: 92 (timed replay: 0); execution proof: 92281 bytes
fetching state #18615397: 63 proof request rounds; 92 RPC calls
...
traced finalized block #18615397 0xcdaa5219de6f7405d6189e42489cb0e2ffe6b77d042f6ffed70c4df54f5a1165: 550 storage operations in 36.737s
```

For each run, `/tmp/ft-perf/<name>.log` retains raw output and `/usr/bin/time -l`
statistics; `<name>-timestamped.log` records completion boundaries;
`<name>.json` holds headers, completed heights and durations; `<name>-rss.log`
retains RSS samples. Memory probe files use the `memory-probe` and
`memory-two-threads` prefixes. Initial exploratory probes ran 65 seconds and
used a process-group signal; they exited 130 and are excluded from the release
tables. The final harness signals only the client, and all tabulated runs exited 0.
All started clients and inspection tools were stopped. No Docker node was started.
The only extra target directory is inside `/tmp/ft-perf`.


### First-block latency and the compiler cap (review round)

The first traced block in the 120-second windows was slower after the change:
**32.4–47.8 s** in the three final-configuration windows, against **26.8–29.3 s**
in the two before windows (and 43.4–44.6 s in the pre-gate trials). Each window
replayed a different block under a different host load, so this was not a
same-block comparison. The suspect was the two-thread Rayon cap, which limits
Wasmtime's parallel compilation during the cold replay.

Isolated test: release build of the review-round tree, one fixed finalized block
(#18616746, `0x7f8777b87243d806d0827bfca7d2f3a14ae8f2fd95a43f8b7a2e117f217f6085`),
`frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block <hash>
--cache-dir <new empty dir>`, so each run fetched all state and compiled the
runtime cold. `block` does not set the pool itself, so the cap was emulated with
`RAYON_NUM_THREADS=2`; "uncapped" uses Rayon's default (12 threads on this host).
Runs alternated. Host 1-minute load fell from 14.2 to 5.3 across the series.

| Run | Rayon threads | Wall time, s | User CPU, s | Peak RSS, MB |
| --- | --- | ---: | ---: | ---: |
| 1 | 12 (default) | 25.96 | 7.89 | 852.5 |
| 1 | 2 | 26.45 | 6.33 | 715.4 |
| 2 | 12 (default) | 28.88 | 7.89 | 805.2 |
| 2 | 2 | 26.18 | 6.28 | 734.3 |
| 3 | 12 (default) | 29.36 | 7.95 | 873.1 |
| 3 | 2 | 27.01 | 6.25 | 755.6 |

Every run passed the root check with 92 RPC calls and zero timed RPC. The cap did
not add startup latency: **26.2–27.0 s** capped versus **26.0–29.4 s** uncapped.
It lowered peak RSS by **71–137 MB** per pair and user CPU by about 1.6 s.
The cap stays at two threads.

The 16 shared RPC slots were checked the same way, with `RAYON_NUM_THREADS=2`:
`--proof-concurrency 16` took **28.37 and 28.75 s**, and 8 took **27.68 and
27.46 s**. That difference is about one second, not the ten seen between windows.
A short `follow` probe on the same binary at host load 4.2 traced its first block
(#18616911) in **26.969 s**, inside the before range. The slower first blocks in
the final windows are therefore attributed to host load and block/endpoint
variance, not to the cap or the slot count. That probe also exited **55 ms** after
SIGINT (`follow shutdown: skipped=0 cancelled=12`), with workers still waiting at
the cold-start gate. Raw output: `/tmp/ft-perf/rayon/` (`summary.txt`, per-run
`.log` with `/usr/bin/time -l`, `-host.txt` uptime captures, `follow-probe.log`).

### Final verification

All required checks passed on the final working tree:

```sh
cargo +1.93.0 fmt --check
cargo +1.93.0 clippy --workspace --all-targets --locked -- -D warnings
cargo +1.93.0 test --workspace --locked
mdbook build
```

Fmt produced no output. Clippy finished successfully without warnings. mdBook
logged only its normal build-start and HTML-backend messages, with no warnings.
Workspace tests reported **182 passed, 0 failed, 1 ignored**, plus **0 doctests**.
Breakdown: CLI **21 unit + 37 integration**; decode **5 unit + 13 integration**;
model **11 unit + 8 integration**; OTel **15 integration**; replay **54 unit + 7 call
integration + 11 offline integration**, with the public-network case ignored.
The five added tests cover worker defaults/range validation, queue ancestry bounds,
concurrent context requests and shared transport counters, immutable node sharing
and offline isolation, and cold-start success/failure handoff. All existing
conformance, tampering and root-check tests remain enabled.

After the review round the same checks pass with **187 passed, 0 failed,
1 ignored** (replay **59 unit**; other suites unchanged). The five new tests cover
a queued RPC call keeping its full request deadline after waiting for a shared
slot, cancellation of a call queued for a slot, and release of the opt-in
cold-start gate after an error, a panic and cancellation of a waiter. The
gate-handoff test now asserts that the follower is still blocked before the
first replay's guard drops. Log: `/tmp/ft-perf/tests-round2.log`.

The final logs are `/tmp/ft-perf/fmt-final.log`, `clippy-final.log`,
`tests-final.log` and `mdbook-final.log`. A separate log check confirmed **239**
completed benchmark replays with matching roots, `PASS`, zero timed RPC, and
successful client exit across the seven tabulated windows.
