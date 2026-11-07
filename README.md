<img src="https://raw.githubusercontent.com/agustinustheo/polkadot-frame-trace/main/docs/images/logo.svg" alt="polkadot-frame-trace logo: pink petals in front of trace timeline bars" width="112">

# polkadot-frame-trace

Replay any block of a Polkadot SDK chain on your machine and see every storage read and write as a trace.

[![CI](https://img.shields.io/github/actions/workflow/status/agustinustheo/polkadot-frame-trace/ci.yml?branch=main&label=CI)](https://github.com/agustinustheo/polkadot-frame-trace/actions/workflows/ci.yml?query=branch%3Amain) [![Nightly](https://img.shields.io/github/actions/workflow/status/agustinustheo/polkadot-frame-trace/nightly.yml?branch=main&label=nightly)](https://github.com/agustinustheo/polkadot-frame-trace/actions/workflows/nightly.yml) [![Latest release](https://img.shields.io/github/v/release/agustinustheo/polkadot-frame-trace?label=release)](https://github.com/agustinustheo/polkadot-frame-trace/releases/latest) [![crates.io](https://img.shields.io/crates/v/polkadot-frame-trace)](https://crates.io/crates/polkadot-frame-trace) [![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/LICENSE) [![Docs](https://img.shields.io/badge/docs-mdBook-E6007A)](https://agustinustheodorus.com/polkadot-frame-trace/) [![GitHub stars](https://img.shields.io/github/stars/agustinustheo/polkadot-frame-trace?style=flat)](https://github.com/agustinustheo/polkadot-frame-trace) [![Contributors](https://img.shields.io/github/contributors/agustinustheo/polkadot-frame-trace)](https://github.com/agustinustheo/polkadot-frame-trace/graphs/contributors)

![A Utility.batch block in Jaeger. Callouts mark the block, a phase, the runtime calls, a storage read, its origin, its proof bytes and its duration.](https://raw.githubusercontent.com/agustinustheo/polkadot-frame-trace/main/docs/images/hero.png)

*Block #4 of a People chain in Jaeger: (1) the block, (2) the `Utility.batch` extrinsic, (3) the runtime calls inside it, (4) a read of Bob's account, (5) where it was answered, (6) the bytes it added to the proof, (7) its duration.*

## What you get

- **Every storage read and write of a block**, in order, with decoded names such as `System::Account(5Grw…)`.
- **Why each read cost what it did**: answered from the block's own [cache](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html#cache) or from the [trie](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html#trie), and how many bytes it added to the block's proof.
- **The busiest storage items, repeated reads and costs per extrinsic**, for one block or a range of thousands.
- **A trace you can trust.** polkadot-frame-trace checks every value it fetches against the chain's [state root](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html#state-root). After the replay, the root check confirms your machine reached the same state as the chain.

It works with any node over normal RPC, with no unsafe methods or special node build.

## Five-minute start

Install polkadot-frame-trace from the [latest release](https://github.com/agustinustheo/polkadot-frame-trace/releases/latest). On Linux x86_64, with the newest version in `VERSION`:

```sh
VERSION=0.1.0
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf "polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
mkdir -p ~/.local/bin && cp "polkadot-frame-trace-$VERSION-$TARGET/polkadot-frame-trace" ~/.local/bin/
```

Only `sha256sum` prints a line, which ends in `OK` when the archive is intact.

[Install](https://agustinustheodorus.com/polkadot-frame-trace/getting-started/install.html) has the commands for ARM64 Linux, macOS and nightly builds, and how to add `~/.local/bin` to your `PATH`.

Or build it from source with Rust. The first build takes several minutes.

```sh
git clone https://github.com/agustinustheo/polkadot-frame-trace
cd polkadot-frame-trace
cargo install --locked --path crates/cli
```

Start Jaeger:

```sh
docker run -d --rm --name jaeger -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest
```

Trace the latest finalized [Westend Asset Hub](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html#westend-asset-hub) block and send it to Jaeger:

```sh
polkadot-frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block finalized --otlp
```

You should see this. Your block number and sizes will differ.

```text
replay #18598757: fetching state (shared limit: 8 concurrent RPC requests)
replay #18598757: loading runtime and preparing state
root check: PASS; RPC calls: 93 (timed replay: 0); execution proof: 90712 bytes
fetching state #18598757: 64 proof request rounds; 93 RPC calls
state root: computed 0x4f7fcffe3f0f1aa008fb3b26fd6130215382ff42372400e0dec39c78476164ba; header 0x4f7fcffe3f0f1aa008fb3b26fd6130215382ff42372400e0dec39c78476164ba
exported 555 spans to http://127.0.0.1:4318
```

`root check: PASS` means the replay produced the same state root as the block header.
So the trace shows what the block really did.

Open <http://127.0.0.1:16686>, pick the service `polkadot-frame-trace` and click **Find Traces**.
Open the trace named `polkadot-frame-trace: block #<number> (0x…)`.
You will see the block's [phases](https://agustinustheodorus.com/polkadot-frame-trace/concepts/phases.html): initialization, the required extrinsics, post inherents and finalization (this block has only inherents, so post inherents is its own phase).
Each phase holds its storage reads and writes.
Click any read to see where it was answered and its proof bytes.

## Learn more

The [documentation](https://agustinustheodorus.com/polkadot-frame-trace/) has a [glossary](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html) and:

- **Getting started:** [install](https://agustinustheodorus.com/polkadot-frame-trace/getting-started/install.html), [your first trace](https://agustinustheodorus.com/polkadot-frame-trace/getting-started/first-trace.html), [reading a trace](https://agustinustheodorus.com/polkadot-frame-trace/getting-started/reading-a-trace.html).
- **Guides** named after the job: [find repeated reads](https://agustinustheodorus.com/polkadot-frame-trace/guides/find-repeated-reads.html), [measure proof size](https://agustinustheodorus.com/polkadot-frame-trace/guides/measure-proof-size.html), [trace a runtime API call](https://agustinustheodorus.com/polkadot-frame-trace/guides/trace-a-runtime-api-call.html), [analyse many blocks](https://agustinustheodorus.com/polkadot-frame-trace/guides/analyse-many-blocks.html) and [more](https://agustinustheodorus.com/polkadot-frame-trace/).
- **Grafana:** [send traces to Grafana](https://agustinustheodorus.com/polkadot-frame-trace/guides/send-traces-to-grafana.html) for a ready dashboard, [find storage problems with TraceQL](https://agustinustheodorus.com/polkadot-frame-trace/guides/query-traces-in-grafana.html), or [send them through Grafana Alloy to Tempo](https://agustinustheodorus.com/polkadot-frame-trace/guides/send-traces-through-alloy.html). Ready setups are in [`examples/`](https://github.com/agustinustheo/polkadot-frame-trace/tree/main/examples).
- **Concepts:** [what a replay proves](https://agustinustheodorus.com/polkadot-frame-trace/concepts/what-a-replay-proves.html), [phases](https://agustinustheodorus.com/polkadot-frame-trace/concepts/phases.html), [cache or trie](https://agustinustheodorus.com/polkadot-frame-trace/concepts/cache-or-trie.html), [proof bytes](https://agustinustheodorus.com/polkadot-frame-trace/concepts/proof-bytes.html), [timings](https://agustinustheodorus.com/polkadot-frame-trace/concepts/timings.html).
- **Reference:** [commands and flags](https://agustinustheodorus.com/polkadot-frame-trace/reference/commands.html), [span attributes](https://agustinustheodorus.com/polkadot-frame-trace/reference/spans.html), [JSON output](https://agustinustheodorus.com/polkadot-frame-trace/reference/json.html).
- **[Troubleshooting](https://agustinustheodorus.com/polkadot-frame-trace/troubleshooting.html)**, one section per error message.

## Requirements

- Linux (x86_64 or ARM64, glibc 2.35 or newer) or macOS for the release binaries. Building from source needs Rust 1.93.0; `rust-toolchain.toml` pins it and `rustup` installs it.
- The RPC URL of a node for the chain. For old blocks, the node must keep their state, so use an archive node.
- A Polkadot SDK chain with Blake2 headers and `u32` block numbers. Decoded names need metadata V14, V15 or V16.
- Docker, or any trace viewer that accepts OTLP over HTTP. JSON output needs neither.

[What polkadot-frame-trace supports](https://agustinustheodorus.com/polkadot-frame-trace/reference/support.html) lists the chain requirements, the RPC methods polkadot-frame-trace sends, the host functions and the tested chains.

## Contributing

Internal notes, research and design documents are in [`dev/`](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/dev/README.md).
Run `cargo test --workspace` before sending a change.
Docs follow [docs/STYLE.md](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/docs/STYLE.md). Build them with `mdbook build`, using `mdbook` 0.4.52 and `mdbook-mermaid` 0.16.0.

## License

Apache-2.0. See [LICENSE](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/LICENSE).
