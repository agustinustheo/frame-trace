# Offline replay fixtures

These are E2's successful full `Core_execute_block` inputs, converted to the
[version 1 artifact format](../../crates/replay/ARTIFACT.md). No live node is needed.
All rehydrate authenticated runtime code, parent header, runtime version, metadata
and complete discovery proof nodes. The shared People Wasm and metadata are stored
once under `blobs/<blake2-256>.zst`; fixture metadata/node strings use `blob:<hash>`.
The loader bounds decompression and verifies each blob's content hash before
rehydrating hex. No arbitrary reference paths are accepted. `Artifact::save`
always writes self-contained artifacts. The ten compressed files, including blobs,
oracles and the public reference, total 11,195,050 bytes (10.68 MiB), down from
15,882,348 bytes.

| Directory | Block | Source | Artifact bytes |
| --- | --- | --- | ---: |
| `people-transfer` | 2 / `ea56e9d6…` | E2 untraced People, signed transfer | 7,882 |
| `people-kill-prefix` | 8 / `7d99aaa1…` | E2 untraced People, sudo kill_prefix | 8,368 |
| `people-traced` | 4 / `06618995…` | E2 traced People, three-transfer batch | 4,115,093 |
| `westend-asset-hub` | 18,558,320 / `4c0d1d57…` | Public Westend Asset Hub, two inherents | 3,154,143 |

The local `oracle.json.zst` files are E2's independent `state_traceBlock` RPC
recordings of the exact same blocks. Tests compare the entire ordered keyed
field list, omitting only random `ext_id`, and the phase ownership of every
oracle operation. E1's older chain recordings remain in `../e1-chains.tar.zst`;
their hashes differ because they were recorded on different fresh dev chains.
Local metadata is copied from E1's `next-people-paseo/2/metadata.hex` for the same
People runtime. Public metadata was fetched at the saved parent hash.

The public service has no unsafe oracle. `reference-events.json.zst` is E2's
independently saved successful public execution, not a `state_traceBlock` response.
Tests compare it and execute the real root assertion again. They do not claim a
public unsafe-oracle match.

E2 provenance, Wasm hashes and acquisition results are in
[`experiments/e2`](../../experiments/e2/README.md). These fixtures were acquired
on 2026-10-25. The production tests use stable2609 host crates to execute both
E1's stable2606-2 People Wasms and public `westmint` spec 1025002. Root tampering
is tested after recomputing the header hash, so rejection reaches the runtime's
actual final root assertion. Missing nodes and changed parent headers also fail.
Remote child-proof loading is tested separately against SDK-generated child
proofs served over loopback HTTP; these four chain blocks do not touch child state.
