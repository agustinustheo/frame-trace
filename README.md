# frame-trace

OpenTelemetry traces for Polkadot SDK chains: see which storage and constants each extrinsic and state query touches.

See [SPEC.md](SPEC.md) for the design.

## Build and run

The toolchain is pinned to Rust 1.93.0 in `rust-toolchain.toml`.

```sh
cargo build --release
cargo test --workspace
```

Today frame-trace reads a recorded `state_traceBlock` response plus the runtime metadata of
that block (`--rpc` is planned). Metadata can be SCALE bytes or the hex that
`state_getMetadata` returns.

```sh
# Tables: storage per phase, busiest items, duplicate reads
target/release/frame-trace summary \
  --from-file fixtures/model/two-extrinsics.json \
  --metadata fixtures/metadata/westend-v16.scale

# Export the block as one trace to an OTLP/HTTP endpoint, and write the decoded trace as JSON
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest
target/release/frame-trace block \
  --from-file fixtures/model/two-extrinsics.json \
  --metadata fixtures/metadata/westend-v16.scale \
  --otlp http://127.0.0.1:4318 --json block.json
```

Open Jaeger at http://127.0.0.1:16686 and look for service `frame-trace`. Op timestamps are
synthetic (evenly spaced, marked `frame.timing = "synthetic"`): the order is real, the widths
are not.

Crates: `model` (trace parsing, phases, stats), `decode` (metadata-based names), `otel`
(spans and OTLP export), `cli` (the `frame-trace` binary).
