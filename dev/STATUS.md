# Status

Last updated 6 November 2026.

## What works

frame-trace replays a Polkadot SDK block, or a runtime API call, outside the node and shows every storage access as an OpenTelemetry trace.

- **Any node, safe RPC only.** It fetches the parent state as authenticated trie proofs (`state_getReadProof`) and runs the block's real `Core_execute_block` with the Polkadot SDK executor. The storage-root check must pass, so a trace is a faithful copy of what the chain did. Tested on public Westend Asset Hub (including a runtime upgrade), the Westend relay chain, Paseo Asset Hub (a contract call with child trie reads and writes), Paseo People Next and a local individuality People chain. See the [support page](../docs/reference/support.md#tested-chains).
- **Every storage op, in order, timed.** Reads, writes, prefix deletions, child tries and iteration, including the events the node's own `state_traceBlock` drops.
- **Where each read came from.** Overlay (cache in the block), warm trie read, or cold trie read, and how many proof bytes each op added. Proof bytes add up exactly to the block's execution proof.
- **Readable.** Keys decode to `System::Account(5Grw…)`, extrinsics to `Balances.transfer_keep_alive`, hooks to `on_initialize: Balances`. Metadata is computed locally from the verified runtime, not trusted from the node.
- **Runtime spans** (dispatch, `batch` → `transfer_keep_alive`, hooks) when the runtime was built with `with-tracing`. For a live chain, build the same runtime source with tracing and replay real blocks with `--runtime-override`: the root check still passes, and the output is labelled as an override everywhere. See [See runtime calls inside extrinsics](../docs/guides/see-runtime-calls.md).
- **Commands:** `block`, `call`, `summary`, `report` (many blocks), `follow` (each finalized block). Traces go to Jaeger/Tempo over OTLP, or to JSON. Replays can be saved and re-run offline.
- **`follow` runs unattended.** It stops cleanly on SIGTERM, SIGHUP and Ctrl-C, backs off failed polls up to 60 s, and prints status on change with a 60 s heartbeat. While the OTLP receiver is down it holds up to 32 traces and later resends only the batches the receiver did not accept, without replaying blocks. Exit code 0 on a signal stop; failures are counted in the summary. A systemd unit is in the [follow guide](../docs/guides/follow-new-blocks.md#6-run-it-as-a-service).
- **Slow nodes.** `--rpc-timeout` (default 7.5 s) is the wait for one RPC try; each request gets four tries. Dwellir's Paseo Asset Hub endpoint needed `--rpc-timeout 120`: its proof of `:code` took about 35 s.

Try it:

```sh
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest
cargo +1.93.0 build --release --locked
target/release/frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block best --otlp http://127.0.0.1:4318
target/release/frame-trace summary --offline fixtures/replay/people-traced
```

Then open http://127.0.0.1:16686.

## How it was built

| Step | Result |
| --- | --- |
| Research (`research/01`–`04`) | Nobody exports Substrate storage traces to OpenTelemetry. `state_traceBlock` is the closest piece. |
| E1 (`experiments/e1`) | `state_traceBlock` is not enough: unsafe RPC only, drops keyless events, no timing or nesting, and omni-node dev mode can't replay signed blocks. Two upstream bug reports, filed as polkadot-sdk #13594 and #13595 and fixed by #13665 and #13666. |
| E2 (`experiments/e2`) | Self-replay from safe RPC works, with the real root check. Became the main path. |
| Code | 5 crates, 165 tests at the time, seven review rounds (GPT 6.1 Sol High + Haiku 5.5 High), each verified by Opus 5.5 Medium. |
| First findings (`research/05-first-findings.md`) | 65 blocks traced. Most storage work is block initialization and finalization, not user extrinsics. `System::BlockWeight` is touched 100–143 times per block. In a 3-transfer batch, Alice's account is read 48 times: 42 from the overlay, 5 warm, 1 cold. |

## Limits

- Replay timings are measured on the tracing machine, with tracing overhead. They show where time goes, not node database latency. Benchmarks stay the source of truth for weights.
- `follow` uses eight workers and 16 shared RPC slots. Two 120-second release runs completed 49 and 38 consecutive Westend Asset Hub blocks, with no expiry skips; the chain advanced 56 and 46 heights. Their final minutes completed 40 and 37 blocks as startup backlog cleared. Peak RSS was 698–752 MB. Public endpoint load and block contents still affect throughput; slower runs can fill the bounded queue and sample recent heights. See [measurements](performance.md#follow-workers-and-memory-2026-10-26).
- The host functions are fixed at build time. A runtime that imports others fails with an error that names them; adding them needs a new build. See [dev/README.md](README.md#add-host-functions-for-a-chain).
- Plain `#[pallet::constant]` values are compiled into the runtime and never touch storage, so they can't be traced yet.

## Decisions for you

1. **Constants (spec phase 4).** Tracing constants needs either an opt-in macro in pallets or an instrumented runtime build. Worth designing now?
2. **PolkaMeter integration (spec phase 5).** Attach a block trace to each load-test step. This changes the PolkaMeter repo.
3. **First release, v0.1.0.** The release workflow is ready. Change `Unreleased` to the date in `CHANGELOG.md`, push the tag `v0.1.0`, check the draft release and publish it. See [RELEASING.md](RELEASING.md).
4. **Publish to crates.io?** Today users install from a clone. `frame-trace-replay` has `publish = false`, and `frame-trace` depends on it, so publishing needs all five crates published, or the library crates folded into one.

## Upstream

| Bug | Issue | Fix PR | State (6 November 2026) |
| --- | --- | --- | --- |
| `polkadot-omni-node --dev` can't trace blocks with signed extrinsics | [#13594](https://github.com/paritytech/polkadot-sdk/issues/13594) | [#13665](https://github.com/paritytech/polkadot-sdk/pull/13665): omni-node uses the parachain tracing executor in `--dev` mode | Merged 2 November 2026. Issue closed. |
| `state_traceBlock` drops keyless events with an empty filter | [#13595](https://github.com/paritytech/polkadot-sdk/issues/13595) | [#13666](https://github.com/paritytech/polkadot-sdk/pull/13666): sc-tracing keeps events without the filtered field when the filter is empty | Merged 2 November 2026. Issue closed. |

The first fix PRs, #13596 and #13597, were closed without merging.
frame-trace does not depend on either fix: it replays blocks itself from safe RPC.

## Done since 25 October 2026

- The repository is public.
- The docs are an mdBook site, with a [support page](../docs/reference/support.md).
- Both upstream bugs are filed and fixed.
- CI runs rustfmt and clippy on Linux, and the tests on Linux and macOS. A nightly job replays live Westend Asset Hub and Westend relay chain blocks, including the runtime upgrade block. Tagging `v*` builds a draft release for four targets. Dependabot groups the Polkadot SDK crates.
- A fixture and tests for a contract call with child trie reads and writes. A runtime upgrade block, the first block after it and relay chain blocks are covered by live tests.
- `follow` hardened for unattended runs, OTLP retries without duplicates, `--rpc-timeout`.
- On 6 November 2026, `cargo +1.93.0 test --workspace --locked` on the integration of the CI, replay coverage and follow branches passed 208 tests offline and ignored 3 live tests. The 3 live tests also passed against the default public endpoints. The nightly job runs them.
