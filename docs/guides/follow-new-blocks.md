# Follow new blocks

`frame-trace follow` traces each new final block as it arrives and sends it to your trace viewer.
Leave it running next to Jaeger to watch a chain's storage work live.

## 1. Start following

Start Jaeger as in [Your first trace](../getting-started/first-trace.md), then run:

```sh
frame-trace follow --rpc wss://westend-asset-hub-rpc.polkadot.io --otlp --cache-dir .frame-trace-cache
```

Press Ctrl-C to stop.
Here is a real run, stopped after 50 seconds:

```text
follow: queued=1 in-flight=0 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
replay #18596971: fetching state (up to 8 concurrent proof requests)
follow: queued=0 in-flight=1 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
replay #18596971: loading runtime and preparing state
follow: queued=0 in-flight=1 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
follow: queued=0 in-flight=1 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
follow: queued=0 in-flight=1 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
follow: queued=0 in-flight=1 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
follow: queued=0 in-flight=1 done=0 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
root check: PASS; RPC calls: 61 (timed replay: 0); execution proof: 89181 bytes
fetching state #18596971: 47 proof request rounds; 61 RPC calls
state root: computed 0xa0b1f52c2aedb89c41e231c8bdec3f5497afd44eadfa5c55ac8b1ee9b23a11d7; header 0xa0b1f52c2aedb89c41e231c8bdec3f5497afd44eadfa5c55ac8b1ee9b23a11d7
traced finalized block #18596971 0x8787996125a97c9a70d7bb7a083e8d49e1296821165050cc7043cd04175a956c: 562 storage operations in 16.892s
follow: queued=0 in-flight=0 done=1 retrying=0 stopped=0 reverted=0 skipped=0 cancelled=0
...
cancelled block 0x095999704fd4e455512b27ad9fe2fb8a7ab727da12198e5c5353eb3b0f2a64db: follow stopped
cancelled block 0x25e832190366cb56aa8fb5de8340b68272feda04fa665a415f90aba4d8b4811a: follow stopped
...
follow shutdown: skipped=0 cancelled=7
follow stopped
```

The first block took 17 seconds.
Later blocks took about 6 seconds, because the cache already held most of the state.
Each traced block appears in Jaeger as its own trace.

## 2. Read the status line

frame-trace checks the node for new final blocks every two seconds and prints a status line:

| Field | Meaning |
| --- | --- |
| `queued` | Blocks waiting to be replayed. |
| `in-flight` | Blocks being replayed now. Two run at a time. |
| `done` | Blocks traced and exported. |
| `retrying` | Blocks that failed for a passing reason, such as a dropped connection, and will be tried again. |
| `stopped` | Blocks that failed for good, such as pruned state or a failed root check. |
| `reverted` | Blocks that left the chain in a reorg. Only with `--best`. |
| `skipped` | Blocks dropped because newer blocks arrived first. |
| `cancelled` | Blocks still waiting when you pressed Ctrl-C. |

## 3. Keep up with the chain

frame-trace keeps at most eight blocks waiting and always prefers the newest.
If replays are slower than the chain, older blocks are skipped.
So `follow` samples the chain at the speed your machine can replay; it does not promise every block.
For a complete range, use [`report`](analyse-many-blocks.md) instead.

To go faster:

- keep `--cache-dir`, so each block fetches only what changed;
- raise `--proof-concurrency` on a fast connection;
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
