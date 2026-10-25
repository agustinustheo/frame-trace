# Timings

Every time in frame-trace comes from your machine, during the timed replay.
The timed replay makes no network calls, so no time is spent waiting for the node.

## What the times include

- The runtime running the block in Wasm on your CPU.
- The storage calls, answered from the proofs frame-trace already checked.
- The cost of recording the trace itself.

## What the times do not mean

- They are not how long the node took to import the block.
- They are not the node's database speed. frame-trace reads from memory, not from a database.
- They are not the block's weight. Weights come from benchmarks on reference hardware.

Use timings to compare parts of the same block, or the same call across blocks on the same machine.
Do not compare them with weights or with numbers from another machine.

## The time columns

| Column | Meaning |
| --- | --- |
| `storage time µs` | Time spent inside storage calls in this row, in microseconds. Overlapping calls count once. Compute between storage calls is not included. |
| `wall time µs` | Time from the start to the end of this phase, in microseconds. Comes from the runtime's own spans when it was built with tracing. Otherwise it runs from the first to the last storage call. `?` means unknown. |
| `span time µs` | Time inside a hook span, in the **Hooks** table. |
| `storage time %` | This phase's share of the block's storage time. |

The `total` row's wall time covers the whole replay, including the runtime starting up.
That is why it is larger than the sum of the phases.

## In Jaeger

Each span's bar length is its duration in the timed replay.
`frame.timing = timed replay` marks spans with real times.

## Recorded traces have no times

`--from-file` reads a saved `state_traceBlock` response, which has no timing.
frame-trace spaces those operations evenly, one millisecond apart, so the tree stays readable.
Those spans carry `frame.timing = no timing`.
Ignore their durations.
