# Real block fixture

Copied from `fixtures/next-people-paseo-traced-official/1/` (recorded 2026-10-24 19:13 UTC by the
experiment-e1 agent) so tests do not depend on that directory.

| File | Source |
| --- | --- |
| `people-paseo-block-1-trace-state.json` | `state_traceBlock(<block 1>, "state", "", "")` response |
| `people-paseo-block-1-metadata.hex` | `state_getMetadata` at that block (hex, V14) |

Chain: `next-people-paseo` (parachain), spec version 3004000, block 1
(`0xb306207327e28fc1c256bb29e6fa838d0dcd790670e9aaf87a6bbf9bca6d5719`). The block has two
inherents (`ParachainSystem.set_validation_data`, `Timestamp.set`) and no signed extrinsic, so
`inherents_applied` (`on_poll`) runs right before finalization. 315 storage ops.
