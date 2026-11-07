# Offline replay fixtures

These are E2's successful full `Core_execute_block` inputs, converted to the
[version 1 artifact format](../../crates/replay/ARTIFACT.md). No live node is needed.
All rehydrate authenticated runtime code, parent header, runtime version, metadata
and complete discovery proof nodes. The shared People/Westend Wasm nodes and metadata are stored
once under `blobs/<blake2-256>.zst`; fixture metadata/node strings use `blob:<hash>`.
The loader bounds decompression and verifies each blob's content hash before
rehydrating hex. No arbitrary reference paths are accepted. `Artifact::save`
always writes self-contained artifacts. The 19 compressed files, including call artifacts, blobs,
oracles and the public reference, total 13,818,269 bytes (13.18 MiB). Sharing the
Westend Wasm as raw blob bytes also reduces the original public block artifact.

| Directory | Block | Source | Artifact bytes |
| --- | --- | --- | ---: |
| `people-transfer` | 2 / `ea56e9d6…` | E2 untraced People, signed transfer | 8,563 |
| `people-kill-prefix` | 8 / `7d99aaa1…` | E2 untraced People, sudo kill_prefix | 9,114 |
| `people-traced` | 4 / `06618995…` | E2 traced People, three-transfer batch | 4,116,156 |
| `westend-asset-hub` | 18,558,320 / `4c0d1d57…` | Public Westend Asset Hub, two inherents | 275,788 |
| `paseo-asset-hub-contract` | 15,366,850 / `e6b71b09…` | Public Paseo Asset Hub, a contract call with child trie reads and writes | 101,324 |

The local `oracle.json.zst` files are E2's independent `state_traceBlock` RPC
recordings of the exact same blocks. Tests compare the entire ordered keyed
field list, omitting only random `ext_id`, and the phase ownership of every
oracle operation. E1's older chain recordings are in `e1-chains.tar.zst`, restored as [`../README.md`](../README.md) says;
their hashes differ because they were recorded on different fresh dev chains.
Local metadata is copied from E1's `next-people-paseo/2/metadata.hex` for the same
People runtime. Public metadata was fetched at the saved parent hash.

The public service has no unsafe oracle. `reference-events.json.zst` is E2's
independently saved successful public execution, not a `state_traceBlock` response.
Tests compare it and execute the real root assertion again. They do not claim a
public unsafe-oracle match.

E2 provenance, Wasm hashes and acquisition results are in
[`dev/experiments/e2`](../../dev/experiments/e2/README.md). These fixtures were acquired
on 2026-10-25. The production tests use stable2609 host crates to execute both
E1's stable2606-2 People Wasms and public `westmint` spec 1025002. Root tampering
is tested after recomputing the header hash, so rejection reaches the runtime's
actual final root assertion. Missing nodes and changed parent headers also fail.
Remote child-proof loading is tested separately against SDK-generated child
proofs served over loopback HTTP. `paseo-asset-hub-contract` is the only chain block
fixture that touches child state.

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
`cargo +1.93.0 run -p polkadot-frame-trace-replay --example record_calls` regenerates those
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

## Runtime upgrade

No runtime upgrade block is committed. Westend Asset Hub #18,985,169
(`0xd0e2fe81…ce55b`) enacted spec 1025003 -> 1025004: the
`ParachainSystem.set_validation_data` inherent wrote the new `:code`. #18,985,170
(`0x1ccb4c8ce07cacb802982d8b2aa1c8ceea40267c2503d1f48504fd7e6a405d8d`) is the first block
executed with the new code; its initialization writes `System::LastRuntimeUpgrade`.

Saved, #18,985,170 alone needs the new runtime (`c7a91f4a…`, 2,716,830 bytes compressed);
with its artifact and metadata it was 3,122,414 bytes, a fixture from 2026-11-06 until
2026-11-07. The enactment block needs both runtimes, the old one as `:code` and the new one
as `ParachainSystem::PendingValidationCode`. The pending value carries a 4-byte length
prefix, so it cannot share the `:code` blob; converted with `blobify.py`, the two runtime
blobs plus metadata measured about 5.3 MB. Neither runtime is shared with another fixture,
and `:code` is already zstd-compressed by the SDK, so recompressing saves nothing.

The ignored live test `runtime_upgrade_enactment_and_next_block_live` replays both blocks
every night from an archive endpoint (`FRAME_TRACE_WESTEND_AH_RPC`), in one session and
each alone. Offline, `one_session_replays_blocks_on_different_runtimes` replays
`westend-asset-hub` (spec 1025002), `paseo-asset-hub-contract` (spec 2005004) and
`westend-asset-hub` again in one session, as `follow` meets two runtimes across an upgrade.
To replay #18,985,170 offline, save it again from an archive node:

```sh
polkadot-frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block 18985170 --save <dir>
polkadot-frame-trace block --offline <dir>
```

## Making a fixture

Fixture blobs were made from the `--save` output with [`blobify.py`](blobify.py): every
proof node of at least 100,000 bytes moves into `blobs/<blake2-256>.zst` (`zstd -19`) and
is replaced by `blob:<hash>`; the artifact itself is recompressed with `zstd -19`. Running
it again on `block --offline paseo-asset-hub-contract --save <dir>` reproduces that
fixture's blobs byte for byte and the same artifact JSON.

`blobify.py` also empties the stored metadata. A replay derives metadata by running the
runtime's metadata API on the saved proof nodes, which `--save` includes, and uses stored
metadata only with `--unauthenticated-metadata` when that fails. The older fixtures still
store theirs. On 2026-11-07 this removed the Paseo fixture's 170,057-byte metadata blob
(`25dcaa7e…`). Runtime blobs barely shrink:
`:code` is already zstd-compressed by the SDK (`sp-maybe-compressed-blob`, magic
`52bc537646db8e05`), and `zstd -19` on top changes it by a few dozen bytes.

## Child trie fixture

`paseo-asset-hub-contract` is Paseo Asset Hub #15,366,850
(`0xe6b71b09271e2dc4ce72ced4f9b7287aa0763e65ddd75e48cf90519693d8e3f5`, spec 2005004),
recorded 2026-11-06 from `wss://asset-hub-paseo-rpc.n.dwellir.com` with `block --save`.
Extrinsic 2 is a `Revive.eth_transact` to contract
`0x0d78e5414aa8c45f40e4c40b0c862ff0486e3262` (Ethereum transaction
`0x5936d008…b2ec`, found with `eth_getBlockTransactionCountByNumber` on
`https://eth-rpc-testnet.polkadot.io`). pallet-revive stores the contract's storage in
default child trie `0xefdd7b91…5a2d`, the trie id in its `Revive::AccountInfoOf` entry.
The call makes 14 child trie reads and 4 child trie writes. The recording run on
2026-11-06 measured root check PASS, 113 RPC calls and 101,420 execution-proof bytes. The
runtime (`d85bfdb1…`) is a blob; the fixture stores no metadata (see
[Making a fixture](#making-a-fixture)).

Paseo Asset Hub was used because Westend Asset Hub has almost no contract activity: on
2026-11-06 a scan of the 40,000 blocks before #19,021,000 through Westend Asset Hub's
Ethereum RPC found one Ethereum transaction (#19,008,649). The same scan on Paseo found 13
in its latest 200 blocks.

### How it was recorded

On 2026-11-06 that endpoint took about 35 seconds to return the proof of `:code`
(5,128,826 hex characters, measured once). `--rpc-timeout` (then 7.5 seconds by default, now 30) is the
wait for one try; a request gets four tries, so its overall deadline is four times that.
With the default, the run failed with `state_getReadProof: overall request deadline
exceeded`. With `--rpc-timeout 120`, each try waits up to 120 seconds and the run passed.
Only the RPC time limits change; the replay and its checks are the same.

```sh
cargo +1.93.0 build --release --locked -p polkadot-frame-trace
target/release/polkadot-frame-trace block --rpc wss://asset-hub-paseo-rpc.n.dwellir.com \
  --block 15366850 --rpc-timeout 120 --save <dir> --json <file>
python3 -I fixtures/replay/blobify.py <dir> fixtures/replay/paseo-asset-hub-contract fixtures/replay/blobs 100000
```

Run again on 2026-11-06 with this command, the replay passed the root check with 113 RPC
calls in 58 seconds, and `blobify.py` reproduced the artifact and both blobs byte for byte.
On 2026-11-07, `blobify.py` without metadata, run on the offline `--save` output, gave the
same artifact JSON apart from the emptied metadata.

## Relay chain

No relay chain block is committed. Westend relay chain #33,614,892
(`0xa7ab2bf3de7abcdc97740fc6a1187288e61490f174f4146a9a0b05e81596ff07`) was a fixture until
2026-11-07; its runtime blob alone was 1,552,297 bytes compressed and the three files 1,733,346
bytes. The runtime's `:code` is already zstd-compressed by the SDK, so recompressing it saves
nothing. The ignored live test `public_relay_network` replays the finalized block from
`FRAME_TRACE_WESTEND_RPC` (default `wss://westend-rpc.polkadot.io`) every night. It checks
the root, that no call fails to decode, that the first extrinsics are `Timestamp.set` and
`ParaInherent.enter`, and that `ParaInherent.enter` writes `ParaInherent::OnChainVotes`.
To replay the old block offline, save it again from an archive node:

```sh
polkadot-frame-trace block --rpc wss://westend-rpc.polkadot.io --block 33614892 --save <dir>
polkadot-frame-trace block --offline <dir>
```
