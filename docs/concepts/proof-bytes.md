# Proof bytes

The **storage proof** of a block is the set of trie nodes that you need to run the block again without the full state.
polkadot-frame-trace records those nodes during the replay.
After each replay, it prints `execution proof`, an estimate of their encoded size:

```text
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 6586 bytes
```

**Proof bytes** divide that total among the storage operations.
Each operation gets the bytes that it added to the proof.

## How the bytes are assigned

- The first operation that uses a trie node gets its bytes.
- Later operations that use the same node get 0 bytes.
  That is why a [trie (already in proof)](cache-or-trie.md) read usually shows 0.
- A [cache](cache-or-trie.md) read does not use the trie, so it always shows 0.
- If work between storage calls makes the proof larger, the next storage call gets those bytes.

Thus the proof bytes of all operations add up to the execution proof.
`summary` prints the check:

```text
Duplicate reads 203 = cache 177, trie (already in proof) 26, trie (new) 0, unknown 0; proof 6586 B (518 operations with proof data; 0 B not assigned to operations)
```

`518 operations with proof data` means every operation has a proof-bytes value.
`0 B not assigned to operations` means the per-operation values add up to the whole 6,586 bytes.

## Where you see proof bytes

| Where | Name | Unit |
| --- | --- | --- |
| Jaeger, on each storage operation | `frame.storage.proof_bytes` | bytes |
| Jaeger, on each `apply_extrinsic` runtime span | `frame.proof.marginal_bytes` | bytes the proof grew during that extrinsic |
| `summary` and `report` tables | `proof bytes` column | bytes |
| JSON | `proof_bytes_added` on each operation, `execution.proof_bytes` for the block | bytes |

## What proof bytes do not mean

- The execution proof is not the proof of validity (PoV) of the block.
  The PoV of a parachain also holds the block, and collators send a smaller, compressed form of the proof.
  The number from polkadot-frame-trace is an estimate of the proof before those steps.
  Use it to compare blocks and to find the largest phases and items. Do not use it as the exact PoV size.
- They are not the size of a proof for one item or one extrinsic on its own.
  The item that reads a node first gets its bytes, also when a later item needs the node.
  The order is important: if you move an extrinsic earlier, its numbers change.
- They are not the block's weight, and not network traffic.
- The execution proof covers the storage the block touched.
  It does not include the runtime code or the metadata polkadot-frame-trace loaded to decode names.

## When the sum is larger than the proof

A storage transaction that is reverted may remove trie nodes from the proof.
If a later operation needs those nodes again, it gets their bytes again.
Then the sum for all operations is larger than the execution proof.
polkadot-frame-trace prints a warning that starts with
`warning: proof bytes assigned to operations exceed the execution proof by <bytes> B`.

Use the execution proof for the block total in that case.
The summary line then ends with `<bytes> B counted more than once`.

See [Measure proof size](../guides/measure-proof-size.md) for a worked example.
