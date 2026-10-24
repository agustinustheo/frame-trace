# Fixtures

| Path | What |
| --- | --- |
| `model/` | Small traces used by the unit tests, including real People blocks under `model/real/` |
| `metadata/` | Westend metadata used by the decode tests |
| `e1-probes/` | E1 probe responses (response-size limit, filters) |
| `e1-chains.tar.zst` | Every block E1 recorded on the local People chain, four node/runtime variants |

The E1 chain recordings are compressed because they are about 70 MB unpacked (each block directory repeats the same 1 MB metadata). Unpack them before running the E1 scripts:

```sh
cd fixtures && zstd -dc e1-chains.tar.zst | tar -xf -
```

The unpacked directories are ignored by git.
