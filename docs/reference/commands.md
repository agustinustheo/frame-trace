# Commands and flags

This page lists every command and flag, taken from `frame-trace <command> --help`.

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
frame-trace block --rpc URL --block finalized --otlp
frame-trace block --offline fixtures/replay/people-traced --json block.json
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
| `--proof-concurrency <N>` | 8 | Proof requests sent at once, 1 to 64. Each request asks for up to 32 keys. |
| `--cache-dir <DIR>` | | Keep fetched state and compiled runtimes in this directory between runs. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | Size limit for the disk cache, in bytes. A runtime in use is kept even above the limit. |
| `--unauthenticated-metadata` | off | If the runtime's own metadata cannot be loaded, use the node's (or the saved) metadata unchecked. Output says so. |
| `--runtime-override <WASM>` | | Run this Wasm instead of the chain's runtime. See [See runtime calls inside extrinsics](../guides/see-runtime-calls.md). Cannot be combined with `--save`. |

Without `--otlp` or `--json`, `block` prints a one-line count of phases and operations.

Flags that do not apply to the chosen source are refused, with exit code 2: `--block` and `--proof-concurrency` need `--rpc`, `--metadata` needs `--from-file`, and the cache and metadata flags do not work with `--from-file` or `--from-json`.

Without `--cache-dir`, compiled runtimes are still kept in `$XDG_CACHE_HOME/frame-trace/wasm` (or `~/.cache/frame-trace/wasm`), and fetched state stays in memory only.

## summary

```sh
frame-trace summary --rpc URL --block finalized
frame-trace summary --from-json block.json
```

`summary` takes the same source flags as `block`, plus `--save`, `--proof-concurrency`, `--cache-dir`, `--cache-max-bytes`, `--unauthenticated-metadata` and `--runtime-override`.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--top <N>` | 15 | Rows in each item, extrinsic and hook table, and per phase in the duplicate reads table. |

It prints these tables: **Storage per phase**, **Phase shares**, **Top items by proof bytes**, **Extrinsic distributions**, **Hooks** (only with runtime spans) and **Duplicate reads per phase**.
[Find the heaviest storage items](../guides/find-heaviest-items.md) explains the columns.

## call

```sh
frame-trace call --rpc URL --method Core_version --data 0x --check-state-call
frame-trace call --offline fixtures/replay/call-people-nonce --summary
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--rpc <URL>` | | Node RPC endpoint. |
| `--at <AT>` | `best` | Block whose state the call reads: a number, a full hash, `best` or `finalized`. |
| `--method <METHOD>` | | Runtime API function, for example `AccountNonceApi_account_nonce`. Required with `--rpc`. |
| `--data <DATA>` | | SCALE-encoded arguments in hex. `0x` for none. |
| `--args <ARGS>` | | Arguments as JSON: a positional array or a named object. Needs metadata V15 or newer. |
| `--offline <DIR>` | | Replay a saved call with no network. |
| `--save <DIR>` | | Save the call and its state for offline replay. |
| `--check-state-call` | off | Compare the result with the node's `state_call` at the same block. |
| `--summary` | off | Print the storage tables. |
| `--json <FILE>` | | Write the trace and result as JSON. `-` writes to stdout and sends tables to stderr. |
| `--otlp [<URL>]` | `http://127.0.0.1:4318` when given without a URL | Send the trace over OTLP/HTTP. |
| `--proof-concurrency <N>` | 8 | Proof requests sent at once, 1 to 64. |
| `--cache-dir <DIR>` | | As for `block`. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | As for `block`. |
| `--unauthenticated-metadata` | off | As for `block`. |
| `--runtime-override <WASM>` | | As for `block`. |

With `--rpc`, give `--method` and exactly one of `--data` or `--args`.

## report

```sh
frame-trace report block-1.json block-2.json
frame-trace report --rpc URL --last 10
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
| `--proof-concurrency <N>` | 8 | Proof requests at once across all blocks, 1 to 64. |
| `--cache-dir <DIR>` | | Keep fetched state and compiled runtimes in this directory. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | Size limit for the disk cache, in bytes. |
| `--top <N>` | 15 | Rows in each ranked table. JSON is always complete. |
| `--sort-by <proof\|ops\|time>` | `proof` | Rank items, extrinsics and hooks by proof bytes, operations or storage time. |
| `--json <FILE>` | | Write the report as JSON. `-` writes it to stdout and the tables to stderr. |

A range holds at most 10,000 blocks.
`--concurrency`, `--proof-concurrency`, `--cache-dir` and `--cache-max-bytes` need `--rpc`; with saved files they are refused.

## follow

```sh
frame-trace follow --rpc URL --otlp
frame-trace follow --rpc URL --best
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--rpc <URL>` | | Node RPC endpoint, HTTP(S) or WS(S). Required. |
| `--best` | off | Follow the newest blocks instead of finalized ones, and report reorgs. |
| `--otlp [<URL>]` | `http://127.0.0.1:4318` when given without a URL | Send each trace over OTLP/HTTP. |
| `--proof-concurrency <N>` | 8 | Proof requests at once, 1 to 64. |
| `--cache-dir <DIR>` | | Keep fetched state and compiled runtimes in this directory. |
| `--cache-max-bytes <N>` | 1073741824 (1 GiB) | Size limit for the disk cache, in bytes. |
| `--unauthenticated-metadata` | off | As for `block`. |

## Environment variables

These standard OpenTelemetry variables change how `--otlp` sends traces.

| Variable | Default | Meaning |
| --- | --- | --- |
| `OTEL_EXPORTER_OTLP_HEADERS` | none | Headers sent with every export, as `key=value` pairs separated by commas. Values are URL-decoded, so write a space as `%20`. |
| `OTEL_EXPORTER_OTLP_TRACES_HEADERS` | none | The same. Replaces `OTEL_EXPORTER_OTLP_HEADERS` when both are set. |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | 10000 | How long to wait for the receiver, in milliseconds. |
| `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` | 10000 | The same. Wins over `OTEL_EXPORTER_OTLP_TIMEOUT`. |

`--otlp` always sets the URL, so `OTEL_EXPORTER_OTLP_ENDPOINT` has no effect.
See [Send traces through Grafana Alloy to Tempo](../guides/send-traces-through-alloy.md#headers-and-authentication).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | The replay, an input or the export failed. For `report`, at least one input failed. |
| 2 | The command line was wrong, for example a missing or unknown flag. |
| 3 | `call --check-state-call` found a different result from the node's. JSON, save and export still ran. |

## Older flag names

`--oracle` still works as `--check-state-call`, and `--rpc-metadata-fallback` as `--unauthenticated-metadata`.
They are hidden from `--help`.
