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
