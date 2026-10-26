# Save a block and replay it offline

`--save` keeps everything frame-trace needs to replay a block later: the block, its parent header, the runtime and the checked state.
`--offline` replays it with no network at all.
Use it to share a block with a colleague, attach it to a bug report or test the same block again and again.
Read [Before you share](#before-you-share) first.

## 1. Save a block

```sh
frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block finalized \
  --save saved-block --json block.json
```

```text
replay #18598772: fetching state (shared limit: 8 concurrent RPC requests)
replay #18598772: loading runtime and preparing state
root check: PASS; RPC calls: 92 (timed replay: 0); execution proof: 91062 bytes
fetching state #18598772: 63 proof request rounds; 92 RPC calls
state root: computed 0xfb9758620031f5227e6b1367d4deb04cab717c33605bc96b17f11227d7279b7a; header 0xfb9758620031f5227e6b1367d4deb04cab717c33605bc96b17f11227d7279b7a
```

`saved-block/` now holds one file, `artifact.json.zst`, of 3.0 MB.
Most of it is the runtime code.
frame-trace only saves the block after the root check passes.

## 2. Replay it with no network

```sh
frame-trace block --offline saved-block --otlp
```

```text
replay #18598772: loading runtime and preparing state
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 91062 bytes
fetching state #18598772: 0 proof request rounds; 0 RPC calls
state root: computed 0xfb9758620031f5227e6b1367d4deb04cab717c33605bc96b17f11227d7279b7a; header 0xfb9758620031f5227e6b1367d4deb04cab717c33605bc96b17f11227d7279b7a
exported 555 spans to http://127.0.0.1:4318
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
block 0x92938c3ead0669e3ced75145374031d429653c6b2589245517d5c56ae9adbaab (parent 0xf551098c224fa6e973e4214523c9b78c951848beca00d44c1a4b618688ff42cb)

Storage per phase
...
```

`0x92938c3e…baab` is the hash of block #18598772, the block saved in step 1.

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

## Before you share

A saved block holds real chain data: the block, its extrinsics and the state values it read.
`--json` output and traces hold raw storage keys and decoded account addresses.
With `--runtime-override`, traces and JSON also hold the local path of your Wasm file.
None of this is secret on a public chain, but review it before you share or upload it, especially for a private or test network.
