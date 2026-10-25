# Troubleshooting

Each heading below is an error message, or the start of one.
Search this page for the text you see.

## `invalid replay input: block <N> not found`

```text
root check: NOT VERIFIED; RPC calls: 1
Error: invalid replay input: block 999999999 not found
```

**What it means.** The node has no block at that height or with that hash.

**Fix.** Check the number against a block explorer for the same chain.
Check that the RPC URL is for the chain you mean.
A hash must be the full 32 bytes, as in `0x` and 64 hex digits.

## `invalid replay input: block number: invalid digit found in string`

```text
root check: NOT VERIFIED; RPC calls: 0
Error: invalid replay input: block number: invalid digit found in string
```

This run used `--block latest`.

**What it means.** `--block` got something that is not `best`, `finalized`, a number or a hash.

**Fix.** Use `--block finalized`, `--block best`, a block number or a full hash.

## `parent state may be pruned (use an archive endpoint)`

```text
replay #18596954: fetching state (up to 8 concurrent proof requests)
root check: NOT VERIFIED; RPC calls: 4
Error: RPC unavailable: state_getReadProof: {"code":4003,"message":"Client error: State already discarded for 0x…"}; parent state may be pruned (use an archive endpoint)
```

The part inside `{…}` is the node's own message, so its wording varies between nodes.

**What it means.** The node no longer keeps the state of the block's parent.
Most nodes keep only recent state.

**Fix.** Use an archive node for older blocks.
Or trace a newer block, such as `--block finalized`.

## `runtime execution failed (root mismatch, unsupported host function, or invalid block)`

```text
replay #2: loading runtime and preparing state
root check: NOT VERIFIED; RPC calls: 0
Error: /tmp/ftdocs/tampered/artifact.json.zst: runtime execution failed (root mismatch, unsupported host function, or invalid block): Execution aborted due to trap: wasm trap: wasm `unreachable` instruction executed
WASM backtrace:
...
```

This run used a saved block whose header state root had been edited.

**What it means.** The runtime stopped while running the block.
The usual cause is the root check: the runtime computed a different state root from the header, and the runtime's own check aborted.
frame-trace exports nothing in that case.
See [What a replay proves](concepts/what-a-replay-proves.md).

**Fix.**

- If you used `--runtime-override`, the error ends with `the runtime override may be the cause`.
  Build the override from exactly the source of the runtime on chain.
- If you used `--offline`, the saved block may have been changed. Save it again.
- If the text after the colon names missing imports, see the next section.
- Otherwise, the block itself may need something frame-trace does not support. Open an issue with the block number and chain.

## `runtime requires function imports which are not present on the host`

```text
Error: fixtures/replay/people-transfer/artifact.json.zst: runtime execution failed (root mismatch, unsupported host function, or invalid block): Other error happened while constructing the runtime: runtime requires function imports which are not present on the host: 'env:ext_frame_trace_missing_version_1'; a runtime override was executed instead of the authenticated :code; the runtime override may be the cause
```

This run used a test runtime that imports a made-up host function.

**What it means.** The runtime calls a host function that frame-trace does not provide.
frame-trace provides the same host functions as a stable2609 Polkadot SDK omni-node.
A chain with its own custom host functions cannot be replayed.

**Fix.** None in frame-trace for custom host functions.
If the missing function is a standard Polkadot SDK one, open an issue with its name.

## `--runtime-override <file>: … not a Wasm module`

```text
Error: --runtime-override /tmp/ftdocs/notwasm.wasm: invalid replay input: not a Wasm module: expected the \0asm magic, plain or inside an SDK-compressed blob
```

Related messages:

```text
Error: reading --runtime-override /tmp/ftdocs/nope.wasm: No such file or directory (os error 2)
error: the argument '--runtime-override <WASM>' cannot be used with '--save <DIR>'
```

**What it means.** The file is not a runtime, does not exist, or you combined it with `--save`.
frame-trace checks the file before it replays anything.

**Fix.** Pass the `.wasm` or `.compact.compressed.wasm` file from the runtime build, under `target/release/wbuild/`.
Drop `--save` when you use an override.
See [See runtime calls inside extrinsics](guides/see-runtime-calls.md).

## `<N> of <N> spans were not exported`

```text
Error: exporting to http://127.0.0.1:4999: 428 of 428 spans were not exported (first error: Operation failed: HTTP export failed: network error)
```

**What it means.** Nothing is listening at the OTLP URL, or it refused the spans.

**Fix.**

- Check that Jaeger or your collector is running: `docker ps`.
- Check the port. OTLP/HTTP is usually 4318. The Jaeger web page, 16686, does not accept traces.
- Map the port when you start Jaeger: `-p 4318:4318`.

## No traces in Jaeger, but no error

frame-trace printed `exported <N> spans`, but Jaeger shows nothing.

**Fix.**

- Pick the service `frame-trace` and click **Find Traces**.
- Widen **Lookback**. Traces are stamped with the time you ran frame-trace.
- Check that frame-trace and the page you opened talk to the same Jaeger. Jaeger's default in-memory storage loses traces on restart.
- Without `--otlp`, frame-trace sends nothing. It prints `Use --otlp or --json to export it.`

## `unsupported metadata version V13 (need V14, V15 or V16)`

```text
Error: reading metadata from /tmp/ftdocs/v13.scale: unsupported metadata version V13 (need V14, V15 or V16)
```

**What it means.** frame-trace decodes names with metadata V14 or newer.
Old runtimes only have older metadata.
You see this error when you pass such a file with `--metadata`.

**Fix.** Use metadata V14 or newer.
For a replayed block, frame-trace instead prints the notice in the next section and continues without names.

## `notice: local metadata unavailable (…); using unnamed storage and hex results`

```text
notice: local metadata unavailable (saved proof does not cover metadata execution); using unnamed storage and hex results
```

**What it means.** frame-trace could not run the runtime's metadata API for this state.
The trace is still complete and checked.
Keys show as `<unknown>` or raw hex, and call results as hex.

**Fix.** For a live block, `--unauthenticated-metadata` lets frame-trace use the node's metadata.
It is not checked, and the output says so.
For a call, use `--data` instead of `--args`.

## The first run is slow

```text
replay #18597850: fetching state (up to 8 concurrent proof requests)
```

This line can stay on screen for 20 seconds or more.

**What it means.** frame-trace is fetching, with proofs, every piece of state the block reads.
A Westend Asset Hub block needed 63 rounds of requests in [Your first trace](getting-started/first-trace.md).
The first run also compiles the runtime.

**Fix.**

- Add `--cache-dir <dir>`. Later blocks and runs reuse fetched state. In [Analyse many blocks](guides/analyse-many-blocks.md), RPC calls per block dropped from 85 to about 40.
- Compiled runtimes are kept in `~/.cache/frame-trace/wasm` even without `--cache-dir`.
- Raise `--proof-concurrency` on a fast connection. The default is 8, the maximum 64.
- Use a node close to you, or your own node.

## `failed to lookup address information`

```text
root check: NOT VERIFIED; RPC calls: 4
Error: RPC error: chain_getBlockHash: connect wss://asset-hub-westend-rpc.dwellir.com: IO error: failed to lookup address information: nodename nor servname provided, or not known
```

**What it means.** The RPC host name does not resolve.

**Fix.** Check the URL for typos and check your network.

## `range limit is 10000 blocks`

```text
Error: range limit is 10000 blocks
Error: range must be ordered, non-genesis and finalized (head #18597087)
```

**What it means.** `report --from --to` got a range that is too long, starts at block 0, runs backwards, or ends after the latest final block.

**Fix.** Keep ranges to 10,000 blocks, start at block 1 or later and end at or before the head shown.

## `state_call check: MISMATCH`

**What it means.** With `--check-state-call`, the node's own `state_call` returned different bytes from frame-trace's run.
frame-trace exits with code 3 after writing all outputs.

**Fix.** Check that the node runs the same runtime as the block's state.
If you used `--runtime-override`, a mismatch is expected when the override behaves differently.
