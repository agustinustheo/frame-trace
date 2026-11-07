# Fixtures

| Path | What |
| --- | --- |
| `model/` | Small traces used by the unit tests, including real People blocks under `model/real/` |
| `metadata/` | Westend metadata used by the decode tests |
| `wasm/missing-host-function.wasm` | A 136-byte test runtime that imports a host function polkadot-frame-trace lacks. Built from the `.wat` next to it; `missing_host_function_wasm_matches_its_source` checks they match and regenerates it with `FRAME_TRACE_REGENERATE_WASM=1` |

Experiment E1's recordings, `e1-probes/` and `e1-chains.tar.zst`, were removed on 2026-11-07.
No test reads them. They and the E1 and E2 experiment scripts are in commit 2e5c243:

```sh
git checkout 2e5c243 -- fixtures/e1-probes fixtures/e1-chains.tar.zst dev/experiments
cd fixtures && zstd -dc e1-chains.tar.zst | tar -xf -
```

The unpacked chain directories are about 70 MB and are ignored by git.
