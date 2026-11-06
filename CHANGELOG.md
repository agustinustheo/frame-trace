# Changelog

All notable changes to frame-trace are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
frame-trace is at 0.x: any release can change the command line or the JSON output, and those changes are listed here.

## [0.1.0] - Unreleased

First release.

### Added

- `block`: replay one block from a node that serves `state_getReadProof` and still has the block's parent state. With `--otlp` or `--json`, it exports every storage read and write, in order, with decoded names. Without them, it prints totals.
- `call`: replay a runtime API call and show its storage use and result. `--check-state-call` compares the result with the node's.
- `summary`: storage use per phase and per storage item, the heaviest items, and repeated reads.
- `report`: compare storage use across saved blocks or a range of up to 10,000 finalized blocks.
- `follow`: trace each new finalized block as it arrives, or each new best block with `--best`.
- Root check: every block replay confirms it reached the same state root as the chain.
- For each read: whether it came from the cache or the trie, and the bytes it added to the block's proof.
- Export to any trace viewer over OTLP/HTTP, or to JSON.
- `--save` and `--offline`: save a block or call, then replay it with no network.
- `--runtime-override`: replay real blocks with a runtime built with tracing, to see runtime calls inside extrinsics.
- Ready setups for Jaeger, Grafana and Grafana Alloy with Tempo, with a provisioned dashboard.
- `--rpc-timeout`: the longest wait for one RPC response, for slow nodes. Each request gets four tries.
- `follow` can run unattended. It stops cleanly on `SIGTERM` and `SIGHUP`, as on Ctrl-C. It waits longer after each failed check for new blocks. When the OTLP receiver is down, it keeps the traces and later resends only the requests the receiver did not accept.
- A runtime that needs host functions frame-trace does not have fails with an error that names them.
- Tested on relay chains and on blocks that upgrade the runtime, see [What frame-trace supports](https://agustinustheodorus.com/frame-trace/reference/support.html).

### Changed

- For anyone running builds from `main`: the disk cache of `--cache-dir` keeps downloaded state in a few files of up to 64 MiB, not one file per piece. `--cache-max-bytes` now counts bytes used on disk, by state and compiled runtimes together. Runtimes in use plus the space kept for state, files frame-trace cannot delete, runtimes compiled since the last check, the old `nodes` folder until it is deleted, and another frame-trace on the same directory can take it over the limit; see [Disk cache](https://agustinustheodorus.com/frame-trace/reference/commands.html#disk-cache). A long `follow` no longer slows down as the disk cache grows, and starts at once with a full one. The files the old version wrote in `nodes` are deleted in the background.
- For anyone running builds from `main`: `follow` prints its status line when a problem count changes, and otherwise at a fixed interval. The status and summary lines have new fields: `done` is replaced by `traced` and `exported`, and there are export, `poll-failures` and `up` counts.

[0.1.0]: https://github.com/agustinustheo/frame-trace/releases/tag/v0.1.0
