# Save a block and replay it offline

`--save` keeps everything frame-trace needs to replay a block later: the block, its parent header, the runtime and the checked state.
`--offline` replays it with no network at all.
Use it to share a block with a colleague, attach it to a bug report or test the same block again and again.

## 1. Save a block

```sh
frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block best \
  --save saved-block --json block.json
```

```text
replay #18597005: fetching state (up to 8 concurrent proof requests)
replay #18597005: loading runtime and preparing state
root check: PASS; RPC calls: 91 (timed replay: 0); execution proof: 89714 bytes
fetching state #18597005: 63 proof request rounds; 91 RPC calls
state root: computed 0x02dd5dc7f6d94f1b4669f0449c31fc2a4831b24c89f52db00e24488994299fcf; header 0x02dd5dc7f6d94f1b4669f0449c31fc2a4831b24c89f52db00e24488994299fcf
```

`saved-block/` now holds one file, `artifact.json.zst`, of 3.0 MB.
Most of it is the runtime code.
frame-trace only saves the block after the root check passes.

## 2. Replay it with no network

```sh
frame-trace block --offline saved-block --otlp
```

```text
replay #18597005: loading runtime and preparing state
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 89714 bytes
fetching state #18597005: 0 proof request rounds; 0 RPC calls
state root: computed 0x02dd5dc7f6d94f1b4669f0449c31fc2a4831b24c89f52db00e24488994299fcf; header 0x02dd5dc7f6d94f1b4669f0449c31fc2a4831b24c89f52db00e24488994299fcf
exported 558 spans to http://127.0.0.1:4318
```

`RPC calls: 0`: the offline replay never opens a connection.
It checks every saved value against the parent state root again and runs the [root check](../concepts/what-a-replay-proves.md) again.
An edited or broken saved block fails instead of giving a wrong trace.

`summary --offline saved-block` and `block --offline saved-block --json <file>` work the same way.

## 3. Read saved JSON without replaying

`--json` output can be read again without running the block:

```sh
frame-trace summary --from-json block.json --top 3
```

```text
block 0x9dbf343c8f64b694c206ad56414526b4e463a647326da3cd6aa3e809f4d757d4 (parent 0xa3a3cbeca31df76a3677f21c103a5c6552c24897973bbc965e611b59bb2003ce)

Storage per phase
...
```

This is instant, but it trusts the JSON file.
The saved block directory is the one that is checked again.

| Keep | To |
| --- | --- |
| `--save` directory | Replay the block again, export it again, prove it again. |
| `--json` file | Read the tables again, or feed many blocks to `report`. |

## Runtime API calls

`call --save <dir>` and `call --offline <dir>` work the same way.
See [Trace a runtime API call](trace-a-runtime-api-call.md#replay-a-call-offline).

## Example blocks in the repository

`fixtures/replay/` holds saved blocks you can replay right away:

| Directory | Block |
| --- | --- |
| `people-transfer` | People chain #2, one signed `transfer_keep_alive`. |
| `people-kill-prefix` | People chain #8, a `Sudo.sudo` call that deletes a storage prefix. |
| `people-traced` | People chain #4, a `Utility.batch` of three transfers, with a runtime built with tracing. |
| `westend-asset-hub` | Westend Asset Hub #18,558,320, two required extrinsics. |

The repository's saved blocks share large files through `fixtures/replay/blobs/`.
frame-trace looks for that folder next to the block's directory, so copy both together.
Blocks you save yourself always hold everything in one file.

[Saved block format](../reference/saved-block-format.md) describes what is inside.
