# Follow new blocks

`frame-trace follow` traces each new final block as it arrives and sends it to your trace viewer.
Leave it running next to Jaeger to watch a chain's storage work live.

## 1. Start following

Start Jaeger as in [Your first trace](../getting-started/first-trace.md), then run:

```sh
frame-trace follow --rpc wss://westend-asset-hub-rpc.polkadot.io --otlp --cache-dir .frame-trace-cache
```

Press Ctrl-C to stop.
Here is output from a three-minute run that sent traces to a local OTLP receiver.

```text
follow: queued=1 in-flight=0 traced=0 exported=0 export-waiting=0 export-rejected=0 export-failed=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 up=2s
replay #19027339: fetching state (shared limit: 16 concurrent RPC requests)
replay #19027339: loading runtime and preparing state
root check: PASS; RPC calls: 55 (timed replay: 0); execution proof: 88836 bytes
fetching state #19027339: 47 proof request rounds; 55 RPC calls
state root: computed 0x09f37412eeb6a15e55daf44cb6a2c425ac273ffac4e4644601d5f72461f95e23; header 0x09f37412eeb6a15e55daf44cb6a2c425ac273ffac4e4644601d5f72461f95e23
traced finalized block #19027339 0x35147e1c3c8f1c54fa83dc651c4aa4ffb22450a6705b65f49accfad3406ddc49: 569 storage operations in 17.179s
...
traced finalized block #19027340 0x8e08a9667e62da3e8553b399d63f73a856879ff198ffdcc8ef5fdfb7bf786e5c: 569 storage operations in 6.164s
...
follow: queued=3 in-flight=3 traced=25 exported=25 export-waiting=0 export-rejected=0 export-failed=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 up=1m03s
...
follow: stopping (signal again to exit now)
replay job cancelled
replay job cancelled
replay job cancelled
follow stopped after 2m59s: traced=85 exported=85 export-rejected=0 export-failed=0 export-dropped=0 export-unsent=0 stopped=0 skipped=0 reverted=0 cancelled=3 poll-failures=0
```

The cache directory was kept from an earlier run, so the first block took 17 seconds.
The second took 6 seconds.
The three blocks still waiting on the node when the stop began count as `cancelled`.
Each time covers loading state and replaying it on this machine.
It does not include sending the trace, or time spent waiting for the first replay.
Each traced block appears in Jaeger as its own trace.

## 2. Read the status line

frame-trace checks the node for new final blocks and prints a status line.
It prints the line when a problem count changes, and otherwise at a fixed interval.
The [reference](../reference/commands.md#follow) lists the intervals.

| Field | Meaning |
| --- | --- |
| `queued` | Blocks waiting to be replayed. |
| `in-flight` | Blocks assigned to workers, including those waiting for the first replay. |
| `traced` | Blocks replayed since the start. |
| `exported` | Traces the receiver accepted. Only with `--otlp`. |
| `export-waiting` | Traces waiting to be sent. Only with `--otlp`. |
| `export-rejected` | Traces the receiver refused in part. They are not sent again. Only with `--otlp`. |
| `export-failed` | Traces whose export crashed inside frame-trace. They are not sent again. Only with `--otlp`. |
| `export-dropped` | Traces dropped because too many were waiting. Only with `--otlp`. |
| `retrying` | Blocks that failed for a passing reason, such as a dropped connection, and will be tried again. |
| `stopped` | Blocks that failed for good, such as pruned state or a failed root check. |
| `skipped` | Blocks not replayed, or not retried, because newer blocks arrived first. |
| `reverted` | Blocks that left the chain in a reorg, counted once whatever their state. Only with `--best`. |
| `up` | Time since `follow` started. |

All counts except `queued`, `in-flight`, `retrying` and `export-waiting` are totals since the start.

When you stop `follow`, the last line sums up the run.
It adds `cancelled` and `poll-failures`, the failed checks for new blocks.
`cancelled` counts blocks that were waiting, waiting on the node, or still running when the stop wait ended.
With `--otlp`, `export-unsent` counts traces not yet sent when the stop ended.

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

- keep `--cache-dir`, so each block asks the node only for state that changed;
- raise `--workers` to replay more blocks at once;
- raise `--proof-concurrency` on a fast connection to send more requests at once;
- use a node close to you.

## 4. Follow the newest blocks with `--best`

`--best` follows the newest blocks instead of final ones.
These can still be replaced by a reorg.
When that happens, frame-trace logs the reorg and counts the replaced block as `reverted`.
If a reverted block returns to the chain, it can be traced again.

## 5. When the node or the receiver goes away

If a check for new blocks fails, frame-trace logs it and tries again.
It waits longer after each failure, and repeats the line at most once a minute.

```text
follow poll failed: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 2s
follow poll still failing after 6 attempts over 1m06s: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 1m00s
```

`follow` keeps running until the node answers again, then logs `follow poll recovered after <N> failed attempt(s) over <time>`.

If the OTLP receiver stops, frame-trace keeps replaying blocks and holds their traces.
It sends them when the receiver is back. It does not replay the blocks again.
Each trace goes out in several requests. A request the receiver acknowledged is never sent again.
If an acknowledgement is lost, the receiver can get the same spans twice, with the same IDs.

In the run from step 1, the receiver refused one request with a 400, then was stopped for 30 seconds:

```text
OTLP export failed (1 attempt(s) in a row): 63 of 575 spans were not exported (first error: Operation failed: HTTP export failed with status code: 400); retrying in 1s, 1 trace(s) waiting
OTLP export recovered after 1 failed attempt(s) over 1s; 0 trace(s) waiting
...
OTLP export failed (2 attempt(s) in a row): 575 of 575 spans were not exported (first error: Operation failed: HTTP export failed: network error); retrying in 2s, 9 trace(s) waiting
OTLP export recovered after 3 failed attempt(s) over 27s; 17 trace(s) waiting
```

The first line comes from the refused request: the trace's first request was accepted, so only the other 63 spans were sent again.
The first failure of the outage was not logged, because a failure line had been printed less than a minute before.

Each try can take several seconds, because the exporter itself retries until the OTLP timeout.
So the time in the recovery line is longer than the waits between tries add up to.

A long outage fills the waiting queue. frame-trace then drops the oldest trace and counts it as `export-dropped`.
The [reference](../reference/commands.md#follow) lists the queue size.
If the receiver is still down when you stop `follow`, the waiting traces are not sent.
The summary counts them as `export-unsent`.

## 6. Run it as a service

`follow` stops cleanly on `SIGTERM` and `SIGHUP`, as on Ctrl-C. `SIGHUP` does not reload anything.
Replays waiting on the node end at once. Replays running on the CPU get a short wait to finish.
Then `follow` sends waiting traces, prints its summary and exits with code 0.
A second signal stops it at once.
The [reference](../reference/commands.md#follow) lists the waits and exit codes.

A minimal systemd unit, in `/etc/systemd/system/frame-trace-follow.service`:

```ini
[Unit]
Description=frame-trace follow
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=300
StartLimitBurst=5

[Service]
ExecStart=/usr/local/bin/frame-trace follow --rpc ws://127.0.0.1:9944 --otlp --cache-dir /var/cache/frame-trace
KillSignal=SIGTERM
TimeoutStopSec=45
Restart=on-failure
RestartSec=10
DynamicUser=yes
CacheDirectory=frame-trace

[Install]
WantedBy=multi-user.target
```

Start it with `sudo systemctl enable --now frame-trace-follow`.
Read its log with `journalctl -u frame-trace-follow -f`.

- `TimeoutStopSec=45` leaves time for the replay and export waits at stop.
- `Restart=on-failure` restarts `follow` after exit code 1, for example when its threads stop.
  A stop by `SIGTERM` or `SIGHUP` exits 0, so the service stays stopped.
- `StartLimitBurst=5` within `StartLimitIntervalSec=300` gives up after five failed starts in five minutes, for example with a wrong URL.

## Retries

A request that fails for a passing reason is retried.
Examples are a timeout, a dropped connection or a rate limit.
A block gets a fixed number of attempts, listed in the [reference](../reference/commands.md#follow).
For a slow node, raise `--rpc-timeout`.
Pruned state, an invalid block, a failed root check or a crash in the replay stop that block for good.
The log then says `stopped retrying block <hash> after <N> attempt(s)`.
