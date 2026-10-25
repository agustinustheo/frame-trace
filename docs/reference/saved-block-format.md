# Saved block format

`--save <DIR>` writes one file, `<DIR>/artifact.json.zst`.
It is a Zstandard-compressed JSON object.
It holds everything needed to replay the block with no network, and nothing else.
It has no paths to other files.

## What is inside

| Field | Meaning |
| --- | --- |
| `format_version` | `1`. Other versions are rejected. |
| `kind` | `block` or `call`. |
| `block_hash` | Hash of the block. Checked against the header. |
| `block` | The block as `chain_getBlock` returned it: header and extrinsics. |
| `parent_header` | The parent block's header. Its hash must match the block's parent hash. |
| `runtime_version` | The runtime version the node reported, kept for reference. |
| `metadata` | Metadata in hex, kept for reference. frame-trace derives it again from the runtime before decoding. |
| `proof_nodes` | The trie nodes the replay read, in hex, each stored once. They include the runtime code. |
| `call` | For calls only: `method`, `data`, the `result`, and the node's `state_call` answer if you used `--check-state-call`. |

For a call, `block` and `parent_header` both hold the header of the block whose state the call read, and the block has no extrinsics.

## What is checked on replay

The saved file is not trusted.
On `--offline`, frame-trace checks:

- the block hash against the header, and the parent header against the block's parent hash;
- every trie node it uses, by walking down from the parent state root;
- the runtime code, read through those same nodes;
- the [root check](../concepts/what-a-replay-proves.md) at the end;
- for calls, that the replay returns the saved `result`.

`runtime_version` and `metadata` are never used to run the block.
A saved `state_call` answer is only compared, and the output marks it `recorded (unverified)`.

## Size limits

frame-trace refuses files beyond these limits:

| Limit | Value |
| --- | --- |
| JSON after decompression | 64 MiB |
| Number of proof nodes | 100,000 |
| Total size of proof nodes | 24 MiB |

A live Westend Asset Hub block saved to 3.0 MB.
Most of that is the runtime code.

## What is not saved

- Traces, timings and tables. Replay the block, or keep `--json` output too.
- A runtime override. `--runtime-override` cannot be combined with `--save`.
- State the replay did not read. The file covers this block only.
