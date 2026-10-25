# Install

You need:

- Rust. The repository pins version 1.93.0 in `rust-toolchain.toml`, and `rustup` installs it for you.
- Docker, to run Jaeger. Any other trace viewer that accepts OTLP over HTTP also works.
- The RPC URL of a node of the chain you want to trace. It must serve `state_getReadProof`. See [Choose an endpoint](../guides/trace-a-block.md#choose-an-endpoint) before tracing many blocks.

## Build and install

Run these commands in a terminal:

```sh
git clone https://github.com/agustinustheo/frame-trace
cd frame-trace
cargo install --locked --path crates/cli
```

The first build compiles the Polkadot SDK executor, so it takes several minutes.
Cargo puts the binary in `~/.cargo/bin`.

## Check it works

```sh
frame-trace --version
```

```text
frame-trace 0.1.0
```

Then replay a block that ships with the repository.
It needs no network.
Run it from the `frame-trace` directory:

```sh
frame-trace block --offline fixtures/replay/people-transfer
```

```text
replay #2: loading runtime and preparing state
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 6479 bytes
fetching state #2: 0 proof request rounds; 0 RPC calls
state root: computed 0xa33960d3b1aadccf9197e0a77aebfea4075457c02800658639eb20fd5c6012c9; header 0xa33960d3b1aadccf9197e0a77aebfea4075457c02800658639eb20fd5c6012c9
block 0xea56e9d68d1664e7998f17d21883a1f4927e67a1cf5c175501bccfa9043f2462: 5 phases, 421 storage operations (242 reads, 177 writes, 134 duplicate reads). Use --otlp or --json to export it.
```

`root check: PASS` means the replay reached the same state as the chain.
[What a replay proves](../concepts/what-a-replay-proves.md) explains each line.

Next: [Your first trace in Jaeger](first-trace.md).
