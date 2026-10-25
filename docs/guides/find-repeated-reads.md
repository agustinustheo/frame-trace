# Find repeated reads

A repeated read is a read of a key the same phase already read.
It is not an error.
Many repeats are cheap [cache](../concepts/cache-or-trie.md) reads.
Repeats that go to the [trie](../concepts/cache-or-trie.md) cost more, and they are worth a look.

This guide uses block #4 of the People fixture.
Its extrinsic 2 is a `Utility.batch` of three `transfer_keep_alive` calls from Alice.

## 1. Print the tables

Run this from the `frame-trace` directory.
`--top 5` keeps five rows per phase.

```sh
frame-trace summary --offline fixtures/replay/people-traced --top 5
```

Against a live chain, use `--rpc <url> --block <block>` instead of `--offline`.

```text
1 block(s); 518 operations; reads 312 = cache 231 + trie 81 (already in proof 47, new 34) + unknown 0
Duplicate reads 203 = cache 177, trie (already in proof) 26, trie (new) 0, unknown 0; proof 6586 B (518 operations with proof data; 0 B not assigned to operations)
...
Duplicate reads per phase (top 5 each)
┌──────────────────────────────────────────────────┬───────────────────────────────────────────────────────────────────┬─────────────────┬───────┬─────────────────────────┬────────────┬─────────┐
│ phase                                            ┆ key                                                               ┆ duplicate reads ┆ cache ┆ trie (already in proof) ┆ trie (new) ┆ unknown │
╞══════════════════════════════════════════════════╪═══════════════════════════════════════════════════════════════════╪═════════════════╪═══════╪═════════════════════════╪════════════╪═════════╡
...
│ extrinsic 2: Utility.batch                       ┆ System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY) ┆              47 ┆    42 ┆                       5 ┆          0 ┆       0 │
│ extrinsic 2: Utility.batch                       ┆ System::Number                                                    ┆              15 ┆    15 ┆                       0 ┆          0 ┆       0 │
│ extrinsic 2: Utility.batch                       ┆ System::Account(5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty) ┆              15 ┆     8 ┆                       7 ┆          0 ┆       0 │
│ extrinsic 2: Utility.batch                       ┆ System::ExecutionPhase                                            ┆              12 ┆    12 ┆                       0 ┆          0 ┆       0 │
│ extrinsic 2: Utility.batch                       ┆ System::EventCount                                                ┆              12 ┆    12 ┆                       0 ┆          0 ┆       0 │
│ extrinsic 2: Utility.batch                       ┆ … 11 more                                                         ┆                 ┆       ┆                         ┆            ┆         │
...
└──────────────────────────────────────────────────┴───────────────────────────────────────────────────────────────────┴─────────────────┴───────┴─────────────────────────┴────────────┴─────────┘
```

## 2. Read the summary lines

- `reads 312`: the block made 312 reads in total.
- `cache 231 + trie 81`: 231 came from the block's own changes in memory. 81 went to the chain state.
- `already in proof 47, new 34`: of the trie reads, 34 added new nodes to the proof.
- `Duplicate reads 203`: 203 reads were repeats within their phase.
  177 of them were cache reads, and 26 went to the trie again.

## 3. Read the table

Each row is one key in one phase.
`duplicate reads` counts the repeats, so the first read of the key is not in it.
The next columns split the repeats by origin.

## Example: Alice's account, read 48 times

The top row says Alice's account, `System::Account(5Grw…utQY)`, has 47 repeats in the batch.
With the first read, that is 48 reads of the same key in one extrinsic:

| Reads | Origin | What happened |
| ---: | --- | --- |
| 1 | trie (new) | The first read. The runtime walked the trie to Alice's entry and added those nodes to the proof. |
| 5 | trie (already in proof) | Five more reads before anything wrote her account. Each walked the trie again but added 0 proof bytes. |
| 42 | cache | Her fee was taken, which wrote her account. Every read after that came from memory. |

Only the first read added proof bytes: 166 of them.
The other 47 were repeats: 5 extra trie walks and 42 memory reads.

Bob's row tells a different story: 15 repeats, 8 from cache and 7 from the trie.
Bob is only read until the first transfer writes his balance.
Until then, every read of his account walks the trie again.
Reading a key does not put it in the cache. Only a write does.

## 4. See the reads in order

The table counts. Jaeger shows the order.
Send the block to Jaeger and open `extrinsic 2: Utility.batch`:

```sh
frame-trace block --offline fixtures/replay/people-traced --otlp
```

Click each `get System::Account(…)` row and check `frame.storage.origin` and `frame.storage.duplicate_read`.
[Reading a trace](../getting-started/reading-a-trace.md) shows where they are.

## What to look for

- **Repeats from the trie** in your own pallet. Each one walks the trie again. Reading once into a local variable avoids them.
- **Many cache repeats of one key**, such as `System::BlockWeight` 49 times in initialization. Each is a host call, even if cheap.
- **The same key in many phases.** The table only counts repeats within one phase. Use `report` to see totals per item across the block, see [Find the heaviest storage items](find-heaviest-items.md).
