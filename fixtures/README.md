# Fixtures

| Path | What |
| --- | --- |
| `model/` | Small traces used by the unit tests, including real People blocks under `model/real/` |
| `metadata/` | Westend metadata used by the decode tests |
| `wasm/missing-host-function.wasm` | A 136-byte test runtime that imports a host function frame-trace lacks. Built from the `.wat` next to it; `missing_host_function_wasm_matches_its_source` checks they match and regenerates it with `FRAME_TRACE_REGENERATE_WASM=1` |

Experiment E1's recordings, `e1-probes/` (probe responses: response-size limit, filters,
pruning) and `e1-chains.tar.zst` (every block E1 recorded on the local People chain, four
node/runtime variants), were removed from the tree on 2026-11-07: no test reads them, and
together they were 3,062,030 bytes. They stay in the history. Restore and unpack them
before running the E1 scripts in [`dev/experiments/e1`](../dev/experiments/e1/README.md):

```sh
git checkout 2e5c243 -- fixtures/e1-probes fixtures/e1-chains.tar.zst
cd fixtures && zstd -dc e1-chains.tar.zst | tar -xf -
```

The unpacked chain directories are about 70 MB (each block directory repeats the same 1 MB
metadata) and are ignored by git.
