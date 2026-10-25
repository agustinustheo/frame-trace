# Phases of a block

A **phase** is one part of a block's execution.
frame-trace puts every storage operation in exactly one phase.
In Jaeger every phase is a child of the block span, except `post inherents (on_poll)`.
That phase sits inside the first extrinsic that is not an inherent, when the block has one.
When the block has only inherents, it is a top-level phase of its own, just before finalization.
In the tables each phase is a row.

## The phases, in order

| Phase name | What runs |
| --- | --- |
| `initialization` | Block setup and every pallet's `on_initialize` hook. |
| `extrinsic N: Pallet.call` | One extrinsic, numbered from 0 in block order, for example `extrinsic 2: Utility.batch`. |
| `post inherents (on_poll)` | Hooks such as `on_poll` that run once the inherents are done. They sit inside the first non-inherent extrinsic, or stand alone before finalization when there is none. See below. |
| `finalization` | `on_idle` hooks, when weight remains and no multi-block migration is running, then every pallet's `on_finalize` hook, then the final checks. |

Here is block #4 of the People fixture as `summary` prints it:

```text
┌──────────────────────────────────────────────────┬────────────┬───────┬────────┬─────────────┬─────────────────┬──────────────┐
│ phase                                            ┆ operations ┆ reads ┆ writes ┆ proof bytes ┆ storage time µs ┆ wall time µs │
╞══════════════════════════════════════════════════╪════════════╪═══════╪════════╪═════════════╪═════════════════╪══════════════╡
│ initialization                                   ┆        152 ┆    74 ┆     77 ┆        3490 ┆           272.5 ┆        478.0 │
│ extrinsic 0: ParachainSystem.set_validation_data ┆         54 ┆    26 ┆     28 ┆         495 ┆           148.7 ┆        267.8 │
│ extrinsic 1: Timestamp.set                       ┆         26 ┆    14 ┆     12 ┆         139 ┆            37.0 ┆         52.8 │
│ extrinsic 2: Utility.batch                       ┆        200 ┆   146 ┆     54 ┆         827 ┆           517.3 ┆        743.8 │
│   └ post inherents (on_poll)                     ┆         14 ┆    12 ┆      2 ┆         686 ┆            30.8 ┆         37.1 │
│ finalization                                     ┆         72 ┆    40 ┆     31 ┆         949 ┆           262.7 ┆        440.7 │
│ total                                            ┆        518 ┆   312 ┆    204 ┆        6586 ┆          1269.0 ┆       4607.9 │
└──────────────────────────────────────────────────┴────────────┴───────┴────────┴─────────────┴─────────────────┴──────────────┘
```

## Why post inherents sit inside an extrinsic

The runtime runs the post-inherent hooks lazily.
They start when the first extrinsic that is not an inherent is applied, inside that extrinsic.
That extrinsic is usually signed, but it can be an unsigned one.
frame-trace shows the hooks nested under it, with `└`, and counts them separately.
The extrinsic's own row does not include them.
If a block has only inherents, post inherents is a phase of its own, as in most Westend Asset Hub blocks.

## Phase groups

When frame-trace adds up many blocks, it groups the extrinsic phases.
It sorts them by whether the extrinsic is signed, read from the extrinsic's bytes.
That is an approximation of inherents and user transactions: an unsigned extrinsic that is not an inherent lands in the inherents group.
The group names appear in the **Phase shares** table:

| Group | Contains |
| --- | --- |
| `initialization` | The initialization phase. |
| `inherents (required extrinsics)` | Unsigned extrinsics, such as the inherents `Timestamp.set` and `ParachainSystem.set_validation_data`. |
| `signed extrinsics` | Extrinsics a user signed. |
| `extrinsics (signature unknown)` | Extrinsics whose bytes did not say whether they are signed. |
| `post inherents (on_poll)` | The post-inherent hooks. |
| `finalization` | The finalization phase. |

## Hooks

When the runtime is built with tracing, each hook is its own span inside its phase.
The span name is the hook and the pallet, for example `on_initialize: Balances`.
The **Hooks** table in `summary` and `report` ranks them.
Without tracing, hook storage operations sit directly in the phase.
See [See runtime calls inside extrinsics](../guides/see-runtime-calls.md).
