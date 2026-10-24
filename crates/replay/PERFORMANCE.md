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
