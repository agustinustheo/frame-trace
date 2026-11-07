# Install

You need:

- Docker, to run Jaeger. Any other trace viewer that accepts [OTLP](../reference/glossary.md#otlp) over HTTP also works.
- The RPC URL of a node of the chain you want to trace. It must serve `state_getReadProof`. See [Choose an endpoint](../guides/trace-a-block.md#choose-an-endpoint) before tracing many blocks.

[What polkadot-frame-trace supports](../reference/support.md) lists the chains and nodes polkadot-frame-trace works with.

There are three ways to install polkadot-frame-trace:

1. [A release binary](#install-a-release-binary), for most people.
2. [A nightly binary](#install-a-nightly-binary), built from the newest code each night.
3. [From source](#build-from-source), which needs Rust.

## Install a release binary

Each release has an archive for these systems:

| System | Target in the file name |
| --- | --- |
| Linux, x86_64 | `x86_64-unknown-linux-gnu` |
| Linux, ARM64 | `aarch64-unknown-linux-gnu` |
| macOS, Apple silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |

The Linux binaries need glibc 2.35 or newer, as in Ubuntu 22.04.

Find the newest version on the [releases page](https://github.com/agustinustheo/polkadot-frame-trace/releases/latest).
Run these commands in a terminal, with that version and the target for your system:

```sh
VERSION=0.1.0
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/SHA256SUMS"
```

These commands print nothing when the download works.
If the version or target is wrong, `curl` stops with an error such as:

```text
curl: (56) The requested URL returned error: 404
```

On Linux x86_64 or ARM64, check the archive against `SHA256SUMS`:

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

On macOS, use `shasum` for the same check:

```sh
shasum -a 256 --check --ignore-missing SHA256SUMS
```

You should see `OK` after the archive's name. On macOS with Apple silicon:

```text
polkadot-frame-trace-0.1.0-aarch64-apple-darwin.tar.gz: OK
```

Unpack the archive and copy `polkadot-frame-trace` to `~/.local/bin`:

```sh
tar -xzf "polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
mkdir -p ~/.local/bin
cp "polkadot-frame-trace-$VERSION-$TARGET/polkadot-frame-trace" ~/.local/bin/
```

These commands print nothing.
If `~/.local/bin` is not on your `PATH`, add it:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Put that line in your shell's startup file, such as `~/.bashrc` or `~/.zshrc`, to keep it.
Then check that your shell finds polkadot-frame-trace:

```sh
polkadot-frame-trace --version
```

```text
polkadot-frame-trace 0.1.0
```

## Install a nightly binary

The [Nightly](https://github.com/agustinustheo/polkadot-frame-trace/releases/tag/nightly) pre-release holds binaries built from the newest code.
It is rebuilt each night that the code changed and every check passed.

> **Warning:** Nightly builds are untested between releases. They can change the command line or the JSON output without notice. Use a release binary unless you need a change that is not released yet.

The archives have `nightly` in place of the version.
Follow the steps for a release binary with these downloads:

```sh
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/nightly/polkadot-frame-trace-nightly-$TARGET.tar.gz"
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/nightly/SHA256SUMS"
```

Check the archive as above, then unpack it and copy the binary. These commands print nothing.

```sh
tar -xzf "polkadot-frame-trace-nightly-$TARGET.tar.gz"
mkdir -p ~/.local/bin
cp "polkadot-frame-trace-nightly-$TARGET/polkadot-frame-trace" ~/.local/bin/
```

A nightly binary prints the date and commit it was built from.
For example, a build from 6 November 2026 prints `0.1.0-nightly.20261106+` and the first seven characters of its commit:

```sh
polkadot-frame-trace --version
```

```text
polkadot-frame-trace 0.1.0-nightly.<date>+<commit>
```

## Build from source

You need Rust 1.93.0 or newer. If you do not have it, install it with [rustup](https://rustup.rs).
The first build compiles the Polkadot SDK executor, so it takes several minutes.

Build and install the newest code with Cargo:

```sh
cargo install --locked --git https://github.com/agustinustheo/polkadot-frame-trace polkadot-frame-trace
```

```text
    Updating git repository `https://github.com/agustinustheo/polkadot-frame-trace`
...
   Installed package `polkadot-frame-trace v0.1.0 (https://github.com/agustinustheo/polkadot-frame-trace#...)` (executable `polkadot-frame-trace`)
```

Add `--tag v<version>` to build a release instead.

Or clone the repository and install from it.
It has the saved blocks the next step replays.
The repository pins Rust 1.93.0 in `rust-toolchain.toml`, and `rustup` installs it for you.

```sh
git clone https://github.com/agustinustheo/polkadot-frame-trace
cd polkadot-frame-trace
cargo install --locked --path crates/cli
```

```text
Cloning into 'polkadot-frame-trace'...
...
   Installed package `polkadot-frame-trace v0.1.0 (.../polkadot-frame-trace/crates/cli)` (executable `polkadot-frame-trace`)
```

Cargo puts the binary in `~/.cargo/bin`.

## Check it works

```sh
polkadot-frame-trace --version
```

```text
polkadot-frame-trace 0.1.0
```

Then replay a block saved in the repository, from a local [People chain](../reference/glossary.md#people-chain).
It needs no network.
If you installed a binary, clone the repository first with `git clone https://github.com/agustinustheo/polkadot-frame-trace`.
Run the replay from the `polkadot-frame-trace` directory:

```sh
polkadot-frame-trace block --offline fixtures/replay/people-transfer
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
