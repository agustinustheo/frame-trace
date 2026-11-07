# Follow new blocks

`polkadot-frame-trace follow` traces each new finalized block and sends it to your trace viewer.
Run it next to Jaeger to see a chain's storage work live.

## 1. Start following

Start Jaeger as in [Your first trace](../getting-started/first-trace.md), then run:

```sh
polkadot-frame-trace follow --rpc wss://westend-asset-hub-rpc.polkadot.io --otlp --cache-dir .polkadot-frame-trace-cache
```

Press Ctrl-C to stop.
Here is output from a three-minute run that sent traces to a local OTLP receiver.

```text
follow: queued=1 in-flight=0 traced=0 exported=0 export-waiting=0 export-rejected=0 export-failed=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 state-memory=0MiB state-pieces=0 state-from-memory=0% up=1s
replay #19079015: fetching state (shared limit: 16 concurrent RPC requests)
replay #19079015: loading runtime and preparing state
root check: PASS; RPC calls: 70 (timed replay: 0); execution proof: 89976 bytes
fetching state #19079015: 49 proof request rounds; 70 RPC calls
state root: computed 0xb7429407e4d537352a6d2a59e8226f97f4bb6b1adb441735f3e920433e9e69af; header 0xb7429407e4d537352a6d2a59e8226f97f4bb6b1adb441735f3e920433e9e69af
traced finalized block #19079015 0xc8364c098607d2b02067a415d162d3f6324877c5ad03baeca91961783b8a10bd: 569 storage operations in 15.435s
...
traced finalized block #19079016 0x33b9bd4c96d7f1a342d87fe16cdc9e36f90e123e75d49a02e8d6653d88298236: 569 storage operations in 6.258s
...
follow: queued=0 in-flight=6 traced=25 exported=25 export-waiting=0 export-rejected=0 export-failed=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 state-memory=7MiB state-pieces=9237 state-from-memory=33% up=1m01s
...
follow: queued=0 in-flight=3 traced=58 exported=58 export-waiting=0 export-rejected=0 export-failed=0 export-dropped=0 retrying=0 stopped=0 skipped=0 reverted=0 state-memory=10MiB state-pieces=16938 state-from-memory=36% up=2m01s
...
follow: stopping (signal again to exit now)
replay job cancelled
follow stopped after 3m00s: traced=88 exported=88 export-rejected=0 export-failed=0 export-dropped=0 export-unsent=0 stopped=0 skipped=0 reverted=0 cancelled=1 poll-failures=0
```

The disk cache was kept from an earlier run. The first block took 15 seconds and the next 6 seconds.
Each time covers loading state and the replay on this machine, not the export.
Each traced block shows in Jaeger as its own trace.

## 2. Read the status line

`follow` prints a status line when a problem count changes, and otherwise at a fixed [interval](../reference/commands.md#follow).

| Field | Meaning |
| --- | --- |
| `queued` | Blocks waiting to be replayed. |
| `in-flight` | Blocks assigned to workers, including those waiting for the first replay. |
| `traced` | Blocks replayed since the start. |
| `exported` | Traces the receiver accepted. Only with `--otlp`. |
| `export-waiting` | Traces waiting to be sent. Only with `--otlp`. |
| `export-rejected` | Traces the receiver refused in part. They are not sent again. Only with `--otlp`. |
| `export-failed` | Traces whose export crashed inside polkadot-frame-trace. They are not sent again. Only with `--otlp`. |
| `export-dropped` | Traces dropped because too many were waiting. Only with `--otlp`. |
| `retrying` | Blocks that failed for a passing reason, such as a dropped connection, and will be tried again. |
| `stopped` | Blocks that failed for good, such as pruned state or a failed root check. The log says `stopped retrying block <hash> after <N> attempt(s)`. |
| `skipped` | Blocks not replayed, or not retried, because newer blocks arrived first. |
| `reverted` | Blocks that left the chain in a reorg, counted once whatever their state. Only with `--best`. |
| `state-memory` | Memory used by downloaded state kept for all workers, in MiB. An upper estimate: the real use is lower. It stops growing at the [limit](../reference/commands.md#follow). |
| `state-pieces` | Pieces of downloaded state kept in memory. |
| `state-from-memory` | Share of the state replays needed that was already in memory, since the start. The rest came from the disk cache or the node. |
| `up` | Time since `follow` started. |

All counts except `queued`, `in-flight`, `retrying`, `export-waiting`, `state-memory` and `state-pieces` are totals since the start.

The last line sums up the run when you stop `follow`. It adds these counts:

- `cancelled`: blocks still waiting, or still running when the stop wait ended;
- `poll-failures`: failed checks for new blocks;
- `export-unsent`: traces not sent when the stop ended, with `--otlp`;
- `log-dropped`: log lines lost because stderr was blocked. It shows only when lines were lost.

## 3. Keep up with the chain

`follow` replays several blocks at once, 8 by default.
The first replay runs alone, and the other workers wait for it.
If replays stay slower than the chain, older blocks are skipped: `follow` samples recent blocks and does not promise every block.
For a complete range, use [`report`](analyse-many-blocks.md).

These log lines show that replays fall behind:

| Line | Meaning |
| --- | --- |
| `follow queue full; sampling near the tip at replay speed` | The waiting queue is full. polkadot-frame-trace stops queueing older blocks and prefers recent heights. |
| `replay job skipped: queued block expired behind tip` | A waiting block fell outside the recent-height limit. polkadot-frame-trace dropped it without replaying it. It counts as `skipped`. |
| `skipped block <hash>: retry expired behind tip` | The same, for a block that was waiting to be retried. |

To go faster, keep `--cache-dir` and raise `--workers`.
On a fast connection, raise `--proof-concurrency`. A node close to you also helps.
The [reference](../reference/commands.md#follow) lists the limits.

## 4. Follow the newest blocks with `--best`

`--best` follows the newest blocks instead of finalized ones.
A reorg may replace them. `follow` then logs the reorg and counts the replaced block as `reverted`.
If a reverted block comes back to the chain, `follow` may trace it again.

## 5. When the node or the receiver goes away

If a check for new blocks fails, `follow` logs it and tries again. It waits longer after each failure.

```text
follow poll failed: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 2s
follow poll still failing after 6 attempts over 1m06s: RPC error: chain_getFinalizedHead: connect ws://127.0.0.1:9: IO error: Connection refused (os error 61); retrying in 1m00s
```

When the node answers again, it logs `follow poll recovered after <N> failed attempt(s) over <time>`.

If the OTLP receiver stops, `follow` keeps replaying blocks and holds their traces, up to the [queue size](../reference/commands.md#follow).
It sends them when the receiver is back, without replaying the blocks, and never sends an acknowledged request again.
Traces still waiting at the stop count as `export-unsent`, and traces dropped from a full queue as `export-dropped`.

In the run from step 1, the receiver refused one request with a 400, then was stopped for 30 seconds:

```text
OTLP export failed (1 attempt(s) in a row): 63 of 575 spans were not exported (first error: Operation failed: HTTP export failed with status code: 400); retrying in 1s, 1 trace(s) waiting
OTLP export recovered after 1 failed attempt(s) over 1s; 0 trace(s) waiting
...
OTLP export failed (2 attempt(s) in a row): 575 of 575 spans were not exported (first error: Operation failed: HTTP export failed: network error); retrying in 2s, 9 trace(s) waiting
OTLP export recovered after 3 failed attempt(s) over 27s; 17 trace(s) waiting
```

## 6. Run it as a service

`follow` stops cleanly on `SIGTERM` and `SIGHUP`, as on Ctrl-C. It sends waiting traces, prints its summary and exits with code 0.
The [reference](../reference/commands.md#follow) lists the stop waits and exit codes.

A minimal systemd unit, in `/etc/systemd/system/polkadot-frame-trace-follow.service`:

```ini
[Unit]
Description=polkadot-frame-trace follow
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=300
StartLimitBurst=5

[Service]
ExecStart=/usr/local/bin/polkadot-frame-trace follow --rpc ws://127.0.0.1:9944 --otlp --cache-dir /var/cache/polkadot-frame-trace
KillSignal=SIGTERM
TimeoutStopSec=45
Restart=on-failure
RestartSec=10
DynamicUser=yes
CacheDirectory=polkadot-frame-trace
CacheDirectoryMode=0700
UMask=0077
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictSUIDSGID=yes
RestrictRealtime=yes
LockPersonality=yes
CapabilityBoundingSet=
AmbientCapabilities=
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
LimitCORE=0

[Install]
WantedBy=multi-user.target
```

Start it with `sudo systemctl enable --now polkadot-frame-trace-follow`.
Read its log with `journalctl -u polkadot-frame-trace-follow -f`.

`TimeoutStopSec=45` leaves time for the replay and export waits at stop.
`Restart=on-failure` restarts `follow` after exit code 1, for example after the [replay deadline](../reference/commands.md#follow) or when it can no longer write its log.
A stop by signal exits 0, so the service stays stopped.
`CacheDirectoryMode=0700` and `UMask=0077` keep the [disk cache](../reference/commands.md#disk-cache) private.
`MemoryDenyWriteExecute=yes` is left out on purpose, because the runtime is compiled to machine code at run time.
