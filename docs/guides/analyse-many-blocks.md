# Analyse many blocks

One block can mislead.
`frame-trace report` adds up many blocks and shows where storage work and [proof bytes](../concepts/proof-bytes.md) go in general.

## 1. Report on the latest blocks

This replays the last five final blocks of Westend Asset Hub:

```sh
frame-trace report --rpc wss://westend-asset-hub-rpc.polkadot.io --last 5 --top 5 \
  --cache-dir .frame-trace-cache --json range.json
```

The progress lines for each block come first, then the tables:

```text
replay #18596964: fetching state (shared limit: 8 concurrent RPC requests)
replay #18596963: fetching state (shared limit: 8 concurrent RPC requests)
replay #18596963: loading runtime and preparing state
replay #18596964: loading runtime and preparing state
root check: PASS; RPC calls: 85 (timed replay: 0); execution proof: 90290 bytes
fetching state #18596963: 58 proof request rounds; 85 RPC calls
state root: computed 0x58471fed6c283d960865ad3ba6b54bf2256c00c8737f66d8f74e96f2bb515598; header 0x58471fed6c283d960865ad3ba6b54bf2256c00c8737f66d8f74e96f2bb515598
...
5 block(s); 2756 operations; reads 1718 = cache 701 + trie 1017 (already in proof 566, new 451) + unknown 0
Duplicate reads 882 = cache 489, trie (already in proof) 393, trie (new) 0, unknown 0; proof 450641 B (2756 operations with proof data; 0 B not assigned to operations)

Phase shares
┌─────────────────────────────────┬──────────────┬────────────────┬─────────┬─────────────┐
│ phase                           ┆ operations % ┆ storage time % ┆ proof % ┆ proof bytes │
╞═════════════════════════════════╪══════════════╪════════════════╪═════════╪═════════════╡
│ initialization                  ┆         40.3 ┆           20.2 ┆    16.0 ┆       72157 │
│ inherents (required extrinsics) ┆         13.8 ┆           18.5 ┆     0.9 ┆        4070 │
│ post inherents (on_poll)        ┆          2.2 ┆            1.2 ┆     0.2 ┆         720 │
│ finalization                    ┆         43.7 ┆           60.1 ┆    82.9 ┆      373694 │
└─────────────────────────────────┴──────────────┴────────────────┴─────────┴─────────────┘

Top items by proof bytes
┌──────────────────────┬──────────────┬─────────────────────────┬────────────┬─────────┬────────┬────────────┬────────────┬───────────────┬─────────────────┬─────────────┐
│ item                 ┆ reads: cache ┆ trie (already in proof) ┆ trie (new) ┆ unknown ┆ writes ┆ iterations ┆ bytes read ┆ bytes written ┆ duplicate reads ┆ proof bytes │
╞══════════════════════╪══════════════╪═════════════════════════╪════════════╪═════════╪════════╪════════════╪════════════╪═══════════════╪═════════════════╪═════════════╡
│ VoterList::ListNodes ┆            0 ┆                     150 ┆         60 ┆       0 ┆     50 ┆          0 ┆      12540 ┆          5700 ┆             150 ┆       91919 │
│ Staking::Ledger      ┆            0 ┆                       0 ┆         50 ┆       0 ┆      0 ┆          0 ┆       2350 ┆             0 ┆               0 ┆       77022 │
│ Staking::Bonded      ┆            0 ┆                      50 ┆         50 ┆       0 ┆      0 ┆          0 ┆       3200 ┆             0 ┆              50 ┆       74952 │
│ Staking::Nominators  ┆            0 ┆                       0 ┆         50 ┆       0 ┆      0 ┆          0 ┆          0 ┆             0 ┆               0 ┆       73397 │
│ System::Account      ┆            0 ┆                       0 ┆         20 ┆       0 ┆      0 ┆          0 ┆       1200 ┆             0 ┆               0 ┆       40575 │
└──────────────────────┴──────────────┴─────────────────────────┴────────────┴─────────┴────────┴────────────┴────────────┴───────────────┴─────────────────┴─────────────┘
… 94 more

Extrinsic distributions (ranked by total proof bytes)
┌─────────────────────────────────────┬───────┬───────────────────────┬────────────────────────┬────────────────────────────┐
│ pallet.call                         ┆ count ┆ operations median/p95 ┆ proof bytes median/p95 ┆ storage time µs median/p95 │
╞═════════════════════════════════════╪═══════╪═══════════════════════╪════════════════════════╪════════════════════════════╡
│ ParachainSystem.set_validation_data ┆     5 ┆             49.0/51.0 ┆            814.0/814.0 ┆               980.4/1063.6 │
│ Timestamp.set                       ┆     5 ┆             26.0/26.0 ┆                0.0/0.0 ┆                  88.0/93.6 │
└─────────────────────────────────────┴───────┴───────────────────────┴────────────────────────┴────────────────────────────┘
```

It took 38 seconds.
The first two blocks needed 85 and 87 RPC calls.
Later blocks needed about 40, because `--cache-dir` kept state the earlier blocks fetched.

## 2. Read the result

- **Finalization dominates.** It made 82.9% of the proof bytes and 60.1% of the storage time in these five blocks.
  No signed extrinsic was in them, so this is the cost of an empty block.
- **Staking data is the reason.** `VoterList::ListNodes`, `Staking::Ledger`, `Staking::Bonded` and `Staking::Nominators` lead the item table.
  Each block reads 10 new ledgers and nominators, and those reads pull new trie nodes into the proof.
- **Repeats go to the trie.** `VoterList::ListNodes` was read 150 times from the trie already in proof.
  No write came first, so none could come from the cache.
  See [Find repeated reads](find-repeated-reads.md).
- **Extrinsics** shows the median and the 95th percentile per extrinsic type.
  `Timestamp.set` added 0 proof bytes in every block: earlier phases had already touched its nodes.

Your numbers will differ: the chain moves on.

## 3. Choose the blocks

| Input | Example |
| --- | --- |
| The last K final blocks | `--rpc <url> --last 10` |
| A range of final blocks | `--rpc <url> --from 18563569 --to 18563608` |
| Saved `block --json` files | `report a.json b.json`, or `--from-json a.json` repeated |

A range can hold up to 10,000 blocks.
It must start after the genesis block and end at or before the latest final block.
Saved files need no network.

## 4. Tune it

| Flag | Effect |
| --- | --- |
| `--top N` | Rows per table in the terminal. JSON always has every row. |
| `--sort-by proof\|ops\|time` | Rank items, extrinsics and hooks by proof bytes, operations or storage time. |
| `--concurrency N` | Blocks replayed at once, 1 to 4. |
| `--proof-concurrency N` | Proof requests at once, shared by all blocks. |
| `--cache-dir DIR` | Keep fetched state and compiled runtimes between blocks and runs. |
| `--json FILE` | Write the full report as JSON. `-` writes it to stdout and the tables to stderr. |

## When a block fails

`report` keeps going.
Failed blocks and files are listed in the output and in the JSON `errors` field.
The command then exits with code 1.
A block hash that appears twice counts as a failed input.

## Mixing chains

`report` groups by pallet and call name.
If you mix blocks from two chains that share names, their numbers are added together.
