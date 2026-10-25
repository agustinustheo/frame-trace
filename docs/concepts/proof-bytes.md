# Proof bytes

A block's **storage proof** is the set of trie nodes needed to re-run the block without the full state.
frame-trace records those nodes while it replays the block.
The `execution proof` it prints after each replay is an estimate of their encoded size:

```text
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 6586 bytes
```

**Proof bytes** split that total across the storage operations that caused it.
Each operation is charged the bytes it added to the proof.

## How the bytes are assigned

- The first operation that touches a trie node pays for it.
- Later operations that use the same node pay nothing.
  That is why a [trie (already in proof)](cache-or-trie.md) read usually shows 0.
- A [cache](cache-or-trie.md) read never touches the trie, so it always shows 0.
- Work between storage calls that grows the proof is charged to the next storage call.

So the proof bytes of all operations add up to the execution proof.
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

- The execution proof is not the block's proof of validity (PoV).
  A parachain's PoV also holds the block itself, and collators send a compacted, compressed form of the proof.
  frame-trace's number is an estimate of the recorded proof before that.
  Use it to compare blocks and to find which phases and items dominate, not as the exact PoV size.

- They are not the size of a proof for one item or one extrinsic on its own.
  An item that reads a node first pays for it, even if a later item needs it too.
  Order matters: move an extrinsic earlier and its numbers change.
- They are not the block's weight, and not network traffic.
- The execution proof covers the storage the block touched.
  It does not include the runtime code or the metadata frame-trace loaded to decode names.

## When the sum is larger than the proof

A storage transaction that rolls back can drop trie nodes from the proof.
If a later operation needs them again, it pays for them again.
Then the per-operation sum is larger than the execution proof.
frame-trace prints a warning that starts with
`warning: proof bytes assigned to operations exceed the execution proof by <bytes> B`.

Use the execution proof for the block total in that case.
The summary line then ends with `<bytes> B counted more than once`.

See [Measure proof size](../guides/measure-proof-size.md) for a worked example.
