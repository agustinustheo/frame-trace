# Performance: current numbers

These are the numbers behind the current defaults. All runs used Rust 1.93.0 release builds on macOS arm64 against `wss://westend-asset-hub-rpc.polkadot.io`, a public node. Endpoint load and block contents change throughput between runs. The round-by-round history is in git, up to commit 2e5c243.

## Follow throughput with 8 workers

`follow` uses 8 workers and 16 RPC slots that all workers share. One cold replay runs first, and the other workers then reuse its state.

Two 120-second windows on 2026-10-26 traced 49 and 38 consecutive blocks, with no blocks skipped. The chain advanced 56 and 46 heights. In the last 60 seconds the runs traced 40 and 37 blocks, so they kept up once startup was done. Peak RSS was 698–752 MB. The old default of 2 workers traced 20 and 18 blocks in the same window and skipped 22–24.

Later runs agree. A 25-minute run with `--cache-dir` traced 145–150 blocks per 5 minutes, with a median replay of 6.3–6.6 s. A 150-minute run traced 3,306 blocks, and every root check passed.

Eight is a judgement, not a measured optimum. A 12-worker run traced 46 blocks, but that window was on a loaded host and cannot separate the worker count from host load.

The CLI limits Wasm compilation to 2 threads. On one block replayed cold three times each way, 2 threads took 26.2–27.0 s and the default 12 threads took 26.0–29.4 s. The cap lowered peak RSS by 71–137 MB.

## Memory of a long follow

All workers share the downloaded state in memory. The limit is 128 MiB, counted as an upper estimate of each piece's real allocation, plus the hash table. When a new piece goes over, the least recently read pieces go first. The table may use at most a quarter of the limit and holds at most 229,376 entries.

Three runs set the limit to 32, 128 and 512 MiB for 60 minutes each. All three read 44–45% of state from memory, with 31.5–37 RPC calls and a 6.3–7.2 s median replay per block. A 16 times larger limit changed nothing, because most misses are pieces that no earlier block read. 128 MiB stays as headroom for chains whose blocks read more state.

On Westend Asset Hub the entry limit binds first, because pieces average about 200 bytes. An 80-minute run reached it at minute 40 and stayed at 99–108 MiB charged. Heap in use was 139–150 MiB. A 150-minute run beside the old code compared the heaps. From minute 60 to 140 the old heap grew by 128 MiB, and the new heap by 72 MiB.

The estimate is high on purpose. On macOS it overcounts pieces of 200–600 bytes by about 43%, and pieces of 1–4 KiB by about 23%.

## Disk cache segments

`--cache-dir` keeps proof nodes in segment files of up to 64 MiB and removes the oldest segment first. The benchmark fills a directory with 1M nodes of 200 bytes. Before is the old layout, one file per node.

| | one file per node | segments |
| --- | --- | --- |
| disk used (`du -sk`) | 4,004,000 KiB, 1M files | 230,712 KiB, 5 files |
| startup | 101 s | 0.10–0.31 ms; index ready after 16–38 ms |
| index memory | 346 MiB | 35–36 MiB |
| writing 50 new nodes per replay | 14.3 ms median | 5–18 µs median |
| maintenance per replay | 184 ms median | 0.17–0.6 µs median |
| removing one 64 MiB segment | — | 5.1 ms |

A 25-minute `follow` with a fresh disk cache kept a flat median replay of 6.3–6.6 s while the cache grew. Ctrl-C stopped it in 0.35 s.

## APFS allocation

APFS reserves space ahead of a file that grows by many small writes. A 4-hour run peaked at 131,888 KiB in `du -sk` against a 128 MiB limit. One 2,324,917-byte segment held 3,166,208 bytes on disk.

Each segment is now charged at least its real allocation, `st_blocks × 512 + 4 KiB`. A 12-minute run with `--cache-max-bytes 33554432` then kept state at 7,640 KiB or less, inside its 8,192 KiB share. The total peaked at 33,696 KiB, over the 32,768 KiB limit. That is the documented case: the compiled runtime in use is 26,726,400 bytes, and the space kept for state is 8 MiB.

## Reproduce

Build the binary:

```sh
cargo +1.93.0 build --release --locked
```

Measure `follow` throughput and memory. Stop it with Ctrl-C after the window and read the summary line:

```sh
target/release/polkadot-frame-trace follow --rpc wss://westend-asset-hub-rpc.polkadot.io \
  --workers 8 --cache-dir <empty-dir> --cache-max-bytes 134217728
```

The disk cache benchmark is no longer in the tree. To run it, check out commit 82c679d first. The first run fills the directory. Measure memory on a second run:

```sh
FT_BENCH_DIR=<empty-dir> cargo +1.93.0 test --release -p polkadot-frame-trace-replay \
  --lib bench_disk_cache -- --ignored --nocapture
```

Set `FT_BENCH_EVICT=1` to also time the removal of one segment.
