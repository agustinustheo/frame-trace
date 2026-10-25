# Measure proof size

On a parachain, every block ships a storage proof to the relay chain.
frame-trace tells you how big that proof is and which phases, items and extrinsics made it grow.
[Proof bytes](../concepts/proof-bytes.md) explains how the bytes are counted.

## 1. The whole block

Every replay prints the size of the block's storage proof:

```sh
frame-trace block --offline fixtures/replay/people-traced
```

```text
replay #4: loading runtime and preparing state
root check: PASS; RPC calls: 0 (timed replay: 0); execution proof: 6586 bytes
...
```

`execution proof: 6586 bytes` is the total for block #4.
A live Westend Asset Hub block in [Your first trace](../getting-started/first-trace.md) needed 90,250 bytes.

## 2. Per phase

```sh
frame-trace summary --offline fixtures/replay/people-traced --top 5
```

```text
Storage per phase
┌──────────────────────────────────────────────────┬────────────┬───────┬────────┬─────────────┬─────────────────┬──────────────┐
│ phase                                            ┆ operations ┆ reads ┆ writes ┆ proof bytes ┆ storage time µs ┆ wall time µs │
╞══════════════════════════════════════════════════╪════════════╪═══════╪════════╪═════════════╪═════════════════╪══════════════╡
│ initialization                                   ┆        152 ┆    74 ┆     77 ┆        3490 ┆           272.2 ┆        472.5 │
│ extrinsic 0: ParachainSystem.set_validation_data ┆         54 ┆    26 ┆     28 ┆         495 ┆           142.5 ┆        253.6 │
│ extrinsic 1: Timestamp.set                       ┆         26 ┆    14 ┆     12 ┆         139 ┆            33.9 ┆         48.3 │
│ extrinsic 2: Utility.batch                       ┆        200 ┆   146 ┆     54 ┆         827 ┆           492.3 ┆        710.0 │
│   └ post inherents (on_poll)                     ┆         14 ┆    12 ┆      2 ┆         686 ┆            29.3 ┆         35.0 │
│ finalization                                     ┆         72 ┆    40 ┆     31 ┆         949 ┆           251.3 ┆        415.9 │
│ total                                            ┆        518 ┆   312 ┆    204 ┆        6586 ┆          1221.5 ┆       4420.7 │
└──────────────────────────────────────────────────┴────────────┴───────┴────────┴─────────────┴─────────────────┴──────────────┘
...
Phase shares
┌─────────────────────────────────┬──────────────┬────────────────┬─────────┬─────────────┐
│ phase                           ┆ operations % ┆ storage time % ┆ proof % ┆ proof bytes │
╞═════════════════════════════════╪══════════════╪════════════════╪═════════╪═════════════╡
│ initialization                  ┆         29.3 ┆           22.3 ┆    53.0 ┆        3490 │
│ inherents (required extrinsics) ┆         15.4 ┆           14.4 ┆     9.6 ┆         634 │
│ signed extrinsics               ┆         38.6 ┆           40.3 ┆    12.6 ┆         827 │
│ post inherents (on_poll)        ┆          2.7 ┆            2.4 ┆    10.4 ┆         686 │
│ finalization                    ┆         13.9 ┆           20.6 ┆    14.4 ┆         949 │
└─────────────────────────────────┴──────────────┴────────────────┴─────────┴─────────────┘
```

- The `proof bytes` column adds up to the execution proof: 6,586 bytes.
- `proof %` is each group's share of it.
- Initialization added 53% of the proof in this block, more than the three-transfer batch.
  The first block hooks touch many items for the first time, so they pay for those trie nodes.

## 3. Per extrinsic

The **Extrinsic distributions** table gives the proof bytes per extrinsic type.
[Find the heaviest storage items](find-heaviest-items.md#rank-extrinsics) shows it.

In Jaeger, each `apply_extrinsic` runtime span has `frame.proof.marginal_bytes`: how much the proof grew during that extrinsic.
For the batch it is 1,513 bytes.
That is the batch's 827 bytes plus the 686 bytes of the post-inherent hooks that ran inside it.
Runtime spans need a runtime built with tracing, see [See runtime calls inside extrinsics](see-runtime-calls.md).

## 4. Per storage item and per read

- The **Top items by proof bytes** table ranks storage items.
- In Jaeger, every storage operation has `frame.storage.proof_bytes`.

## Reading the numbers

- An item pays for a trie node only if it is the first to touch it.
  Change the order of extrinsics and the split between them changes.
- Proof bytes are not weight, and they do not include the runtime code.
- For many blocks, use `report`. **Phase shares** then sums all blocks, and **Extrinsic distributions** shows the median and 95th percentile per extrinsic.
  See [Analyse many blocks](analyse-many-blocks.md).
