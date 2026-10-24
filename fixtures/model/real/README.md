# Real block fixtures

Blocks of a `next-people-paseo` parachain (spec version 3004000) recorded from a real node by the
experiment-e1 agent, copied here so tests do not depend on those directories.

| File | Source |
| --- | --- |
| `people-paseo-block-1-trace-state.json` | `fixtures/next-people-paseo-traced-official/1/trace-state.json` (2026-10-24 19:13 UTC) |
| `people-paseo-block-2-trace-state.json` | `fixtures/next-people-paseo/2/trace-state.json` (2026-10-24 19:25 UTC) |
| `people-paseo-block-1-metadata.hex` | `state_getMetadata` (hex, V14) from block 1 of the first chain; byte-identical to the metadata recorded for block 2 |

All traces are `state_traceBlock(<hash>, "state", "", "")` responses.

- Block 1 (`0xb306…5719`): two inherents (`ParachainSystem.set_validation_data`,
  `Timestamp.set`), no signed extrinsic, so `inherents_applied` (`on_poll`) runs right before
  finalization. 315 storage ops.
- Block 2 (`0x9fba…3b59`): the same inherents plus a signed `Balances.transfer_keep_alive`. Its
  `check` reads (ops 230..234) come before `inherents_applied` (234..248), which runs inside it.
  418 storage ops.
