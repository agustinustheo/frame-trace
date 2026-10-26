# Follow new blocks

`frame-trace follow` traces each new final block as it arrives and sends it to your trace viewer.
Leave it running next to Jaeger to watch a chain's storage work live.

## 1. Start following

Start Jaeger as in [Your first trace](../getting-started/first-trace.md), then run:

```sh
frame-trace follow --rpc wss://westend-asset-hub-rpc.polkadot.io --otlp --cache-dir .frame-trace-cache
```

Press Ctrl-C to stop.
Here is output from a 120-second run. Trace export was disabled when measuring these times.

```text
replay #18615397: fetching state (shared limit: 16 concurrent RPC requests)
...
root check: PASS; RPC calls: 92 (timed replay: 0); execution proof: 92281 bytes
fetching state #18615397: 63 proof request rounds; 92 RPC calls
state root: computed 0x7dd23e635d4db00b0067ceec74d8b67b1c5c3627014eaea3b59488352a4f86df; header 0x7dd23e635d4db00b0067ceec74d8b67b1c5c3627014eaea3b59488352a4f86df
traced finalized block #18615397 0xcdaa5219de6f7405d6189e42489cb0e2ffe6b77d042f6ffed70c4df54f5a1165: 550 storage operations in 36.737s
...
follow: queued=2 in-flight=8 done=48 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
...
follow shutdown: skipped=0 cancelled=10
follow stopped
```

The first block took 37 seconds. After startup, the median was 12 seconds per block in this run.
Each time covers loading state and replaying it on this machine.
It does not include time spent waiting for the first replay.
Each traced block appears in Jaeger as its own trace.

## 2. Read the status line

frame-trace checks the node for new final blocks every two seconds and prints a status line:

| Field | Meaning |
| --- | --- |
| `queued` | Blocks waiting to be replayed. |
| `in-flight` | Blocks assigned to workers, including those waiting for the first replay. |
| `done` | Blocks traced and exported. |
| `retrying` | Blocks that failed for a passing reason, such as a dropped connection, and will be tried again. |
| `stopped` | Blocks that failed for good, such as pruned state or a failed root check. |
| `reverted` | Blocks that left the chain in a reorg. Only with `--best`. |
| `skipped` | Blocks dropped because newer blocks arrived first. |
| `cancelled` | Blocks still waiting when you pressed Ctrl-C. |

## 3. Keep up with the chain

frame-trace replays several blocks at once. Raise `--workers` when the queue keeps growing.
See the [worker and queue limits](../reference/commands.md#follow).
The first replay runs on its own. Other workers wait until it finishes.
If it fails, the next block also runs on its own.

If replays remain slower than the chain, older blocks are skipped.
A full queue makes `follow` sample recent blocks at replay speed; it does not promise every block.
For a complete range, use [`report`](analyse-many-blocks.md) instead.

These log lines tell you this is happening:

| Line | Meaning |
| --- | --- |
| `follow queue full; sampling near the tip at replay speed` | The waiting queue is full. frame-trace stops queueing older blocks and prefers recent heights. |
| `replay job skipped: queued block expired behind tip` | A waiting block fell outside the recent-height limit. frame-trace dropped it without replaying it. It counts as `skipped`. |
| `skipped block <hash>: retry expired behind tip` | The same, for a block that was waiting to be retried. |

These lines mean replays are falling behind the chain.

To go faster:

- keep `--cache-dir`, so each block fetches only what changed;
- raise `--workers` to replay more blocks at once;
- raise `--proof-concurrency` on a fast connection to send more requests at once;
- use a node close to you.

## 4. Follow the newest blocks with `--best`

`--best` follows the newest blocks instead of final ones.
These can still be replaced by a reorg.
When that happens, frame-trace logs the reorg and counts the replaced block as `reverted`.
If a reverted block returns to the chain, it can be traced again.

## Retries

A request that fails for a passing reason, such as a timeout, a dropped connection or a rate limit, is retried.
A block gets at most four attempts.
Pruned state, an invalid block, a failed root check or a crash in the replay stop that block for good.
The log then says `stopped retrying block <hash> after <N> attempt(s)`.
