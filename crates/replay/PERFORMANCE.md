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
