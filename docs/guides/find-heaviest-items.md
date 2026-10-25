# Find the heaviest storage items

"Heaviest" can mean three things: most [proof bytes](../concepts/proof-bytes.md), most storage operations, or most storage time.
`summary` ranks one block by proof bytes.
`report` ranks one or many blocks by any of the three.

## Rank by proof bytes

Run this from the `frame-trace` directory:

```sh
frame-trace summary --offline fixtures/replay/people-traced --top 5
```

```text
...
Top items by proof bytes
┌───────────────────────────────────────┬──────────────┬─────────────────────────┬────────────┬─────────┬────────┬────────────┬────────────┬───────────────┬─────────────────┬─────────────┐
│ item                                  ┆ reads: cache ┆ trie (already in proof) ┆ trie (new) ┆ unknown ┆ writes ┆ iterations ┆ bytes read ┆ bytes written ┆ duplicate reads ┆ proof bytes │
╞═══════════════════════════════════════╪══════════════╪═════════════════════════╪════════════╪═════════╪════════╪════════════╪════════════╪═══════════════╪═════════════════╪═════════════╡
│ System::EventTopics                   ┆            0 ┆                       0 ┆          0 ┆       0 ┆      1 ┆          1 ┆          0 ┆             0 ┆               0 ┆        1120 │
│ <unknown>                             ┆            0 ┆                       0 ┆          0 ┆       0 ┆      0 ┆          0 ┆          0 ┆             0 ┆               0 ┆         571 │
│ ParachainSystem::DidSetValidationCode ┆            0 ┆                       0 ┆          1 ┆       0 ┆      1 ┆          0 ┆          0 ┆             0 ┆               0 ┆         432 │
│ System::Account                       ┆           52 ┆                      20 ┆          2 ┆       0 ┆      9 ┆          0 ┆       5200 ┆           720 ┆              69 ┆         332 │
│ Aura::Authorities                     ┆            0 ┆                       2 ┆          1 ┆       0 ┆      0 ┆          0 ┆         99 ┆             0 ┆               1 ┆         323 │
└───────────────────────────────────────┴──────────────┴─────────────────────────┴────────────┴─────────┴────────┴────────────┴────────────┴───────────────┴─────────────────┴─────────────┘
… 69 more
...
```

`… 69 more` means 69 items did not fit in `--top 5`.

## What each column means

| Column | Meaning |
| --- | --- |
| `item` | The storage item, as `Pallet::Item`. `<unknown>` holds operations with no named key, such as computing the state root. Keys starting with `:` are well-known keys, such as `:code`. |
| `reads: cache` | Reads answered from the block's own changes. See [Cache or trie](../concepts/cache-or-trie.md). |
| `trie (already in proof)` | Reads that went to the chain state and added no new trie nodes. |
| `trie (new)` | Reads that added new trie nodes to the proof. |
| `unknown` | Reads with no origin recorded. |
| `writes` | Writes, including deletes and appends. |
| `iterations` | Times the runtime stepped through keys, for example to delete a prefix. |
| `bytes read`, `bytes written` | Sum of the value lengths, in bytes. A cache read still counts its bytes. |
| `duplicate reads` | Reads of a key already read in the same phase. See [Find repeated reads](find-repeated-reads.md). |
| `proof bytes` | Bytes this item's operations added to the block's proof. |

## What the example shows

- `System::EventTopics` added the most proof bytes, 1,120, with no reads at all.
  Initialization deletes this prefix, and walking the keys to delete them pulls trie nodes into the proof.
- `System::Account` was read 74 times: 52 from cache, 20 from the trie already in proof, 2 new.
  Only the two new reads added proof bytes, 166 each. The other reads found their trie path already in the proof.

## Rank by operations or time

`report` takes saved JSON from `block --json` and has `--sort-by`:

```sh
frame-trace block --offline fixtures/replay/people-traced --json block.json
frame-trace report block.json --sort-by ops --top 5
```

```text
...
Top items by operations
┌─────────────────────┬──────────────┬─────────────────────────┬────────────┬─────────┬────────┬────────────┬────────────┬───────────────┬─────────────────┬─────────────┐
│ item                ┆ reads: cache ┆ trie (already in proof) ┆ trie (new) ┆ unknown ┆ writes ┆ iterations ┆ bytes read ┆ bytes written ┆ duplicate reads ┆ proof bytes │
╞═════════════════════╪══════════════╪═════════════════════════╪════════════╪═════════╪════════╪════════════╪════════════╪═══════════════╪═════════════════╪═════════════╡
│ System::BlockWeight ┆           60 ┆                       0 ┆          0 ┆       0 ┆     58 ┆          0 ┆        724 ┆           692 ┆              54 ┆           0 │
│ System::Account     ┆           52 ┆                      20 ┆          2 ┆       0 ┆      9 ┆          0 ┆       5200 ┆           720 ┆              69 ┆         332 │
│ System::EventCount  ┆           15 ┆                       0 ┆          0 ┆       0 ┆     16 ┆          0 ┆         56 ┆            60 ┆              12 ┆           0 │
│ :transaction_level: ┆           11 ┆                       0 ┆          1 ┆       0 ┆     12 ┆          0 ┆         36 ┆            36 ┆               9 ┆          39 │
│ System::Number      ┆           21 ┆                       0 ┆          1 ┆       0 ┆      1 ┆          0 ┆         88 ┆             4 ┆              16 ┆          48 │
└─────────────────────┴──────────────┴─────────────────────────┴────────────┴─────────┴────────┴────────────┴────────────┴───────────────┴─────────────────┴─────────────┘
… 69 more
...
```

`System::BlockWeight` has 118 operations but 0 proof bytes.
Every read of it came from the cache, because the block keeps updating it.
Use `--sort-by time` to rank by storage time instead.

## Rank extrinsics

The same commands rank extrinsic types:

```text
Extrinsic distributions (ranked by total proof bytes)
┌─────────────────────────────────────┬───────┬───────────────────────┬────────────────────────┬────────────────────────────┐
│ pallet.call                         ┆ count ┆ operations median/p95 ┆ proof bytes median/p95 ┆ storage time µs median/p95 │
╞═════════════════════════════════════╪═══════╪═══════════════════════╪════════════════════════╪════════════════════════════╡
│ Utility.batch                       ┆     1 ┆           200.0/200.0 ┆            827.0/827.0 ┆                492.3/492.3 │
│ ParachainSystem.set_validation_data ┆     1 ┆             54.0/54.0 ┆            495.0/495.0 ┆                142.5/142.5 │
│ Timestamp.set                       ┆     1 ┆             26.0/26.0 ┆            139.0/139.0 ┆                  33.9/33.9 │
└─────────────────────────────────────┴───────┴───────────────────────┴────────────────────────┴────────────────────────────┘
```

`count` is how many extrinsics of that type the input had.
The other columns give the median and the 95th percentile per extrinsic.
With many blocks, these show which calls cost the most storage in a typical case and a bad case.
See [Analyse many blocks](analyse-many-blocks.md).
