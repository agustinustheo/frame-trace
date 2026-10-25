# Status

Last updated 25 October 2026.

## What works

frame-trace replays a Polkadot SDK block, or a runtime API call, outside the node and shows every storage access as an OpenTelemetry trace.

- **Any node, safe RPC only.** It fetches the parent state as authenticated trie proofs (`state_getReadProof`) and runs the block's real `Core_execute_block` with the Polkadot SDK executor. The storage-root check must pass, so a trace is a faithful copy of what the chain did. Tested on public Westend Asset Hub and Paseo People Next, and a local individuality People chain.
- **Every storage op, in order, timed.** Reads, writes, prefix deletions, child tries and iteration, including the events the node's own `state_traceBlock` drops.
- **Where each read came from.** Overlay (cache in the block), warm trie read, or cold trie read, and how many proof bytes each op added. Proof bytes add up exactly to the block's execution proof.
- **Readable.** Keys decode to `System::Account(5Grw…)`, extrinsics to `Balances.transfer_keep_alive`, hooks to `on_initialize: Balances`. Metadata is computed locally from the verified runtime, not trusted from the node.
- **Runtime spans** (dispatch, `batch` → `transfer_keep_alive`, hooks) when the runtime was built with `with-tracing`. For a live chain, build the same runtime source with tracing and replay real blocks with `--runtime-override`: the root check still passes, and the output is labelled as an override everywhere. See [See runtime calls inside extrinsics](../docs/guides/see-runtime-calls.md).
- **Commands:** `block`, `call`, `summary`, `report` (many blocks), `follow` (each finalized block). Traces go to Jaeger/Tempo over OTLP, or to JSON. Replays can be saved and re-run offline.

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
| E1 (`experiments/e1`) | `state_traceBlock` is not enough: unsafe RPC only, drops keyless events, no timing or nesting, and omni-node dev mode can't replay signed blocks. Two upstream bug reports drafted in `upstream/` (not filed). |
| E2 (`experiments/e2`) | Self-replay from safe RPC works, with the real root check. Became the main path. |
| Code | 5 crates, 165 tests, seven review rounds (GPT 6.1 Sol High + Haiku 5.5 High), each verified by Opus 5.5 Medium. |
| First findings (`research/05-first-findings.md`) | 65 blocks traced. Most storage work is block initialization and finalization, not user extrinsics. `System::BlockWeight` is touched 100–143 times per block. In a 3-transfer batch, Alice's account is read 48 times: 42 from the overlay, 5 warm, 1 cold. |

## Limits

- Replay timings are measured on the tracing machine, with tracing overhead. They show where time goes, not node database latency. Benchmarks stay the source of truth for weights.
- `follow` replays about 6–9 blocks a minute on a public endpoint, slower than Asset Hub produces them, so it samples near the tip.
- The host functions are fixed at build time. Chains with extra host functions need them added.
- Plain `#[pallet::constant]` values are compiled into the runtime and never touch storage, so they can't be traced yet.

## Decisions for you

1. **Upstream bug reports.** `upstream/omni-node-dev-replay.md` and `upstream/trace-block-drops-keyless-events.md` are ready to file on paritytech/polkadot-sdk. File them?
2. **Make the repo public?** It is private now.
3. **Constants (spec phase 4).** Tracing constants needs either an opt-in macro in pallets or an instrumented runtime build. Worth designing now?
4. **PolkaMeter integration (spec phase 5).** Attach a block trace to each load-test step. This changes the PolkaMeter repo.
5. **Docs.** Move the README and `docs/` to an mdBook site like PolkaMeter's?
