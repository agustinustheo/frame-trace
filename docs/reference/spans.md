# Span names and attributes

Every trace uses the service name `frame-trace`.
This page lists the span names and the attributes on each kind of span.

## Span tree

```text
block #4 (0x06618995…0637)                  block span
├─ initialization                           phase span
│  └─ init_block                            runtime span (traced runtimes only)
│     ├─ get System::Number                 storage operation span
│     └─ on_initialize: Aura                hook span (traced runtimes only)
├─ extrinsic 2: Utility.batch               phase span
│  ├─ apply_extrinsic                       runtime span
│  │  └─ batch                              runtime span
│  │     └─ transfer_keep_alive             runtime span
│  │        └─ get System::Account(5FHn…)   storage operation span
│  └─ post inherents (on_poll)              phase span, nested
└─ finalization                             phase span
```

## Span names

| Span | Name |
| --- | --- |
| Block | `block #<number> (<first 10 characters of the hash>…<last 4>)` |
| Runtime API call | `call <method> at #<number>` |
| Either, with a runtime override | The name above, then ` [runtime override]` |
| Phase | `initialization`, `extrinsic <N>: <Pallet>.<call>`, `post inherents (on_poll)`, `finalization`. See [Phases of a block](../concepts/phases.md). |
| Runtime span | The name the runtime reports: `apply_extrinsic`, `init_block`, a call name such as `transfer_keep_alive`. |
| Hook | `<hook>: <Pallet>`, for example `on_initialize: Balances`. Hooks are `on_initialize`, `on_finalize`, `on_idle` and `on_poll`. |
| Storage operation | `<operation> <decoded key>`, for example `get System::Account(5Grw…)`. Long names are cut at 160 characters. |

## Storage operations

| Operation | Kind | Meaning |
| --- | --- | --- |
| `get` | read | Read a value. |
| `exists` | read | Check that a key exists. |
| `hash` | read | Read the hash of a value. |
| `put` | write | Write a value. |
| `clear` | write | Delete a key. |
| `append` | write | Add an item to a value. |
| `clear_prefix` | write | Delete every key under a prefix. |
| `kill_child` | write | Delete a whole child trie. |
| `storage_root` | other | Compute a storage root. |
| `index_transaction`, `renew_transaction_index` | other | Index a transaction for storage. |
| `next_key` | other | Find the next key. The result can still change from pending writes. |
| `iteration` | other | Step through keys in the trie. |

Child trie operations use the same names and carry `frame.storage.child_trie`.

## Block and call span attributes

| Attribute | Type | Meaning |
| --- | --- | --- |
| `frame.block.number` | int | Block number. |
| `frame.block.hash` | string | Block hash. |
| `frame.block.parent_hash` | string | Parent block hash. |
| `frame.block.extrinsics` | int | Number of extrinsics. |
| `frame.trace.skipped_events` | int | Recorded events that were not storage operations. |
| `frame.timing` | string | `timed replay` for a replay, `no timing` for a recorded `state_traceBlock` file. See [Timings](../concepts/timings.md). |
| `frame.storage.*` totals | int | See [Totals](#totals). |

On a runtime API call trace, also:

| Attribute | Type | Meaning |
| --- | --- | --- |
| `frame.trace.kind` | string | `call`. |
| `frame.call.method` | string | The runtime API function. |
| `frame.root_check` | string | `not applicable`: calls have no root check. |
| `frame.state.root` | string | State root of the block the call read. |
| `frame.proof.bytes` | int | The call's storage proof, in bytes. |
| `frame.call.oracle_match` | bool | With `--check-state-call`: whether the node's `state_call` returned the same bytes. |
| `frame.call.oracle_source` | string | Where the compared answer came from. `recorded (unverified)` when replayed offline. |

With `--runtime-override`, also:

| Attribute | Type | Meaning |
| --- | --- | --- |
| `frame.runtime.override` | bool | `true`. |
| `frame.runtime.override_path` | string | The Wasm file that ran. |
| `frame.runtime.override_code_hash` | string | Hash of that Wasm. |
| `frame.runtime.chain_code_hash` | string | Hash of the chain's own runtime, which did not run. |
| `frame.runtime.override_version_differences` | string | Differences between the two runtime versions, if any. |

## Phase span attributes

| Attribute | Type | Meaning |
| --- | --- | --- |
| `frame.phase` | string | Phase kind: `initialization`, `extrinsic`, `post inherents (on_poll)`, `finalization`, or `no phase markers` for a recorded trace without them. |
| `frame.extrinsic.index` | int | Extrinsic number, for extrinsic phases. |
| `frame.extrinsic.call` | string | `Pallet.call`, for extrinsic phases. |
| `frame.phase.timing_available` | bool | `false` if the phase had no timed operations. Its bar then spans the whole block. |
| `frame.timing` | string | `timed replay`, or `no timing` if the phase had no timed operations. |
| `frame.storage.*` totals | int | See [Totals](#totals). |

## Runtime span attributes

| Attribute | Type | Meaning |
| --- | --- | --- |
| `frame.runtime.target` | string | The Rust module that reported the span, for example `frame_executive`. |
| `frame.proof.marginal_bytes` | int | On `apply_extrinsic` only: how many bytes the proof grew during the extrinsic. See [Proof bytes](../concepts/proof-bytes.md). |
| `frame.timing` | string | `timed replay`. |

## Storage operation attributes

| Attribute | Type | Meaning |
| --- | --- | --- |
| `db.system.name` | string | Always `substrate`. |
| `db.operation.name` | string | The operation, such as `get`. |
| `db.collection.name` | string | The pallet that owns the key. Otherwise `well known keys` for keys such as `:code`, `child trie` for child trie operations, `state` for whole-state operations, or `unknown`. |
| `frame.storage.item` | string | The storage item, such as `System::Account`. |
| `frame.storage.key` | string | The full raw key, in hex. |
| `frame.storage.map_keys` | string array | The decoded map keys, such as an SS58 address. |
| `frame.storage.key_complete` | bool | `true` if the whole key was decoded. |
| `frame.storage.key_truncated` | bool | Present and `true` if the key was cut. |
| `frame.storage.seq` | int | Position of this operation in the block, from 0. |
| `frame.storage.origin` | string | Where a read was answered: `cache`, `trie (already in proof)` or `trie (new)`. `unknown` for writes and other operations, or when not recorded. See [Cache or trie](../concepts/cache-or-trie.md). |
| `frame.storage.origin_raw` | string | The same origin as the JSON `origin` field. See [JSON output](json.md#a-storage-operation). Absent when not recorded. |
| `frame.storage.proof_bytes` | int | Bytes this operation added to the block's proof. |
| `frame.storage.duplicate_read` | bool | `true` if this key was already read in the same phase. |
| `frame.storage.value_len` | int | Length of the value read or written, in bytes. |
| `frame.storage.value_truncated` | bool | Present and `true` if the value was cut. |
| `frame.storage.child_trie` | string | The child trie, for child trie operations. |
| `frame.storage.child_trie_truncated` | bool | Present and `true` if the child trie name was cut. |
| `frame.extrinsic.index` | int | The extrinsic this operation belongs to, if any. |
| `frame.timing` | string | `timed replay`, or `no timing` for a recorded `state_traceBlock` file. |

## Totals

Block, call and phase spans carry these counts for everything inside them:

| Attribute | Meaning |
| --- | --- |
| `frame.storage.ops` | Storage operations. |
| `frame.storage.reads` | Reads. |
| `frame.storage.writes` | Writes. |
| `frame.storage.distinct_keys` | Different keys touched. |
| `frame.storage.bytes_read` | Sum of value lengths read, in bytes. |
| `frame.storage.bytes_written` | Sum of value lengths written, in bytes. |
| `frame.storage.duplicate_reads` | Reads of a key already read in the same phase. |
| `frame.storage.truncated_keys` | Keys that were cut. |
