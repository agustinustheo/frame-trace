# JSON output

`block --json` and `call --json` write one trace as JSON.
`report --json` writes the totals of many blocks.
Both carry `"schema_version": 2`.
`summary --from-json` and `report` also read version 1 files, written by older versions; they reject unknown versions.

Field names here are stable.
Some raw values differ from the words in the tables, for example `overlay` for **cache**.

## Block JSON

```sh
frame-trace block --offline fixtures/replay/people-traced --json block.json
```

Top-level fields:

| Field | Meaning |
| --- | --- |
| `schema_version` | `2`. |
| `block_number`, `block_hash`, `parent_hash` | The block. |
| `execution` | The replay result, see below. Absent for recorded `state_traceBlock` input. |
| `timing` | `start_ns` and `duration_ns` of the whole replay. |
| `totals` | Counts for the whole block, see [Stats](#stats). |
| `phases` | The [phases](../concepts/phases.md), in order. |
| `runtime_spans` | Spans the runtime reported, with `id`, `parent_id`, `name`, `target`, times, and the proof size at start and end. Empty without a traced runtime. |
| `skipped_events` | Recorded events that were not storage operations. |
| `call` | Absent for blocks. See [Call JSON](#call-json). |
| `runtime_override` | Present only with `--runtime-override`, see below. |

### `execution`

```json
{
  "root_check": true,
  "proof_bytes": 6586,
  "rpc_calls": 0,
  "measured_rpc_calls": 0,
  "proof_rounds": 0
}
```

| Field | Meaning |
| --- | --- |
| `root_check` | `true` when the [root check](../concepts/what-a-replay-proves.md) passed. |
| `proof_bytes` | The execution proof, in bytes. |
| `rpc_calls` | Requests sent to the node. |
| `measured_rpc_calls` | Requests sent during the timed replay. Always 0. |
| `proof_rounds` | Proof request rounds. |

### A phase

| Field | Meaning |
| --- | --- |
| `kind` | `initialization`, `apply_extrinsic`, `post_inherents` or `finalization`. |
| `label` | The name in tables and Jaeger, such as `extrinsic 2: Utility.batch`. |
| `extrinsic_index`, `call`, `signed` | For extrinsics: the index, `Pallet.call`, and whether it was signed. |
| `call_error` | Why frame-trace could not decode the extrinsic's call name, otherwise `null`. It says nothing about whether the extrinsic succeeded. |
| `stats` | Counts for this phase, see [Stats](#stats). |
| `wall_time` | `start_ns`, `end_ns`, `duration_ns` and `source`. `source` is `runtime_span` when the runtime reported the phase, or `storage_envelope` when the times run from the first to the last storage call. `null` without times. |
| `ops` | The storage operations, in order. |
| `post_inherents` | The nested post-inherents phase, on the extrinsic it ran inside. Otherwise `null`. |

Post-inherent operations and proof bytes are counted in the nested phase, not in its parent.

### A storage operation

The first read of Alice's account in block #4:

```json
{
  "origin": "backend_cold",
  "proof_bytes_added": 166,
  "timing": {
    "start_ns": 3568541,
    "duration_ns": 8250
  },
  "parent_span": 319,
  "seq": 257,
  "op": "get",
  "child_trie": null,
  "child_trie_truncated": false,
  "key": "0x26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da9de1e86a9a8c739864cf3cc5ec2bea59fd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d",
  "key_truncated": false,
  "key_complete": true,
  "pallet": "System",
  "item": "System::Account",
  "map_keys": [
    "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
  ],
  "name": "System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)",
  "label": "get System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)",
  "value_len": 80,
  "value_truncated": false,
  "duplicate_read": false
}
```

| Field | Meaning |
| --- | --- |
| `seq` | Position in the block, from 0. |
| `op` | The operation, such as `get`. See [Storage operations](spans.md#storage-operations). |
| `key` | The raw key in hex. |
| `pallet`, `item`, `map_keys`, `name`, `label` | Decoded names. |
| `key_complete`, `key_truncated` | Whether the key was fully decoded, and whether it was cut. |
| `child_trie`, `child_trie_truncated` | The child trie, for child trie operations. |
| `value_len`, `value_truncated` | Length of the value in bytes, and whether it was cut. |
| `duplicate_read` | `true` if the key was already read in this phase. |
| `origin` | Reads only. `null` or missing means not recorded. See the table below and [Cache or trie](../concepts/cache-or-trie.md). |
| `proof_bytes_added` | Bytes this operation added to the proof. `0` is a measured zero. `null` or missing means not recorded. See [Proof bytes](../concepts/proof-bytes.md). |
| `timing` | `start_ns` and `duration_ns` on your machine. |
| `parent_span` | The `id` of the runtime span this operation ran in. |

The `origin` values, which Jaeger also shows as `frame.storage.origin_raw`:

| JSON `origin` | Jaeger `frame.storage.origin` and tables |
| --- | --- |
| `overlay` | `cache` |
| `backend_warm` | `trie (already in proof)` |
| `backend_cold` | `trie (new)` |
| `null` or missing | `unknown` |

### Stats

| Field | Meaning |
| --- | --- |
| `ops` | Storage operations. |
| `reads`, `writes` | Reads and writes. |
| `distinct_keys` | Different keys. |
| `bytes_read`, `bytes_written` | Sum of value lengths, in bytes. |
| `duplicate_reads` | Reads of a key already read in the same phase. |
| `truncated_keys` | Keys that were cut. |

### `runtime_override`

```json
{
  "path": "people-traced.wasm",
  "code_hash": "0xba01b1de0c55e48c84c70a59381594c8540eb2482ac8e785908de120ce617074",
  "chain_code_hash": "0xee817a9ecd07f4c0f861dd6f2b5105fd316638b827d4d425ae062add4b651c1c",
  "version_differences": []
}
```

`code_hash` is the Wasm that ran.
`chain_code_hash` is the chain's runtime, which did not run.
`version_differences` lists where the two runtime versions differ.

## Call JSON

`call --json` uses the same layout, with one phase for the call and a `call` object:

```json
{
  "method": "AccountNonceApi_account_nonce",
  "data": "0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d",
  "result": "0x08020000",
  "decoded": "520",
  "oracle_match": true,
  "oracle_source": "recorded (unverified)",
  "proof_bytes": 3713,
  "state_root": "0x13c95f8d230c2a4499077afd64a9a7e778f1f430f8f8a0bcc459c83409632567"
}
```

| Field | Meaning |
| --- | --- |
| `method` | The runtime API function. |
| `data` | The SCALE-encoded input. |
| `result`, `decoded` | The raw result in hex, and decoded when the metadata has its type. |
| `oracle_match`, `oracle_source` | With `--check-state-call`: whether the node's `state_call` agreed, and where that answer came from. |
| `proof_bytes` | The call's storage proof, in bytes. |
| `state_root` | State root of the block the call read. |

## Report JSON

```sh
frame-trace report --rpc wss://westend-asset-hub-rpc.polkadot.io --last 5 --json range.json
```

| Field | Meaning |
| --- | --- |
| `schema_version` | `2`. |
| `blocks`, `block_hashes` | How many blocks succeeded, and their hashes. |
| `errors` | Failed inputs, each with `input` and `error`. |
| `totals` | A tally for all blocks, see below. |
| `phases` | A tally per phase group, with `ops_share`, `storage_time_share` and `proof_bytes_share` from 0 to 1. |
| `phase_order` | Phase groups in the order they first ran. |
| `items` | A tally per storage item. |
| `hooks` | A tally per hook. Empty without runtime spans. |
| `extrinsics` | Per extrinsic type: `count`, `median` and `p95` of `ops`, `proof_bytes`, `storage_ns` and `wall_ns`, plus totals. |
| `execution_proof_bytes` | Sum of the blocks' execution proofs. |
| `unattributed_proof_bytes` | Proof bytes not assigned to any operation. |
| `over_attributed_proof_bytes`, `over_attributed_blocks` | Proof bytes counted more than once after rollbacks, and in how many blocks. |
| `runtime_overrides` | Blocks that ran a runtime override. |

A tally holds `ops`, `reads`, `writes`, `iterations`, the read origins (`overlay`, `backend_warm`, `backend_cold`, `origin_unknown`), `bytes_read`, `bytes_written`, `proof_bytes`, `proof_ops_observed`, the duplicate counts per origin and `storage_ns`.

Read `proof_bytes` together with `proof_ops_observed`.
If `proof_ops_observed` is 0, proof bytes were not recorded, even though the sum shows 0.

Medians and the 95th percentile are exact up to 1,024 samples per extrinsic type.
Above that they are within 1.5625% of the true value.
A distribution is `null` when not every sample had the data, for example `wall_ns` without runtime spans.
JSON lists every row; `--top` only limits the terminal tables.
