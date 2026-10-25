# E2: self-replay from safe RPC, with a real storage-root check

Run on 2026-10-25, Apple M4 Pro (12 cores), macOS 15.8.1, Rust 1.93.0. **The hypothesis works for the tested blocks.** All eight cases pass on both People runtime variants: 16 exact, ordered oracle matches and 16 successful `Core_execute_block` root checks. A recent public Westend Asset Hub block also passes, using only safe RPC. Recommend self-replay as frame-trace's primary block source, subject to the production gaps below.

The standalone crate has its own `[workspace]`; it does not join or edit the root workspace. Nothing was committed or pushed. Every node started by this experiment was stopped by E1's `run-node.py` ownership wrapper, including the unsuccessful harness attempt.

## Storage-root design: authenticated, demand-paged trie, then offline verification

This implements option **(c), a refinement of (b)**. There is no values-only backend, fabricated root, skipped root assertion, patched runtime, `TryRuntime_execute_block`, or initialize/apply/finalize substitution.

1. Fetch the block, its parent header and the parent's runtime version. Verify the parent header's hash and pin every proof request to that parent hash. Remove seals from the replay header, as `sc-tracing` does.
2. Authenticate the parent's `:code` and `:heappages` through a real `TrieBackend`, rooted at the parent's published state root. The runtime is obtained from proof nodes rather than trusting a separate unproved value read.
3. Run the SDK `WasmExecutor` + `StateMachine` + ordinary `sp-io` host functions, with the same extra host-function families as omni-node: storage-proof-size, statement-store, and RFC163 elliptic-curve functions. People imports statement-store functions even when these extrinsics do not call them; omitting that family fails runtime instantiation.
4. `ProofDb` implements `TrieBackendStorage`. A missing hash arrives with the SDK's **nibble path** to that trie node. Request `state_getReadProof` for that path, padding an odd final nibble with zero. A proof for that potentially absent or partial storage key includes the node being traversed. When a full logical read key is available, a hint requests its entire path in one call. Check the proof's `at` hash, hash and cache its nodes, then let the SDK authenticate them by traversing from the real root. Nodes that do not connect to that root cannot supply a valid traversal.
5. This also resolves nodes needed only by writes, iteration, final root calculation and branch collapse after deletion. A naive list of keys observed by `Get` would miss some of these. No whole-state scrape or `state_getKeysPaged` is needed for these tests.
6. Register a **fresh** `sp_trie::recorder::Recorder` and `ProofSizeExt` for each execution. Fetching/cache-loading nodes does not pre-seed that recorder. It grows only as the SDK traverses the trie, so parachain proof-size-dependent fee reclaim works. Parent runtime loading uses a separate backend without this recorder.
7. After online discovery succeeds, disable all proof fetching. Replay five times with a new overlay, backend and recorder each time. Each measured replay executes the real `Core_execute_block`, including signatures and root assertions, using only cached nodes. A cache miss is an error. The RPC count must remain unchanged.

Only four RPC methods were actually used by Rust: `chain_getBlock`, `chain_getHeader`, `state_getRuntimeVersion`, `state_getReadProof`. An explicit allowlist in `src/rpc.rs` prevents using unsafe or node-local methods. `state_traceBlock` exists only in the separate JavaScript **test oracle** and is excluded from all replay RPC counts.

`--offline ARTIFACT_DIR` reconstructs another process entirely from the exported proof and block. It passed for both the local batch and public block, with zero RPCs, after the local node had stopped. A negative control changes the expected block state root to `0x4242…4242`: execution computes the original, authentic root, then traps in executive final checks. `results/root-tamper-control.json` records both roots. This checks that the root assertion is active.

## Prior art read before implementation

- **frame-remote-externalities:** read the requested `/tmp/pm-audit/polkadot-sdk` clone and the pinned stable source. It downloads keys/values for selected prefixes or the whole state, includes child state, and builds `TestExternalities`/snapshots. It is a useful bulk-scrape implementation; partial value snapshots cannot reproduce an arbitrary chain root. E2 uses SDK trie/proof machinery instead of rebuilding that scraper. [Stable implementation](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/utils/frame/remote-externalities/src/lib.rs).
- **try-runtime-cli**, commit `6e1c4e95e76c7deee7a19bc05ae2496dda0ee0be`: `execute-block` loads parent state with remote-externalities and calls `TryRuntime_execute_block`, hardcoding state-root and signature checks to false. `fast-forward` mines new blocks on the loaded state; it is not proof that an existing canonical block reproduces its root. Reused the executor/state-machine integration pattern, not those relaxed checks. [Execute block](https://github.com/paritytech/try-runtime-cli/blob/6e1c4e95e76c7deee7a19bc05ae2496dda0ee0be/core/src/commands/execute_block.rs), [fast forward](https://github.com/paritytech/try-runtime-cli/blob/6e1c4e95e76c7deee7a19bc05ae2496dda0ee0be/core/src/commands/fast_forward.rs).
- **Chopsticks**, commit `3930ed4dac15a77ab5513cc2aea980bd366e4a38`: `RemoteStorageLayer` fetches values lazily and caches/in-flight-deduplicates RPC requests. Its run-block command uses initialize/apply/finalize. Its smoldot adapter uses a constant proof-size response, resumes unknown descendant Merkle values, and skips branch-node requests during root calculation; that path does not establish the canonical state-root assertion E2 needs. [Storage layer](https://github.com/AcalaNetwork/chopsticks/blob/3930ed4dac15a77ab5513cc2aea980bd366e4a38/packages/core/src/blockchain/storage-layer.ts), [run-block](https://github.com/AcalaNetwork/chopsticks/blob/3930ed4dac15a77ab5513cc2aea980bd366e4a38/packages/chopsticks/src/plugins/run-block/cli.ts), [executor adapter](https://github.com/AcalaNetwork/chopsticks/blob/3930ed4dac15a77ab5513cc2aea980bd366e4a38/executor/src/task.rs).
- **smoldot**, commit `82b4cf1f8c20d63131441602c15f6bf6576dd236`: its resumable runtime-call executor already models storage gets, next-key requests, descendant Merkle values, overlays and root calculation. It is good prior art for lazy execution. E2 uses the requested SDK executor so it directly reuses SDK host events, Wasm tracing transport and the same trie recorder. [Runtime-call executor](https://github.com/smol-dot/smoldot/blob/82b4cf1f8c20d63131441602c15f6bf6576dd236/lib/src/executor/runtime_call.rs).
- **sc-tracing and cumulus:** read the stable block replay and E1's [proof-aware dev-node patch](../e1/omni-dev-proof-replay.patch). Reused seal stripping and synchronous dispatch, but replaced the lossy block subscriber. Cumulus's existing replay can use authoring-time proof-size observations from node-local auxiliary storage. Those observations are not available through these safe RPCs. E2 instead recomputes sizes with a fresh trie recorder; the exact roots and oracle values agree for every local signed case. This does not prove every future authoring/import path will have identical observations. [Block replay](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/client/tracing/src/block/mod.rs), [parachain replay](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/cumulus/client/service/src/lib.rs), [proof-size extension](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/primitives/trie/src/proof_size_extension.rs).

## Capture and timing

`src/capture.rs` accepts **every event with target `state`**, whether or not it has `key` or `method`. It records ordered events, monotonic timestamps, fields, span creation, entry/exit, inclusive durations and parents. For the Wasm transport's explicit `None` parents, the currently entered synchronous span supplies the parent. All recorded span/event intervals and parent references pass `verify.py`.

The traced batch contains this actual nesting, with each storage event additionally parented under its native storage host-call span:

```text
replay
  execute_block
    apply_extrinsic                 # extrinsic 2
      batch
        transfer_keep_alive
        transfer_keep_alive
        transfer_keep_alive
```

`src/backend.rs` adds backend next-key, child next-key and raw iterator advancement events with elapsed nanoseconds. Native `sp_io::storage::next_key_version_1` spans time complete host calls. **Backend events describe backend attempts**: overlay merging can change the final returned key or cause several backend attempts for one host call. Do not count backend events as one-to-one runtime next-key calls. The real People cases exercise prefix-clear iteration (one iterator advancement per ordinary block, three in the deletion block); they do not exercise a direct runtime next-key call.

A separate host conformance test exercises direct top/child next-key calls, an overlay successor, `ChildClearPrefix`, `ChildKill`, `ChildStorageRoot`, `ClearPrefix` and `StorageRoot` against a complete in-memory trie. It checks the returned keys, omitted event coverage and nonzero host timings. This is **host coverage, not remote child-proof coverage**.

Timings below are local, instrumented Wasm replay timings, not the original chain's execution time or database I/O latency. Storage durations include tracing/recorder overhead. Native storage spans work without runtime tracing. Exact executive extrinsic durations and dispatch/hook nesting require a runtime compiled with `with-tracing`; neither is fabricated when it is absent.

## Local results

Reused E1's two Wasms and scratch proof-aware omni-node. Each fresh chain submitted transfer, remark, a three-transfer batch, PeopleLite allowance grant/clear, 4 KiB raw write, and prefix deletion. Inclusion and dispatch success (including sudo inner success and all three batch transfers) are checked on-chain. The deleted probe key is null afterwards.

The oracle comparison checks the **entire ordered keyed event list and every field**, excluding only random `ext_id`. It includes full results/values and the 4 KiB write, not just method/key pairs. Full hashes, RPC counts, sample timings and proof sizes are retained under [results/traced](results/traced/) and [results/untraced](results/untraced/).

### Runtime with tracing

CP = extra `ClearPrefix`; SR = extra `StorageRoot`. Every row also captures the previously discarded `Call`/`Return` runtime API boundary events. Warm replay is the median of five runs with zero RPC calls. Acquisition includes the initial RPC work, authenticated Wasm loading, first executor invocation and discovery; a compiled-module disk cache already existed for this traced run. RPC counts include three context requests and the remaining read-proof requests; oracle calls are separate.

| Block / call | Keyed events | Oracle match | Root check | Extra CP / SR | Warm replay ms | Acquisition ms | Safe RPC calls |
| --- | ---: | :---: | :---: | ---: | ---: | ---: | ---: |
| 1 / Inherents | 315 | yes | pass | 1 / 1 | 1.476 | 300.7 | 31 |
| 2 / transfer_keep_alive | 418 | yes | pass | 1 / 1 | 1.879 | 277.5 | 51 |
| 3 / remark | 391 | yes | pass | 1 / 1 | 1.787 | 277.1 | 50 |
| 4 / batch (3 transfers) | 515 | yes | pass | 1 / 1 | 2.213 | 277.7 | 51 |
| 5 / PeopleLite grant (sudo) | 412 | yes | pass | 1 / 1 | 1.845 | 282.0 | 52 |
| 6 / PeopleLite clear (sudo) | 411 | yes | pass | 1 / 1 | 1.837 | 277.9 | 53 |
| 7 / 4 KiB set_storage (sudo) | 406 | yes | pass | 1 / 1 | 2.133 | 281.3 | 51 |
| 8 / kill_prefix (sudo) | 405 | yes | pass | 2 / 1 | 1.839 | 282.7 | 52 |

### Original runtime without tracing

The same eight oracle matches and root checks pass. Native storage timing remains available; there are zero Wasm spans and no exact executive extrinsic timings. The first acquisition additionally populated the compiled-module cache for this Wasm.

| Block / call | Keyed events | Oracle match | Root check | Extra CP / SR | Warm replay ms | Acquisition ms | Safe RPC calls |
| --- | ---: | :---: | :---: | ---: | ---: | ---: | ---: |
| 1 / Inherents | 315 | yes | pass | 1 / 1 | 1.280 | 1057.9 | 31 |
| 2 / transfer_keep_alive | 418 | yes | pass | 1 / 1 | 1.565 | 258.7 | 51 |
| 3 / remark | 391 | yes | pass | 1 / 1 | 1.432 | 258.2 | 50 |
| 4 / batch (3 transfers) | 515 | yes | pass | 1 / 1 | 1.730 | 263.7 | 51 |
| 5 / PeopleLite grant (sudo) | 412 | yes | pass | 1 / 1 | 1.383 | 264.3 | 52 |
| 6 / PeopleLite clear (sudo) | 411 | yes | pass | 1 / 1 | 1.350 | 262.6 | 53 |
| 7 / 4 KiB set_storage (sudo) | 406 | yes | pass | 1 / 1 | 1.578 | 260.8 | 51 |
| 8 / kill_prefix (sudo) | 405 | yes | pass | 2 / 1 | 1.355 | 261.5 | 52 |

### Per-extrinsic measurements with the traced runtime

Durations are medians of the same five warm runs. Index 0 is `ParachainSystem.set_validation_data`; index 1 is `Timestamp.set`; index 2 is the signed call. All three transfers in the batch remain inside index 2.

Proof bytes are the recorder's encoded-size increase during that extrinsic, from the last warm run (identical replay state). They are **marginal block-execution proof bytes**, not standalone extrinsic proofs, weights, total fetched proof size or PoV bytes. Initialization has already recorded some nodes; final root calculation can record further nodes after the extrinsics. Start/end sizes for every extrinsic are in the JSON results.

| Block | Extrinsic 0 ms | Extrinsic 1 ms | Extrinsic 2 ms | Extrinsic 2 incremental proof bytes |
| --- | ---: | ---: | ---: | ---: |
| 1 / Inherents | 0.228 | 0.047 | — | — |
| 2 / transfer_keep_alive | 0.239 | 0.046 | 0.381 | 1409 |
| 3 / remark | 0.250 | 0.046 | 0.267 | 1314 |
| 4 / batch (3 transfers) | 0.247 | 0.046 | 0.667 | 1513 |
| 5 / PeopleLite grant (sudo) | 0.250 | 0.047 | 0.332 | 1578 |
| 6 / PeopleLite clear (sudo) | 0.241 | 0.047 | 0.327 | 1575 |
| 7 / 4 KiB set_storage (sudo) | 0.245 | 0.045 | 0.404 | 1608 |
| 8 / kill_prefix (sudo) | 0.249 | 0.047 | 0.328 | 1690 |

## Public endpoint

Used **HTTPS** on the user-suggested Westend Asset Hub service, `https://westend-asset-hub-rpc.polkadot.io`; this prototype's transport is HTTP(S), not WebSocket. No public unsafe RPC was requested.

- Block **18558320**, hash `0x4c0d1d575c92a57445db93a0204dd77c5d94e110663c5dd8a55fb94052e35363`.
- Runtime `westmint`, specVersion `1025002`, stateVersion `1`; body has two inherents. Public signed traffic was not exercised; the local signed cases cover that path.
- **Root check passes**, including five offline warm runs. Oracle match: **not requested / N/A** on the public endpoint.
- Acquisition **38,149.6 ms**; warm replay median **3.855 ms**, five samples in `results/public-westend.json`; zero warm RPC calls.
- **133 safe RPC calls**: 1 block, 1 parent header, 1 runtime version, 130 read proofs. All returned HTTP 200 with no JSON-RPC errors or observed rate-limit responses. No retries were needed. This is one endpoint/run, not a rate-limit guarantee.
- 5,894,142 total response bytes; 387 unique fetched nodes / 2,747,791 node bytes, including the 2,650,083-byte compressed runtime. The execution recorder's final proof estimate is 92,458 bytes, excluding runtime-loading work.
- The public runtime has no Wasm spans. All native storage host-call spans have real timing, but exact per-extrinsic timing is unavailable.
- A second process replayed the exported proof with RPC disabled after acquisition: root passes, zero RPC calls. Its ~311 ms first invocation includes loading/instantiating the compiled module; it is not a warm sample.

An earlier unoptimized demand-per-node probe also passed a different recent block, #18558235, with 357 calls / ~113 s. Complete-key hints reduced serial proof requests in the final implementation. That is an engineering observation across different blocks, not a controlled performance comparison.

## Reproduce

From `/Users/theo/Projects/parity/frame-trace`:

```sh
export CARGO_TARGET_DIR=/tmp/pm-audit/e2-target
export CARGO_BUILD_JOBS=8
cargo +1.93.0 build --release --locked --manifest-path dev/experiments/e2/Cargo.toml
cargo +1.93.0 test --locked --manifest-path dev/experiments/e2/Cargo.toml
cargo +1.93.0 clippy --locked --manifest-path dev/experiments/e2/Cargo.toml --all-targets -- -D warnings

export NODE_PATH=/tmp/pm-audit/e1/client/node_modules
export OMNI_NODE=/tmp/pm-audit/e1/target-node/release/polkadot-omni-node
export E2_BINARY=/tmp/pm-audit/e2-target/release/frame-trace-e2
RPC_PORT=19954 LOG=/tmp/pm-audit/e2-node-release-traced.log \
  python3 dev/experiments/e1/run-node.py /tmp/pm-audit/e1/people-traced.json -- \
  node dev/experiments/e2/local.cjs /tmp/pm-audit/e2-local-traced
python3 dev/experiments/e2/verify.py /tmp/pm-audit/e2-local-traced dev/experiments/e2/results/traced

RPC_PORT=19955 LOG=/tmp/pm-audit/e2-node-release-untraced-final.log \
  python3 dev/experiments/e1/run-node.py /tmp/pm-audit/e1/people-untraced.json -- \
  node dev/experiments/e2/local.cjs /tmp/pm-audit/e2-local-untraced-final
python3 dev/experiments/e2/verify.py /tmp/pm-audit/e2-local-untraced-final dev/experiments/e2/results/untraced

E2_RPC_PROGRESS=1 "$E2_BINARY" https://westend-asset-hub-rpc.polkadot.io head \
  /tmp/pm-audit/e2-public-westend 5
"$E2_BINARY" --offline /tmp/pm-audit/e2-public-westend
"$E2_BINARY" --offline /tmp/pm-audit/e2-local-traced/batch
"$E2_BINARY" --offline /tmp/pm-audit/e2-local-traced/batch --tamper-root
```

Pass a full block hash instead of `head` to target a particular block. Re-fetching the recorded public hash later may require an archive endpoint; recent-state retention still applies. For a cold Wasm compilation measurement, use an empty `/tmp/pm-audit/e2-target/wasm-cache` before the run; do not compare that invocation with the reported warm samples.

The E1 README documents rebuilding the existing binaries, specs and npm dependencies if `/tmp/pm-audit/e1` is unavailable. The local harness waits for block 1 before recording its hash; an initial untraced harness attempt queried too early and got the API's zero hash. That was corrected and the complete untraced dataset was rerun on a fresh node.

Per-block scratch outputs contain `block.json`, parent header, runtime version, `proof.json`, RPC measurements, discovery events, warm events/spans, storage durations, oracle, on-chain success events and summary. Large complete proof/trace outputs remain under `/tmp/pm-audit/e2-local-traced`, `/tmp/pm-audit/e2-local-untraced-final` and `/tmp/pm-audit/e2-public-westend`; only compact evidence is kept in this experiment directory. `--offline` needs those scratch files. Original exploratory captures are in `e2-local-probe` and `e2-public-probe`.

SDK dependency versions are pinned to the stable2606-2 family and locked: `sc-executor 0.50.0`, `sp-core 43.0.0`, `sp-io / sp-runtime 48.0.0`, `sp-state-machine 0.53.0`, `sp-trie 46.0.0`, `sp-externalities 0.34.0`, proof-size hostfunction `0.20.0`, statement-store `28.0.0`, EC utils `0.23.0`. Runtime-interface/proc-macro lock versions are `37.0.0 / 21.0.0`; frame-support is stable's `48.0.0`. Provenance and binary/Wasm hashes are in [results/provenance.json](results/provenance.json).

## Limits and recommendation

**Use self-replay as the primary path**, with successful root verification required for a trace labelled faithful. It removes the unsafe-RPC dependency, preserves keyless events, gives actual local timings, and reconstructs useful trees when the runtime supports them. Keep initialize/apply/finalize as an explicitly unverified diagnostic mode if it is added later, rather than silently weakening a failed root check.

Before production:

- **Remote child-trie proof fetching is not implemented.** Host capture and child iteration/removal work on the complete trie in the conformance test, but a missing remote child node cannot be obtained by top-level `state_getReadProof`. The precise extension is namespace-aware child proof fetching using the SDK's additional safe `state_getChildReadProof` method, authenticated through the parent child-root entry. Neither the People cases nor this public block exercised remote child storage. Do not describe this prototype as universal child-state replay.
- **Proof-size replay is validated here, not universally guaranteed.** Some authoring paths may have different recorder history, transactions or host observations; cumulus has node-local recorded estimates for that reason. If fresh-recorder execution differs, root verification must fail rather than accepting a plausible trace. Broader production blocks and differing proof/transaction paths need coverage.
- **Runtime upgrades and alternative chains need more work.** This experiment selects the parent's `:code`, a u32 block number and Blake2 trie/header hashing. Pending-code activation paths, alternate header/hash types, unrecognized future host imports, runtime upgrade blocks and offchain extensions were not tested. Unknown imports are rejected; signatures are not mocked.
- **Public acquisition is still expensive.** Requests are serial and the cache is scoped to a parent/root; proof deduplication, batching complete discovered keys, persistent caching, rate-limit backoff and WS transport are follow-ups. Warm zero-RPC timings must not be sold as end-to-end public RPC latency.
- **No runtime tracing means no runtime span tree.** Native storage timing still works. Export only the timing/nesting actually observed, and distinguish backend iteration attempts from final overlay-aware host results.

The central root question is resolved by working code: safe-RPC proofs plus the SDK executor reproduce both signed People state transitions and a public parachain block without disabling the real root assertion. The remaining gaps are coverage and operational hardening, with remote child proof loading an explicit missing capability.
