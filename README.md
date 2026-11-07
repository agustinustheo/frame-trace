<p align="center">
  <img src="https://raw.githubusercontent.com/agustinustheo/polkadot-frame-trace/main/docs/images/logo.svg" alt="polkadot-frame-trace logo: pink petals in front of trace timeline bars" width="160">
</p>

<h1 align="center">polkadot-frame-trace</h1>

<p align="center">
  <b>Replay any Polkadot SDK block on your machine and see every storage read and write as a trace.</b>
</p>

<p align="center">
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/actions/workflows/ci.yml?query=branch%3Amain"><img alt="Build" src="https://img.shields.io/github/actions/workflow/status/agustinustheo/polkadot-frame-trace/ci.yml?branch=main&label=build&logo=githubactions&logoColor=white"></a>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/actions/workflows/nightly.yml"><img alt="Nightly" src="https://img.shields.io/github/actions/workflow/status/agustinustheo/polkadot-frame-trace/nightly.yml?branch=main&label=nightly&logo=githubactions&logoColor=white"></a>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/agustinustheo/polkadot-frame-trace?logo=github&color=E6007A"></a>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/blob/main/LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-B0005E"></a>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/agustinustheo/polkadot-frame-trace/total?color=FF6EB4"></a>
  <br>
  <img alt="Rust 1.93" src="https://img.shields.io/badge/rust-1.93-E6007A?logo=rust&logoColor=white">
  <img alt="Polkadot SDK stable2609" src="https://img.shields.io/badge/Polkadot%20SDK-stable2609-E6007A?logo=polkadot&logoColor=white">
  <img alt="Linux x86-64 and arm64" src="https://img.shields.io/badge/platform-linux%20x86--64%20%7C%20arm64-555?logo=linux&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/platform-macOS-555?logo=apple&logoColor=white">
  <img alt="No node changes" src="https://img.shields.io/badge/node%20changes-none-B0005E">
  <br>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace"><img alt="Stars" src="https://img.shields.io/github/stars/agustinustheo/polkadot-frame-trace?style=flat&logo=github"></a>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/graphs/contributors"><img alt="Contributors" src="https://img.shields.io/github/contributors/agustinustheo/polkadot-frame-trace"></a>
  <a href="https://github.com/agustinustheo/polkadot-frame-trace/commits/main"><img alt="Last commit" src="https://img.shields.io/github/last-commit/agustinustheo/polkadot-frame-trace"></a>
</p>

---

polkadot-frame-trace runs a block again on your machine, with the chain's own runtime and state from any node's normal RPC.
It records every storage read and write and checks that the replay reached the block's state root.
It exports the result as an OpenTelemetry trace for Jaeger or Grafana Tempo, or as JSON.

It is for runtime and pallet developers and chain teams who want to know why a block costs what it does.
Find the storage items a block reads most, the reads it repeats, and the bytes each one adds to the proof.

![A Utility.batch block in Jaeger. Callouts mark the block, a phase, the runtime calls, a storage read, its origin, its proof bytes and its duration.](https://raw.githubusercontent.com/agustinustheo/polkadot-frame-trace/main/docs/images/hero.png)

*Block #4 of a People chain in Jaeger. (1) The block, (2) the `Utility.batch` extrinsic and (3) the runtime calls inside it. (4) A read of Bob's account, (5) where it was answered, (6) the bytes it added to the proof and (7) its duration.*

## Highlights

- Any node, no changes: safe RPC only, with the parent state as proofs from `state_getReadProof`.
- Every value is checked against the parent's [state root](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html#state-root), and the [root check](https://agustinustheodorus.com/polkadot-frame-trace/concepts/what-a-replay-proves.html) must pass before anything is exported.
- The chain's own runtime runs in the Polkadot SDK executor, with signature checks on.
- Every read and write, in order, with decoded names such as `System::Account(5Grw…)`.
- For each read: whether the [cache or the trie](https://agustinustheodorus.com/polkadot-frame-trace/concepts/cache-or-trie.html) answered it, and the [proof bytes](https://agustinustheodorus.com/polkadot-frame-trace/concepts/proof-bytes.html) it added.
- Spans per [phase](https://agustinustheodorus.com/polkadot-frame-trace/concepts/phases.html) and extrinsic, and per runtime call when the runtime is built with [`with-tracing`](https://agustinustheodorus.com/polkadot-frame-trace/guides/see-runtime-calls.html).

## Install

Download the archive for your system from the [latest release](https://github.com/agustinustheo/polkadot-frame-trace/releases/latest). On Linux x86_64:

```sh
VERSION=0.1.0
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf "polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
mkdir -p ~/.local/bin && cp "polkadot-frame-trace-$VERSION-$TARGET/polkadot-frame-trace" ~/.local/bin/
```

Other targets are `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-apple-darwin`. On macOS, check with `shasum -a 256`.
The Linux binaries need glibc 2.35 or newer.
[Install](https://agustinustheodorus.com/polkadot-frame-trace/getting-started/install.html) covers nightly builds, your `PATH` and building from source.

## Tour

Start Jaeger first: `docker run -d --rm --name jaeger -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest`.

```sh
RPC=wss://westend-asset-hub-rpc.polkadot.io
polkadot-frame-trace block --rpc $RPC --block finalized --otlp       # one block, as a trace in Jaeger
polkadot-frame-trace summary --rpc $RPC --block finalized            # a block as tables         
polkadot-frame-trace block --rpc $RPC --block finalized \
  --save saved-block --json block.json                               # keep it to replay offline
polkadot-frame-trace block --offline saved-block --otlp              # replay it with no network
polkadot-frame-trace report --rpc $RPC --last 10                     # the last 10 finalized blocks in one report
polkadot-frame-trace follow --rpc $RPC --otlp                        # trace each new finalized block
polkadot-frame-trace call --rpc $RPC --method Core_version --data 0x \
  --check-state-call                                                 # trace a runtime API call
```

Open <http://127.0.0.1:16686>, pick the service `polkadot-frame-trace` and click **Find Traces**.
[Commands and flags](https://agustinustheodorus.com/polkadot-frame-trace/reference/commands.html) lists everything else.

### A replay, as you see it

Block #4 of the local People chain is saved in this repository. From a clone, replay it with no network:

```sh
polkadot-frame-trace summary --offline fixtures/replay/people-traced
```

```text
replay #4: loading runtime and preparing state
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 6586 bytes
fetching state #4: 0 proof request rounds; 0 RPC calls
state root: computed 0x47116eac39ac59ba0534a0e4d624f78c4c8c56af5c3084e71c98ca1811e6e4f1; header 0x47116eac39ac59ba0534a0e4d624f78c4c8c56af5c3084e71c98ca1811e6e4f1
block 0x0661899538bf75f600c05c9cc31a01d34e95be6ea6096230a2af6a0074d90637 (parent 0xaa9b6d9f7f0d74a603f7d763724d812f468139427ba81f5f22dc82a7d4026de6)

Storage per phase
┌──────────────────────────────────────────────────┬────────────┬───────┬────────┬─────────────┬─────────────────┬──────────────┐
│ phase                                            ┆ operations ┆ reads ┆ writes ┆ proof bytes ┆ storage time µs ┆ wall time µs │
╞══════════════════════════════════════════════════╪════════════╪═══════╪════════╪═════════════╪═════════════════╪══════════════╡
│ initialization                                   ┆        152 ┆    74 ┆     77 ┆        3490 ┆           277.0 ┆        486.4 │
│ extrinsic 0: ParachainSystem.set_validation_data ┆         54 ┆    26 ┆     28 ┆         495 ┆           149.5 ┆        274.2 │
│ extrinsic 1: Timestamp.set                       ┆         26 ┆    14 ┆     12 ┆         139 ┆            38.3 ┆         54.4 │
│ extrinsic 2: Utility.batch                       ┆        200 ┆   146 ┆     54 ┆         827 ┆           532.2 ┆        774.0 │
│   └ post inherents (on_poll)                     ┆         14 ┆    12 ┆      2 ┆         686 ┆            32.4 ┆         38.9 │
│ finalization                                     ┆         72 ┆    40 ┆     31 ┆         949 ┆           271.7 ┆        452.1 │
│ total                                            ┆        518 ┆   312 ┆    204 ┆        6586 ┆          1301.1 ┆       4811.4 │
└──────────────────────────────────────────────────┴────────────┴───────┴────────┴─────────────┴─────────────────┴──────────────┘

1 block(s); 518 operations; reads 312 = cache 231 + trie 81 (already in proof 47, new 34) + unknown 0
Duplicate reads 203 = cache 177, trie (already in proof) 26, trie (new) 0, unknown 0; proof 6586 B (518 operations with proof data; 0 B not assigned to operations)
...
```

`root check: PASS` means the replay reached the same state root as the block header.
Times are measured on your machine, not on the node. The rest of the output ranks storage items, extrinsics and repeated reads.

## Chains

Point `--rpc` at a node of any Polkadot SDK chain with Blake2-256 headers and `u32` block numbers.
For old blocks, use an [archive node](https://agustinustheodorus.com/polkadot-frame-trace/reference/glossary.html#archive-node).
Every block below passed the root check.

| Chain | Live, from a public node | Offline, from a saved block |
| --- | --- | --- |
| Westend Asset Hub | Finalized and best blocks with `block`, `report` and `follow`; nightly | #18,558,320 |
| Westend Asset Hub runtime upgrade | #18,985,169 and #18,985,170, the first on the new code; nightly | |
| Westend relay chain | The finalized block, nightly; #33,614,892 | |
| Paseo Asset Hub | | #15,366,850, a contract call with child trie reads and writes |
| Paseo People Next | 20 blocks with `block` and `summary` | |
| A local People chain | | Saved blocks, including a traced runtime |

[What polkadot-frame-trace supports](https://agustinustheodorus.com/polkadot-frame-trace/reference/support.html) lists the chain requirements, the RPC methods it sends and the host functions.

## Build from source

See [Install](https://agustinustheodorus.com/polkadot-frame-trace/getting-started/install.html#build-from-source).
Docs follow [docs/STYLE.md](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/docs/STYLE.md), and developer notes are in [`dev/`](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/dev/README.md).

## License

Apache-2.0. See [LICENSE](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/LICENSE).

<sub>Polkadot is a trademark of the Web3 Foundation. This project is independent and is not affiliated with or endorsed by the Web3 Foundation.</sub>
