# Commands and flags

This page lists every command and flag, taken from `polkadot-frame-trace <command> --help`.

| Command | Does |
| --- | --- |
| [`block`](#block) | Trace storage use in one block and export it to JSON or a trace viewer. |
| [`summary`](#summary) | Show storage use by block phase and storage item. |
| [`call`](#call) | Trace storage use and the result of a runtime API call. |
| [`report`](#report) | Compare storage use across saved blocks or a range of finalized blocks. |
| [`follow`](#follow) | Trace new finalized blocks as they arrive. |

## Sources

`block` and `summary` need exactly one source:

| Flag | Source |
| --- | --- |
| `--rpc <URL>` with `--block <NUMBER\|HASH\|best\|finalized>` | Replay a block from a node. |
| `--offline <DIR>` | Replay a block saved with `--save`, with no network. |
| `--from-json <FILE>` | Read saved `block --json` output without replaying. |
| `--from-file <FILE>` | Read a recorded `state_traceBlock` response. No replay, no root check, no origins, no proof bytes, made-up timings. |

## block

```sh
polkadot-frame-trace block --rpc URL --block finalized --otlp
polkadot-frame-trace block --offline fixtures/replay/people-traced --json block.json
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--rpc <URL>` | | Node RPC endpoint: `ws://`, `wss://`, `http://` or `https://`. |
| `--block <NUMBER\|HASH\|best\|finalized>` | | Block to trace with `--rpc`: a block number, a full hash, `best` (newest) or `finalized` (newest finalized). |
| `--offline <DIR>` | | Replay a saved block with no network. |
| `--from-json <FILE>` | | Read saved `block --json` output without replay. |
| `--from-file <FILE>` | | Recorded `state_traceBlock` response, as the JSON-RPC envelope or the bare result. |
| `--metadata <FILE>` | | Metadata for `--from-file`: SCALE bytes (`.scale`), or hex as returned by `state_getMetadata` (`.hex`, or the JSON-RPC response). |
| `--save <DIR>` | | Save the block, its state and runtime for offline replay. |
| `--otlp [<URL>]` | `http://127.0.0.1:4318` when given without a URL | Send the trace over OTLP/HTTP. |
| `--json <FILE>` | | Write the trace as JSON. `-` writes to stdout. |
| `--proof-concurrency <N>` | 8 | RPC requests sent at once, 1 to 64. Each proof request asks for up to 32 keys. |
| `--rpc-timeout <SECONDS>` | 30 | Longest wait for one RPC response, more than 0 and at most 3600 seconds. A request gets four tries, so it can take four times this, 2 minutes by default. Raise it for a node that takes longer to send a large proof. |
| `--cache-dir <DIR>` | | Keep the [disk cache](#disk-cache) in this directory between runs. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | Size limit for the [disk cache](#disk-cache), as bytes used on disk. A runtime in use is kept even above the limit. |
| `--unauthenticated-metadata` | off | If the runtime's own metadata cannot be loaded, use the node's (or the saved) metadata unchecked. Output says so. |
| `--runtime-override <WASM>` | | Run this Wasm instead of the chain's runtime. See [See runtime calls inside extrinsics](../guides/see-runtime-calls.md). Cannot be combined with `--save`. |

Without `--otlp` or `--json`, `block` prints a one-line count of phases and operations.

A flag that does not apply to the source fails with exit code 2.
`--block`, `--proof-concurrency` and `--rpc-timeout` need `--rpc`, and `--metadata` needs `--from-file`.
The cache and metadata flags do not work with `--from-file` or `--from-json`.

Without `--cache-dir`, downloaded state stays in memory only.
Compiled runtimes are still kept in `$XDG_CACHE_HOME/polkadot-frame-trace/wasm`, or `~/.cache/polkadot-frame-trace/wasm`.
Without both `XDG_CACHE_HOME` and `HOME`, they go to `polkadot-frame-trace-<uid>/wasm` in the temporary directory.

These limits protect against a node that sends too much. They apply to every command that uses `--rpc`.

| Limit | Value | Meaning |
| --- | --- | --- |
| One RPC response | 64 MiB | A larger response fails the request at once, without another try. The largest real one, the proof of the runtime, is about 5.5 MB. |
| One WebSocket frame | 16 MiB | A response over `ws://` or `wss://` may come in one frame. A larger frame fails the request at once, without another try. |
| State the node sends for one block | 1,000,000 pieces or 256 MiB | More fails the block. Recent blocks of Polkadot, Kusama, Westend and their Asset Hubs, and a Westend Asset Hub runtime upgrade, needed at most 3,656 pieces and 5.6 MB. |
| One replay | 10 minutes | A runtime that never finishes cannot be stopped from inside polkadot-frame-trace. `block`, `summary`, `call` and `report` warn once after 10 minutes of elapsed replay time, including RPC waits, and keep going: press Ctrl-C to stop them. `follow` exits with code 1 when a replay runs 10 minutes without RPC activity, see [follow](#follow). Real blocks take seconds, and compiling a new runtime about a minute. |

Output shows `--rpc` and `--otlp` URLs as scheme, host and port only.
A user name, password or API key in the URL stays out of logs.
Text from the node or the OTLP receiver in error lines is cut to 512 characters.
A plain `ws://` or `http://` URL to another machine prints one warning, because anyone on the way can read and change what is sent. `localhost` and loopback addresses do not.

## summary

```sh
polkadot-frame-trace summary --rpc URL --block finalized
polkadot-frame-trace summary --from-json block.json
```

`summary` takes the same source flags as `block`, plus `--save`, `--proof-concurrency`, `--rpc-timeout`, `--cache-dir`, `--cache-max-bytes`, `--unauthenticated-metadata` and `--runtime-override`.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--top <N>` | 15 | Rows in each item, extrinsic and hook table, and per phase in the duplicate reads table. |

It prints the tables **Storage per phase**, **Phase shares**, **Top items by proof bytes** and **Extrinsic distributions**.
Then **Hooks**, only with runtime spans, and **Duplicate reads per phase**.
[Find the heaviest storage items](../guides/find-heaviest-items.md) explains the columns.

## call

```sh
polkadot-frame-trace call --rpc URL --method Core_version --data 0x --check-state-call
polkadot-frame-trace call --offline fixtures/replay/call-people-nonce --summary
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--rpc <URL>` | | Node RPC endpoint. |
| `--at <AT>` | `best` | Block whose state the call reads: a number, a full hash, `best` or `finalized`. |
| `--method <METHOD>` | | Runtime API function, for example `AccountNonceApi_account_nonce`. Required with `--rpc`. |
| `--data <DATA>` | | SCALE-encoded arguments in hex. `0x` for none. |
| `--args <ARGS>` | | Arguments as JSON: a positional array or a named object. Needs metadata V15 or V16. |
| `--offline <DIR>` | | Replay a saved call with no network. |
| `--save <DIR>` | | Save the call and its state for offline replay. |
| `--check-state-call` | off | Compare the result with the node's `state_call` at the same block. |
| `--summary` | off | Print the storage tables. |
| `--json <FILE>` | | Write the trace and result as JSON. `-` writes to stdout and sends tables to stderr. |
| `--otlp [<URL>]` | `http://127.0.0.1:4318` when given without a URL | Send the trace over OTLP/HTTP. |
| `--proof-concurrency <N>` | 8 | RPC requests sent at once, 1 to 64. Each proof request asks for up to 32 keys. |
| `--rpc-timeout <SECONDS>` | 30 | Longest wait for one RPC response, more than 0 and at most 3600 seconds. A request gets four tries, so it can take four times this, 2 minutes by default. Raise it for a node that takes longer to send a large proof. |
| `--cache-dir <DIR>` | | As for `block`. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | As for `block`. |
| `--unauthenticated-metadata` | off | As for `block`. |
| `--runtime-override <WASM>` | | As for `block`. |

With `--rpc`, give `--method` and exactly one of `--data` or `--args`.

## report

```sh
polkadot-frame-trace report block-1.json block-2.json
polkadot-frame-trace report --rpc URL --last 10
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `[BLOCK.json]...` | | Saved `block --json` files. |
| `--from-json <BLOCK.json>` | | More saved files. Repeat the flag for each. |
| `--rpc <URL>` | | Replay a range of finalized blocks from this node. |
| `--from <N>` | | First block of the range. Not the genesis block. |
| `--to <N>` | | Last block of the range, included. At most the latest finalized block. |
| `--last <K>` | | The K most recent finalized blocks. |
| `--concurrency <N>` | 2 | Blocks replayed at once, 1 to 4. |
| `--proof-concurrency <N>` | 8 | RPC requests at once across all blocks, 1 to 64. Each proof request asks for up to 32 keys. |
| `--rpc-timeout <SECONDS>` | 30 | Longest wait for one RPC response, more than 0 and at most 3600 seconds. A request gets four tries, so it can take four times this, 2 minutes by default. Raise it for a node that takes longer to send a large proof. |
| `--cache-dir <DIR>` | | Keep the [disk cache](#disk-cache) in this directory. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | Size limit for the [disk cache](#disk-cache), as bytes used on disk. |
| `--top <N>` | 15 | Rows in each ranked table. JSON is always complete. |
| `--sort-by <proof\|ops\|time>` | `proof` | Rank items, extrinsics and hooks by proof bytes, operations or storage time. |
| `--json <FILE>` | | Write the report as JSON. `-` writes it to stdout and the tables to stderr. |

A range holds at most 10,000 blocks.
`--concurrency`, `--proof-concurrency`, `--rpc-timeout`, `--cache-dir` and `--cache-max-bytes` need `--rpc`; with saved files they are refused.

## follow

```sh
polkadot-frame-trace follow --rpc URL --otlp
polkadot-frame-trace follow --rpc URL --best
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--rpc <URL>` | | Node RPC endpoint, HTTP(S) or WS(S). Required. |
| `--best` | off | Follow the newest blocks instead of finalized ones, and report reorgs. |
| `--otlp [<URL>]` | `http://127.0.0.1:4318` when given without a URL | Send each trace over OTLP/HTTP. |
| `--workers <N>` | 8 | Blocks replayed at once, 1 to 16. |
| `--proof-concurrency <N>` | 16 | RPC requests at once across all workers, 1 to 64. Each proof request asks for up to 32 keys. |
| `--rpc-timeout <SECONDS>` | 30 | Longest wait for one RPC response, more than 0 and at most 3600 seconds. A request gets four tries, so it can take four times this, 2 minutes by default. Raise it for a node that takes longer to send a large proof. |
| `--cache-dir <DIR>` | | Keep the [disk cache](#disk-cache) in this directory. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | Size limit for the [disk cache](#disk-cache), as bytes used on disk. |
| `--unauthenticated-metadata` | off | As for `block`. |

| Limit | Value | Meaning |
| --- | --- | --- |
| Waiting blocks | 4 × `--workers`, at least 8 | Blocks queued for replay. A block older than this many recent heights is skipped. |
| Check for new blocks | every 2 s | How often `follow` asks the node for its newest block. |
| Wait after a failed check | 2 s, doubling to 60 s | Resets after the next check that works. |
| Attempts per block | 4 | For temporary failures, such as a timeout. Other failures stop the block at once. |
| Status line | on change, and every 60 s | Printed at once when `retrying`, `stopped`, `skipped`, `reverted`, `export-rejected` or `export-failed` changes. A check for new blocks that waits on the node delays it. |
| Repeated failure lines | first, then at most once a minute | For failed checks, failed exports and a full queue. A failure that comes back within the minute is not logged again. |
| State kept in memory | 128 MiB | Downloaded state shared by all workers. Counts an upper estimate of what each piece and its index take in memory, so the real use is lower. When full, the least recently read piece goes first. With `--cache-dir`, an evicted piece may still be in the [disk cache](#disk-cache). |
| Traces waiting for export | 32 | Held while the receiver is down. Each holds all spans of one block in memory. When full, the oldest is dropped. |
| Spans per export request | 512 | An acknowledged request is never sent again. A retry sends only the others. A lost acknowledgement can deliver the same spans twice. |
| Wait between export tries | 1 s, doubling to 30 s | Each request of a try, with the exporter's own retries, takes at most the OTLP timeout. The block is not replayed again. |
| Wait at stop | 10 s for replays, then 10 s for exports | See below. |
| Replay deadline | 10 minutes | A replay that runs the runtime this long without waiting on the node stops `follow` with exit code 1, so a supervisor such as systemd starts a fresh process. Time waiting on the node does not count. |

`follow` stops on Ctrl-C, `SIGTERM` or `SIGHUP`. `SIGHUP` does not reload anything.
At the stop, replays waiting on the node end at once and count as `cancelled`.
Replays running on the CPU get the stop wait to finish. Those still running after it count as `cancelled`.
Then `follow` sends waiting traces and prints a summary.
A second signal exits at once.

| Exit code | Meaning |
| --- | --- |
| 0 | Stopped by a signal. Failed blocks and exports are counted in the summary, not in the exit code. |
| 1 | `follow` could not start, for example a malformed `--rpc` or `--otlp` URL, or its replay or export threads stopped unexpectedly, or a replay passed the replay deadline, or it could no longer write to stderr. |
| 2 | The command line was wrong. |
| 130 | A second signal arrived during the stop. |

## Environment variables

These standard OpenTelemetry variables change how `--otlp` sends traces.

| Variable | Default | Meaning |
| --- | --- | --- |
| `OTEL_EXPORTER_OTLP_HEADERS` | none | Headers sent with every export, as `key=value` pairs separated by commas. Values are URL-decoded, so write a space as `%20`. |
| `OTEL_EXPORTER_OTLP_TRACES_HEADERS` | none | The same. Replaces `OTEL_EXPORTER_OTLP_HEADERS` when both are set. |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | 10000 | Longest time for one export request, in milliseconds. Covers connecting, the whole response and the exporter's own retries. A trace of several requests can take several times this. |
| `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` | 10000 | The same. Wins over `OTEL_EXPORTER_OTLP_TIMEOUT`. |

`--otlp` always sets the URL, so `OTEL_EXPORTER_OTLP_ENDPOINT` has no effect.
See [Send traces through Grafana Alloy to Tempo](../guides/send-traces-through-alloy.md#headers-and-authentication).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. `follow` also exits 0 when a signal stops it. |
| 1 | The replay, an input or the export failed. For `report`, at least one input failed. For `follow`, see [its exit codes](#follow). |
| 2 | The command line was wrong, for example a missing or unknown flag. |
| 3 | `call --check-state-call` found a different result from the node's. JSON, save and export still ran. |
| 130 | `follow` got a second stop signal and exited at once. |

## Older flag names

`--oracle` still works as `--check-state-call`, and `--rpc-metadata-fallback` as `--unauthenticated-metadata`.
They are hidden from `--help`.

## Disk cache

`--cache-dir <DIR>` keeps downloaded state with its proofs in `<DIR>/proof-nodes`, and compiled runtimes in `<DIR>/wasm`, between blocks and runs.
`--cache-max-bytes` limits both together.

The directory must be private, because compiled runtimes are native code that polkadot-frame-trace runs.
polkadot-frame-trace creates the directory, `wasm` and `proof-nodes` readable and writable by you alone (mode 0700).
It does not use a directory that another user owns or that group or others can write.
It does not use a `wasm` or `proof-nodes` that is a symlink.
Every directory above it must be owned by root or by you.
Group and others must not write to those directories, unless they are sticky, as `/tmp` is.
If a check fails, it logs one warning and runs without that part of the disk cache. See [Disk cache warnings](../troubleshooting.md#disk-cache-warnings).

The limit counts bytes on disk. Each file counts its size rounded up to 4 KiB, plus 4 KiB.
If the file system gave the file more space, that space counts instead of the size.
When the disk cache is full, compiled runtimes not in use go first, then the oldest state file.
The runtime in use is always kept, and so is space for state: 64 MiB, or a quarter of the limit if that is less.
So the disk cache may go over a small limit.
It may also go over the limit when files cannot be deleted, or when runtimes were compiled since the last check.
`--cache-max-bytes 0` keeps no state on disk and deletes runtimes not in use.

Use one polkadot-frame-trace per cache directory. Each process counts only its own files.
