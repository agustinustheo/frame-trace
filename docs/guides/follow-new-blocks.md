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
follow: queued=1 in-flight=0 traced=0 exported=0 export-waiting=0 export-rejected=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 up=1s
replay #19025595: fetching state (shared limit: 16 concurrent RPC requests)
replay #19025595: loading runtime and preparing state
root check: PASS; RPC calls: 65 (timed replay: 0); execution proof: 84606 bytes
fetching state #19025595: 47 proof request rounds; 65 RPC calls
state root: computed 0xc5757953430c96600a4030e29bdb7f27ea02c7218da70d35cf0da01a84991618; header 0xc5757953430c96600a4030e29bdb7f27ea02c7218da70d35cf0da01a84991618
traced finalized block #19025595 0x1e745baf8ea958a66066bd538b118fa49c5c50267980da8d3b322a613dcdbe77: 572 storage operations in 16.826s
...
traced finalized block #19025596 0x77a13455dad91faa1ab32fb7b8ad427134eb499257fe2a245d56d337dbfd8c3b: 572 storage operations in 6.623s
...
follow: queued=0 in-flight=4 traced=27 exported=27 export-waiting=0 export-rejected=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 up=1m01s
...
follow: stopping; waiting up to 10s for 0 block(s) being replayed (signal again to exit now)
follow stopped after 2m57s: traced=88 exported=88 export-rejected=0 export-dropped=0 export-unsent=0 stopped=0 skipped=0 reverted=0 cancelled=0 poll-failures=0
```

The cache directory was kept from an earlier run, so the first block took 17 seconds.
After that, the median was 7.7 seconds per block in this run.
Each time covers loading state and replaying it on this machine.
It does not include sending the trace, or time spent waiting for the first replay.
Each traced block appears in Jaeger as its own trace.

## 2. Read the status line

frame-trace checks the node for new final blocks every two seconds.
It prints a status line when a problem count changes, and once a minute otherwise.
The [reference](../reference/commands.md#follow) lists these intervals.

| Field | Meaning |
| --- | --- |
| `queued` | Blocks waiting to be replayed. |
| `in-flight` | Blocks assigned to workers, including those waiting for the first replay. |
| `traced` | Blocks replayed since the start. |
| `exported` | Traces the receiver accepted. Only with `--otlp`. |
| `export-waiting` | Traces waiting to be sent. Only with `--otlp`. |
| `export-rejected` | Traces the receiver refused in part. They are not sent again. Only with `--otlp`. |
| `export-dropped` | Traces dropped because too many were waiting. Only with `--otlp`. |
| `retrying` | Blocks that failed for a passing reason, such as a dropped connection, and will be tried again. |
| `stopped` | Blocks that failed for good, such as pruned state or a failed root check. |
| `skipped` | Blocks dropped because newer blocks arrived first. |
| `reverted` | Blocks that left the chain in a reorg. Only with `--best`. |
| `up` | Time since `follow` started. |

All counts except `queued`, `in-flight`, `retrying` and `export-waiting` are totals since the start.

When you stop `follow`, the last line sums up the run.
It adds `cancelled`, the blocks still waiting at the stop, and `poll-failures`, the failed checks for new blocks.
With `--otlp`, `export-unsent` counts traces still waiting when the stop ended.

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

## 5. When the node or the collector goes away

If a check for new blocks fails, frame-trace logs it and tries again.
It waits longer after each failure, up to a minute, and logs once a minute while it keeps failing.

```text
follow poll failed: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 2s
follow poll still failing after 6 attempts over 1m06s: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 1m00s
```

`follow` keeps running until the node answers again, then logs `follow poll recovered after <N> failed attempt(s) over <time>`.

If the OTLP receiver stops, frame-trace keeps replaying blocks and holds their traces.
It sends them when the receiver is back. It does not replay the blocks again.
Here the receiver was stopped for 30 seconds:

```text
OTLP export failed (1 attempt(s) in a row): 578 of 578 spans were not exported (first error: Operation failed: HTTP export failed: network error); retrying in 1s, 10 trace(s) waiting
OTLP export recovered after 3 failed attempt(s) over 27s; 21 trace(s) waiting
```

A long outage fills the waiting queue. frame-trace then drops the oldest trace and counts it as `export-dropped`.
If the receiver is still down when you stop `follow`, the waiting traces are not sent.
The summary counts them as `export-unsent`.

## 6. Run it as a service

`follow` stops cleanly on `SIGTERM` and `SIGHUP`, as on Ctrl-C.
It finishes the blocks it is replaying, sends waiting traces and prints its summary.
A second signal stops it at once.
The [exit codes](../reference/commands.md#follow) table lists what each exit means.

A minimal systemd unit, in `/etc/systemd/system/frame-trace-follow.service`:

```ini
[Unit]
Description=frame-trace follow
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/frame-trace follow --rpc ws://127.0.0.1:9944 --otlp --cache-dir /var/cache/frame-trace
KillSignal=SIGTERM
TimeoutStopSec=30
Restart=on-failure
RestartSec=10
DynamicUser=yes
CacheDirectory=frame-trace

[Install]
WantedBy=multi-user.target
```

Start it with `sudo systemctl enable --now frame-trace-follow`, and read its log with `journalctl -u frame-trace-follow -f`.
`TimeoutStopSec=30` leaves time for the replay and export waits at stop.

## Retries

A request that fails for a passing reason, such as a timeout, a dropped connection or a rate limit, is retried.
A block gets at most four attempts.
Pruned state, an invalid block, a failed root check or a crash in the replay stop that block for good.
The log then says `stopped retrying block <hash> after <N> attempt(s)`.
