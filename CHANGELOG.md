# Changelog

All notable changes to polkadot-frame-trace are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
polkadot-frame-trace is at 0.x: any release can change the command line or the JSON output, and those changes are listed here.

## [0.1.0] - Unreleased

First release.

The project is named polkadot-frame-trace: the command, the crates, the OTLP service name and the folder for compiled runtimes. It was called frame-trace before.

### Added

- `block`: replay one block from a node that serves `state_getReadProof` and still has the block's parent state. With `--otlp` or `--json`, it exports every storage read and write, in order, with decoded names. Without them, it prints totals.
- `call`: replay a runtime API call and show its storage use and result. `--check-state-call` compares the result with the node's.
- `summary`: storage use per phase and per storage item, the heaviest items, and repeated reads.
- `report`: compare storage use across saved blocks or a range of up to 10,000 finalized blocks.
- `follow`: trace each new finalized block as it arrives, or each new best block with `--best`.
- Root check: every block replay confirms it reached the same state root as the chain.
- For each read: whether it came from the cache or the trie, and the bytes it added to the block's proof.
- Export to any trace viewer over OTLP/HTTP, or to JSON. Ready setups for Jaeger, Grafana and Grafana Alloy with Tempo, with a provisioned dashboard.
- `--save` and `--offline`: save a block or call, then replay it with no network.
- `--runtime-override`: replay real blocks with a runtime built with tracing, to see runtime calls inside extrinsics.
- `--rpc-timeout`: the longest wait for one RPC response, for slow nodes. Each request gets four tries. The default is 30 seconds, so a request can take up to 2 minutes. Public nodes take 7 to 35 seconds to send the runtime code.
- `follow` can run unattended. It stops cleanly on `SIGTERM` and `SIGHUP`, as on Ctrl-C. It waits longer after each failed check for new blocks. When the OTLP receiver is down, it keeps the traces and later resends only the requests the receiver did not accept.
- A runtime that needs host functions polkadot-frame-trace does not have fails with an error that names them.
- Nightly builds of `main` for the same four systems as releases, in the `Nightly` pre-release on GitHub. `polkadot-frame-trace --version` shows the date and commit they were built from.
- Tested on relay chains and on blocks that upgrade the runtime, see [What polkadot-frame-trace supports](https://agustinustheodorus.com/polkadot-frame-trace/reference/support.html).
- Limits against a node that sends too much: 64 MiB per RPC response and 16 MiB per WebSocket frame. One block may use 1,000,000 pieces of state or 256 MiB.
- A replay deadline: `follow` exits 1 when a replay runs the runtime for 10 minutes, so a supervisor restarts it. Other commands warn once.
- A warning when `--rpc` or `--otlp` is plain `ws://` or `http://` to another machine.
- A private disk cache: polkadot-frame-trace creates the `--cache-dir` directory readable by you alone, and runs without one that another user owns or can write. Without `HOME`, compiled runtimes go to a per-user folder in the temporary directory.
- URL redaction: output and errors show `--rpc` and `--otlp` URLs as scheme, host and port only, so credentials and API keys stay out of logs. Error text from the node or the OTLP receiver is cut to 512 characters.

Nightly users: delete `~/.cache/frame-trace` and `<cache-dir>/nodes`.

[0.1.0]: https://github.com/agustinustheo/polkadot-frame-trace/releases/tag/v0.1.0
