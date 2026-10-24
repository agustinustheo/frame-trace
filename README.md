# frame-trace

Replay a Polkadot SDK block and view its storage accesses in Jaeger or Tempo.
Self-replay uses safe RPC and authenticated parent-state proofs. The SDK executes
`Core_execute_block`, including signatures and the real storage-root assertion.

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

Each replay prints the root-check result and RPC count. Discovery fetches proofs.
The measured run uses a warm cache and zero RPC. A missing proof is an error.
Offline replay never creates an RPC client. HTTP(S) endpoints also work.

`follow` polls best headers every two seconds. It walks new ancestry, including
best-chain reorgs. Two workers and an eight-block queue bound the work. Errors and
full-queue skips are reported. It keeps 256 recent hashes and catches up at most
64 ancestors per poll. Press Ctrl-C to stop. Best blocks can later be reverted.

Times are measured on the replay machine, including tracing overhead. They are
not the original node's database latency or benchmark weights. Runtime spans
appear when the Wasm has `with-tracing`. Without them, ops sit under phase spans.
Backend iteration events describe attempts; the overlay may change the result.
Traced `apply_extrinsic` spans include marginal execution-proof bytes. These are
not standalone extrinsic proof sizes.

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
