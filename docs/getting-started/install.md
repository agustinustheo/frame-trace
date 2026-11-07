# Install

You need:

- Docker, to run Jaeger. Any other trace viewer that accepts [OTLP](../reference/glossary.md#otlp) over HTTP also works.
- The RPC URL of a node of the chain you want to trace. It must serve `state_getReadProof`. See [Choose an endpoint](../guides/trace-a-block.md#choose-an-endpoint) before tracing many blocks.

[What polkadot-frame-trace supports](../reference/support.md) lists the chains and nodes it works with.
Most people install a release binary. You can also [build from source](#build-from-source).

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
Download it with your version and target:

```sh
VERSION=0.1.0
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/polkadot-frame-trace-$VERSION-$TARGET.tar.gz"
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/v$VERSION/SHA256SUMS"
```

If the version or target is wrong, `curl` stops with an error:

```text
curl: (56) The requested URL returned error: 404
```

Check the archive against `SHA256SUMS`. On macOS, use `shasum -a 256` in place of `sha256sum`:

```sh
sha256sum --check --ignore-missing SHA256SUMS
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

If `~/.local/bin` is not on your `PATH`, add this line to your shell's startup file, such as `~/.bashrc` or `~/.zshrc`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

### Nightly binary

The [Nightly](https://github.com/agustinustheo/polkadot-frame-trace/releases/tag/nightly) pre-release is rebuilt each night that the code changed and every check passed.

> **Warning:** Nightly builds are not tested between releases. They may change the command line or the JSON output without notice.

Follow the same steps with `nightly` in place of the version:

```sh
TARGET=x86_64-unknown-linux-gnu
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/nightly/polkadot-frame-trace-nightly-$TARGET.tar.gz"
curl -fsSLO "https://github.com/agustinustheo/polkadot-frame-trace/releases/download/nightly/SHA256SUMS"
```

Its `--version` shows the build date and the first seven characters of the commit, for example `polkadot-frame-trace 0.1.0-nightly.20261106+<commit>`.

## Build from source

You need Rust 1.93.0 or newer, from [rustup](https://rustup.rs).
The first build takes several minutes. Install the newest code with Cargo:

```sh
cargo install --locked --git https://github.com/agustinustheo/polkadot-frame-trace polkadot-frame-trace
```

```text
    Updating git repository `https://github.com/agustinustheo/polkadot-frame-trace`
...
   Installed package `polkadot-frame-trace v0.1.0 (https://github.com/agustinustheo/polkadot-frame-trace#...)` (executable `polkadot-frame-trace`)
```

Add `--tag v<version>` to build a release instead.

Or clone the repository, which has the saved blocks the next step replays.
`rust-toolchain.toml` pins Rust 1.93.0, and `rustup` installs it.

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

Then replay a block from a local [People chain](../reference/glossary.md#people-chain) that is saved in the repository. It needs no network.
If you installed a binary, clone the repository first with `git clone https://github.com/agustinustheo/polkadot-frame-trace`.
Run this from the `polkadot-frame-trace` directory:

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
