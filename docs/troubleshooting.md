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
replay #18596954: fetching state (shared limit: 8 concurrent RPC requests)
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
polkadot-frame-trace exports nothing in that case.
See [What a replay proves](concepts/what-a-replay-proves.md).

**Fix.**

- If you used `--runtime-override`, the error ends with `the runtime override may be the cause`.
  Build the override from exactly the source of the runtime on chain.
- If you used `--offline`, the saved block may have been changed. Save it again.
- Otherwise, the block itself may need something polkadot-frame-trace does not support. Open an issue with the block number and chain.

## `the runtime needs host functions polkadot-frame-trace does not have`

This output comes from a test runtime in the repository that imports one made-up host function, `ext_frame_trace_missing_version_1`.
Run this from the repository root to see it:

```sh
polkadot-frame-trace block --offline fixtures/replay/westend-asset-hub \
  --runtime-override fixtures/wasm/missing-host-function.wasm
```

```text
...
root check: NOT VERIFIED; RPC calls: 0
Error: fixtures/replay/westend-asset-hub/artifact.json.zst: the runtime needs host functions polkadot-frame-trace does not have: env:ext_frame_trace_missing_version_1; a runtime override was executed instead of the authenticated :code; the runtime override may be the cause. polkadot-frame-trace's host functions are fixed when it is built, so this chain needs a polkadot-frame-trace build that adds them. See https://agustinustheodorus.com/polkadot-frame-trace/troubleshooting.html#the-runtime-needs-host-functions-polkadot-frame-trace-does-not-have
```

Without `--runtime-override`, the error has no part about the runtime override.

**What it means.** The runtime calls host functions that polkadot-frame-trace does not provide.
The list after the colon names each one.
polkadot-frame-trace provides the same host functions as a stable2609 Polkadot SDK omni-node.
They are fixed when polkadot-frame-trace is built, so no flag adds more.
polkadot-frame-trace stops before it runs the block and exports nothing.

**Fix.** polkadot-frame-trace needs a new build that includes these host functions.
Open an issue with the chain name and the list of names from the error.
If you build polkadot-frame-trace yourself, [the developer notes](https://github.com/agustinustheo/polkadot-frame-trace/blob/main/dev/README.md#add-host-functions-for-a-chain) describe how to add them.

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
polkadot-frame-trace checks the file before it replays anything.

**Fix.** Pass the `.wasm` or `.compact.compressed.wasm` file from the runtime build, under `target/release/wbuild/`.
Drop `--save` when you use an override.
See [See runtime calls inside extrinsics](guides/see-runtime-calls.md).

## `<N> of <M> spans were not exported`

```text
Error: exporting to http://127.0.0.1:4999: 428 of 428 spans were not exported (first error: Operation failed: HTTP export failed: network error)
```

**What it means.** Nothing is listening at the OTLP URL, or it refused the spans.

**Fix.**

- Check that Jaeger or your OTLP receiver is running: `docker ps`.
- Check the port. OTLP/HTTP is usually 4318. The Jaeger web page, 16686, does not accept traces.
- Map the port when you start Jaeger: `-p 4318:4318`.

## `<N> of <M> spans rejected by the receiver`

```text
Error: exporting to http://127.0.0.1:4319: 5 of 428 spans rejected by the receiver: span attribute limit exceeded
```

This run used a test receiver that rejects five spans.

**What it means.** The receiver answered, but refused some spans.
The text after the last colon is the receiver's own reason.
polkadot-frame-trace exits with code 1, because the trace in your viewer is incomplete.

**Fix.** Read the receiver's reason and its logs.
Common causes are limits on attributes per span, span size or spans per request.
Raise the limit in the receiver's configuration, or send to a receiver without it, such as a local Jaeger.

## `follow poll failed`

```text
follow poll failed: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 2s
follow poll still failing after 6 attempts over 1m06s: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 1m00s
```

**What it means.** `follow` could not ask the node for its newest block.
It keeps running and tries again, waiting longer after each failure.
While it fails, the log repeats the line once a minute with the count so far.

**Fix.**

- Check that the node is running and that the `--rpc` URL and port are right.
- Wait for `follow poll recovered after <N> failed attempt(s)`. Nothing else is needed.

## `OTLP export failed`

```text
OTLP export failed (1 attempt(s) in a row): 578 of 578 spans were not exported (first error: Operation failed: HTTP export failed: network error); retrying in 1s, 10 trace(s) waiting
```

**What it means.** `follow` replayed a block but could not send its trace.
It keeps the trace and tries again. It does not replay the block, and it does not resend requests the receiver acknowledged.
If an acknowledgement is lost, the receiver can get the same spans twice, with the same IDs.
It logs `OTLP export recovered` when the receiver is back.

**Fix.** Start the receiver again, or fix the `--otlp` URL. The fixes for [`<N> of <M> spans were not exported`](#n-of-m-spans-were-not-exported) apply.

## `OTLP export queue full`

```text
OTLP export queue full (32 traces); dropped the oldest, block #19025706 0x4c91709d161faccb5df6c0561e7a745f7b27d8d9b6e472c8d60f5e19dd2538ed (1 dropped so far)
```

**What it means.** The receiver has been down long enough that `follow` holds the most traces it keeps.
To make room for a new trace, it drops the oldest one. The status line counts it as `export-dropped`.
While this goes on, the line repeats once a minute.

**Fix.** Start the receiver again. Dropped traces are not sent later.
To trace a dropped block, run `polkadot-frame-trace block --rpc <URL> --block <hash> --otlp` with the hash from the line.
This works while the node still has the state before that block.

## No traces in Jaeger, but no error

polkadot-frame-trace printed `exported <N> spans`, but Jaeger shows nothing.

**Fix.**

- Pick the service `polkadot-frame-trace` and click **Find Traces**.
- Widen **Lookback**. Traces are stamped with the time you ran polkadot-frame-trace.
- Check that polkadot-frame-trace and the page you opened talk to the same Jaeger. Jaeger's default in-memory storage loses traces on restart.
- Without `--otlp`, polkadot-frame-trace sends nothing to Jaeger. When you give neither `--otlp` nor `--json`, `block` ends with `Use --otlp or --json to export it.`

## `unsupported metadata version V13 (need V14, V15 or V16)`

```text
Error: reading metadata from /tmp/ftdocs/v13.scale: unsupported metadata version V13 (need V14, V15 or V16)
```

**What it means.** polkadot-frame-trace decodes names with metadata V14, V15 or V16.
Old runtimes only have older metadata.
You see this error when you pass such a file with `--metadata`.

**Fix.** Use metadata V14, V15 or V16.
For a replayed block, polkadot-frame-trace instead prints the notice in the next section and continues without names.

## `notice: local metadata unavailable (…); using unnamed storage and hex results`

```text
notice: local metadata unavailable (saved proof does not cover metadata execution); using unnamed storage and hex results
```

**What it means.** polkadot-frame-trace could not run the runtime's metadata API for this state.
The trace is still complete and checked.
Keys show as `<unknown>` or raw hex, and call results as hex.

**Fix.** For a live block, `--unauthenticated-metadata` lets polkadot-frame-trace use the node's metadata.
It is not checked, and the output says so.
For a call, use `--data` instead of `--args`.

## `overall request deadline exceeded`

```text
replay #15366850: fetching state (shared limit: 8 concurrent RPC requests)
root check: NOT VERIFIED; RPC calls: 8
Error: RPC error: state_getReadProof: overall request deadline exceeded
```

This run used `wss://asset-hub-paseo-rpc.n.dwellir.com`.

**What it means.** The node took too long to answer one request.
`--rpc-timeout` is the wait for one try, 30 seconds by default.
A request gets four tries, so it fails after 2 minutes by default.

The largest answer is the proof of the runtime code, about 2.7 MB.
Public Westend nodes took 7 to 20 seconds to send it, and this node 25 to 35 seconds.

**Fix.** Raise `--rpc-timeout` when one try needs more than 30 seconds.
This run failed with the old default of 7.5 seconds.
With `--rpc-timeout 120`, the same command passed:

```text
root check: PASS; RPC calls: 113 (timed replay: 0); execution proof: 101420 bytes
```

See [defaults and limits](reference/commands.md#block).

## The first run is slow

```text
replay #18598757: fetching state (shared limit: 8 concurrent RPC requests)
```

This line can stay on screen for 20 seconds or more.

**What it means.** polkadot-frame-trace is fetching, with proofs, every piece of state the block reads.
A Westend Asset Hub block needed 64 rounds of requests in [Your first trace](getting-started/first-trace.md).
The first run also compiles the runtime.

**Fix.**

- Add `--cache-dir <dir>`. Later blocks and runs reuse the state earlier ones downloaded. In [Analyse many blocks](guides/analyse-many-blocks.md), RPC calls per block dropped from 85 to about 40.
- Compiled runtimes are kept in `~/.cache/polkadot-frame-trace/wasm` even without `--cache-dir`.
- Raise `--proof-concurrency` on a fast connection. See [defaults and limits](reference/commands.md).
- Use a node close to you, or your own node.

## `disk cache: removing <dir>/nodes, left by an older polkadot-frame-trace`

```text
disk cache: removing /private/tmp/pft-docs/.polkadot-frame-trace-cache/nodes, left by an older polkadot-frame-trace; the disk cache is now in /private/tmp/pft-docs/.polkadot-frame-trace-cache/proof-nodes
disk cache: kept /private/tmp/pft-docs/.polkadot-frame-trace-cache/nodes: it holds files polkadot-frame-trace did not write
```

**What it means.** Earlier versions kept the [disk cache](reference/commands.md#disk-cache) as one file per piece of state.
A long `follow` could leave a million files there, using several GiB.
polkadot-frame-trace now keeps it in a few large files. It deletes the files it wrote in `nodes` in the background, and keeps anything else.
The second line appears only when something else is in the folder.

**Fix.** Nothing to do. If polkadot-frame-trace stops first, the next run carries on. If you see the second line, look at what is left in `<dir>/nodes` and delete it yourself if you do not need it.

## `warning: disk cache: cannot write: …`

```text
warning: disk cache: cannot write: Permission denied (os error 13)
```

**What it means.** polkadot-frame-trace could not write to the disk cache, for example because the disk is full or the directory is read-only.
The trace is not affected. Later blocks download that state from the node again.
This line is logged once per run.

**Fix.** Free some disk space, or point `--cache-dir` at a directory you can write to.

## `warning: disk cache: --cache-max-bytes <N> is less than the runtimes in use …`

```text
warning: disk cache: --cache-max-bytes 1048576 is less than the runtimes in use (26710016 bytes) plus 262144 bytes kept for downloaded state; the disk cache stays above the limit
```

**What it means.** The compiled runtime polkadot-frame-trace is using, plus the space it always keeps for downloaded state, is larger than `--cache-max-bytes`.
polkadot-frame-trace keeps both, so the disk cache stays above the limit. See [Disk cache](reference/commands.md#disk-cache).

**Fix.** Raise `--cache-max-bytes` above the size in the message plus the kept space. A compiled Westend Asset Hub runtime is about 26 MB.

## `warning: disk cache: old files in <dir> cannot be deleted; not writing until they can`

```text
warning: disk cache: /private/tmp/pft-docs/stuck/proof-nodes/0000000000.seg: Permission denied (os error 13)
warning: disk cache: old files in /private/tmp/pft-docs/stuck/proof-nodes cannot be deleted; not writing until they can
```

**What it means.** The disk cache is full, and polkadot-frame-trace could not delete its oldest file. It still counts that file, and stops writing new state so the disk cache does not grow.
Traces are not affected. Blocks ask the node for state the disk cache does not have.
polkadot-frame-trace tries again before each write, and writes again once the file is gone.

**Fix.** Give your user permission to delete files in the folder named in the message.

## `warning: disk cache: <dir>: group or others can write to it …; running without it`

```text
warning: disk cache: /private/tmp/pft-docs/shared: group or others can write to it (mode 775); running without it
```

**What it means.** The [disk cache](reference/commands.md#disk-cache) directory must be private, because compiled runtimes in it are native code that polkadot-frame-trace runs.
This one can be written by other users, so polkadot-frame-trace does not use it. A directory owned by another user gets `owned by user <uid>, not by this user` instead.
Traces are not affected, but each run compiles the runtime again and asks the node for everything.

**Fix.** Run `chmod go-w <dir>`, or point `--cache-dir` at a directory only you can write to.

## `warning: disk cache: <dir>/proof-nodes: …; running without it`

```text
warning: disk cache: /private/tmp/pft-docs/blocked/proof-nodes: is not a directory; running without it
```

**What it means.** polkadot-frame-trace could not create or open the folder for downloaded state, here because a file has that name.
A symlink there is refused too, with `is a symlink`.
It runs without keeping state on disk. Traces are not affected, but each block asks the node for everything.

**Fix.** Remove what is in the way, or point `--cache-dir` at another directory.

## `warning: disk cache: <dir>/wasm: …; runtimes are compiled again each run`

```text
warning: disk cache: /private/tmp/pft-docs/nowasm/wasm: is not a directory; runtimes are compiled again each run
```

**What it means.** polkadot-frame-trace could not create the folder for compiled runtimes, or it is a symlink, which is refused. Each run compiles the runtime again, which can take a few seconds per block's first replay.

**Fix.** Remove what is in the way, or point `--cache-dir` at another directory.

## `warning: disk cache: skipping <file>: …`

```text
warning: disk cache: skipping /private/tmp/pft-docs/ro/proof-nodes/0000000000.seg: Permission denied (os error 13)
```

**What it means.** One file of the disk cache could not be opened. polkadot-frame-trace leaves it alone and does not count it towards `--cache-max-bytes`.

**Fix.** Give your user read and write access to the file, or delete it.

## `failed to lookup address information`

```text
root check: NOT VERIFIED; RPC calls: 4
Error: RPC error: chain_getFinalizedHead: connect wss://asset-hub-westend-rpc.dwellir.com: IO error: failed to lookup address information: nodename nor servname provided, or not known
```

This run used `--block finalized`, so the first request was `chain_getFinalizedHead`.
With a block number it would be `chain_getBlockHash`.

**What it means.** The RPC host name does not resolve.

**Fix.** Check the URL for typos and check your network.

## `the argument '…' cannot be used with '…'`

```text
error: the argument '--rpc <URL>' cannot be used with '--metadata <FILE>'

Usage: polkadot-frame-trace block --block <NUMBER|HASH|best|finalized> <--rpc <URL>|--offline <DIR>|--from-file <FILE>|--from-json <FILE>>
```

**What it means.** Two flags do not work together, so polkadot-frame-trace refuses to run instead of ignoring one.
For example, `--metadata` only applies to `--from-file`, `--block` only to `--rpc`, and `report --cache-dir` only to `report --rpc`.
The exit code is 2.

**Fix.** Drop the flag that does not apply to your source.
[Commands and flags](reference/commands.md) says which flags go with which source.

## `range limit is 10000 blocks`

```text
Error: range limit is 10000 blocks
Error: range must be ordered, non-genesis and finalized (head #18597087)
```

**What it means.** `report --from --to` got a range that is too long, starts at block 0, runs backwards, or ends after the latest final block.

**Fix.** Keep ranges to 10,000 blocks, start at block 1 or later and end at or before the head shown.

## `state_call check: MISMATCH`

**What it means.** With `--check-state-call`, the node's own `state_call` returned different bytes from polkadot-frame-trace's run.
polkadot-frame-trace exits with code 3 after writing all outputs.

**Fix.** Check that the node runs the same runtime as the block's state.
If you used `--runtime-override`, a mismatch is expected when the override behaves differently.
