# Offline replay fixtures

These are E2's successful full `Core_execute_block` inputs, converted to the
[version 1 artifact format](../../crates/replay/ARTIFACT.md). No live node is needed.
All rehydrate authenticated runtime code, parent header, runtime version, metadata
and complete discovery proof nodes. The shared People/Westend Wasm nodes and metadata are stored
once under `blobs/<blake2-256>.zst`; fixture metadata/node strings use `blob:<hash>`.
The loader bounds decompression and verifies each blob's content hash before
rehydrating hex. No arbitrary reference paths are accepted. `Artifact::save`
always writes self-contained artifacts. The seventeen compressed files, including call artifacts, blobs,
oracles and the public reference, total 11,153,408 bytes (10.63 MiB). Sharing the
Westend Wasm as raw blob bytes also reduces the original public block artifact.

| Directory | Block | Source | Artifact bytes |
| --- | --- | --- | ---: |
| `people-transfer` | 2 / `ea56e9d6…` | E2 untraced People, signed transfer | 8,563 |
| `people-kill-prefix` | 8 / `7d99aaa1…` | E2 untraced People, sudo kill_prefix | 9,114 |
| `people-traced` | 4 / `06618995…` | E2 traced People, three-transfer batch | 4,116,156 |
| `westend-asset-hub` | 18,558,320 / `4c0d1d57…` | Public Westend Asset Hub, two inherents | 275,788 |

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

## State call fixtures

| Directory | State | Method | Artifact bytes |
| --- | --- | --- | ---: |
| `call-people-nonce` | People #1 / `e1e91429…` | `AccountNonceApi_account_nonce` (Alice) | 2,425 |
| `call-people-fee` | People #1 / `e1e91429…` | `TransactionPaymentApi_query_info` (the saved signed transfer, plus u32 byte length) | 2,009 |
| `call-people-metadata` | People #1 / `e1e91429…` | `Metadata_metadata_versions` | 1,662 |
| `call-westend-nonce` | Westend Asset Hub #18,561,759 / `60c39d38…` | `AccountNonceApi_account_nonce` (Alice) | 6,302 |
| `call-westend-dryrun` | Westend Asset Hub #18,563,012 | `DryRunApi_dry_run_call` (root set_code([])) | 5,066 |

The People calls execute against the existing transfer fixture's **parent state**,
not block #2's resulting state. They use its original V14 metadata, so results
remain hex; their recorded outputs are local executions, **not node oracle data**.
`cargo +1.93.0 run -p frame-trace-replay --example record_calls` regenerates those
three artifacts offline. The recorder extracts proofs only for observed keys plus
`:code`/`:heappages`, executes again with the reduced proof, and reuses shared blobs.
Nonce is 0; query_info returns the saved runtime's dispatch info; metadata versions
are `[14, 15, 16]`. The metadata-version query has no storage operations and a
zero-byte execution proof, but still needs authenticated runtime-loading nodes.

The public call was recorded on 2026-10-25 from
`wss://westend-asset-hub-rpc.polkadot.io`, using `--at finalized --oracle` and Alice's
32-byte account (`d43593c7…a56da27d`). Exact state hash:
`0x60c39d38f0135762de46c998a9628e606483e52d4b838ea6d4c5da383b940502`.
V16 metadata came from `Metadata_metadata_at_version(16)` at that same hash.
Local result and independent node `state_call` both returned `0x08020000` (nonce
520). The call reads one `System::Account`, with a 3,713-byte execution proof.
Its artifact shares the identical runtime node with the existing public block
fixture and stores V16 metadata in a shared content-addressed blob. Offline tests
re-execute and compare with the recorded oracle; they do not contact the endpoint.
No call claims a block root check. All reads authenticate against its state header.

Round 3 added `call-westend-dryrun` at Westend Asset Hub #18,563,012, recorded
2026-10-25 with `DryRunApi_dry_run_call`, input `0x000000020005000000`
(root `System.set_code([])`). Local execution and live `state_call` matched;
24 storage operations, 3,047 execution-proof bytes, zero measured RPCs. This
exercises the executor-backed runtime-version reader. Its oracle is recorded
(unverified), as with every offline oracle. Both this fixture and the refreshed
`call-westend-nonce` retain metadata-execution dependencies and re-derive V16
locally without RPC or fallback. Round 4 extends `people-traced`, `people-transfer`, and `people-kill-prefix`
with metadata-only proof nodes. A fresh local E1 traced People chain executed
`Metadata_metadata_at_version(16)` while loading metadata for a saved
`Metadata_metadata_versions` call at genesis. Small dependency proof nodes were
merged into the existing artifacts; original block/parent hashes and independent
ordered oracles remain unchanged. Every needed node is re-authenticated by
traversing each original parent root; fresh offline sessions regenerate V16 names
without any fallback or RPC. The transferred nodes cover unchanged constants
(including NetworkSuffix/Parameters); unrelated large runtime-loading nodes were
excluded. Artifact sizes are now 8,563 / 9,114 / 4,116,156 bytes for transfer,
kill-prefix / traced batch. Local regeneration used E1 `run-node.py`,
`/tmp/pm-audit/e1/people-traced.json`, and the documented omni-node binary. The
runtime source/Binary/Wasm provenance remains E1’s. `--unauthenticated-metadata`
(`--rpc-metadata-fallback` alias) explicitly allows stored metadata for older
artifacts that still lack execution dependencies; output labels it unauthenticated.
