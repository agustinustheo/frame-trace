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

[0.1.0]: https://github.com/agustinustheo/frame-trace/releases/tag/v0.1.0
