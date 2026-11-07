# Offline replay fixtures

Saved blocks and runtime API calls in the [version 1 artifact format](../../crates/replay/ARTIFACT.md). Tests replay them with no network and run the real root check.

Proof nodes of 100,000 bytes or more, such as runtimes, are stored once under `blobs/<blake2-256>.zst`. An artifact refers to them as `blob:<hash>`. The loader limits decompression and checks each blob's hash. `Artifact::save` always writes a self-contained artifact.

The local People fixtures come from experiment E2, recorded on 2026-10-25. Its provenance, Wasm hashes and scripts are in `dev/experiments/e2` at commit 2e5c243.

## Blocks

| Directory | Block | What it is | Recorded | Artifact bytes |
| --- | --- | --- | --- | ---: |
| `people-transfer` | 2 / `ea56e9d6…` | Local People chain (E2, untraced runtime), a signed transfer | 2026-10-25 | 8,563 |
| `people-kill-prefix` | 8 / `7d99aaa1…` | Local People chain (E2, untraced), sudo `kill_prefix` | 2026-10-25 | 9,114 |
| `people-traced` | 4 / `06618995…` | Local People chain (E2, `with-tracing` runtime), a three-transfer batch | 2026-10-25 | 4,116,156 |
| `westend-asset-hub` | 18,558,320 / `4c0d1d57…` | Public Westend Asset Hub, spec 1025002, two inherents | 2026-10-25 | 275,788 |
| `paseo-asset-hub-contract` | 15,366,850 / `e6b71b09…` | Public Paseo Asset Hub, spec 2005004, a contract call with 14 child trie reads and 4 child trie writes | 2026-11-06 | 101,324 |

The `oracle.json.zst` files are `state_traceBlock` recordings of the same blocks from the local node. Tests compare the whole ordered field list, except the random `ext_id`, and the phase of every operation. The public node has no unsafe RPC, so `westend-asset-hub` has `reference-events.json.zst` instead: a saved earlier replay, not a node response.

The local People metadata comes from E1 for the same runtime. The People fixtures also hold the proof nodes their runtime needs to build V16 metadata, added in a later round. The production tests run the stable2606-2 People runtimes and Westend spec 1025002 with stable2609 host crates.

Tests also change the root, drop nodes and change parent headers, and check that each fails. `paseo-asset-hub-contract` is the only block fixture with child state. Paseo was used because Westend Asset Hub has almost no contract activity: a scan of 40,000 blocks found one Ethereum transaction.

## Runtime API calls

| Directory | State | Method | Artifact bytes |
| --- | --- | --- | ---: |
| `call-people-nonce` | People #1 / `e1e91429…` | `AccountNonceApi_account_nonce` (Alice) | 2,425 |
| `call-people-fee` | People #1 / `e1e91429…` | `TransactionPaymentApi_query_info` (the saved transfer, plus its length) | 2,009 |
| `call-people-metadata` | People #1 / `e1e91429…` | `Metadata_metadata_versions` | 1,662 |
| `call-westend-nonce` | Westend Asset Hub #18,561,759 / `60c39d38…` | `AccountNonceApi_account_nonce` (Alice) | 6,302 |
| `call-westend-dryrun` | Westend Asset Hub #18,563,012 | `DryRunApi_dry_run_call` (root `System.set_code([])`) | 5,066 |

The People calls run on the parent state of `people-transfer`, not on block #2's result. Their recorded results are local runs, not node data. Regenerate them offline:

```sh
cargo +1.93.0 run -p polkadot-frame-trace-replay --example record_calls
```

The Westend calls were recorded on 2026-10-25 with `--check-state-call`. The local result matched the node's `state_call` for both. The nonce call returns `0x08020000` (nonce 520) and reads one `System::Account`.

## Make a fixture

Save the block, then move large nodes into blobs with [`blobify.py`](blobify.py). It compresses with `zstd -19` and empties the stored metadata, because a replay builds metadata from the saved runtime. This is the command for `paseo-asset-hub-contract`:

```sh
cargo +1.93.0 build --release --locked -p polkadot-frame-trace
target/release/polkadot-frame-trace block --rpc wss://asset-hub-paseo-rpc.n.dwellir.com \
  --block 15366850 --rpc-timeout 120 --save <dir> --json <file>
python3 -I fixtures/replay/blobify.py <dir> fixtures/replay/paseo-asset-hub-contract fixtures/replay/blobs 100000
```

That node took about 35 seconds to send the proof of `:code`, so the run needs `--rpc-timeout 120`. On 2026-11-06 the replay passed the root check with 113 RPC calls, and `blobify.py` gave the same artifact and blobs byte for byte.

Runtime blobs do not shrink: the SDK already compresses `:code` with zstd.

## Blocks not committed

A runtime upgrade block and a relay chain block are not committed, because each runtime adds 1.5 to 2.7 MB. Ignored live tests replay them every night instead.

| Block | Why | Live test |
| --- | --- | --- |
| Westend Asset Hub #18,985,169 | Enacts spec 1025003 → 1025004. Needs both runtimes, about 5.3 MB. | `runtime_upgrade_enactment_and_next_block_live` |
| Westend Asset Hub #18,985,170 | First block on the new runtime. Was a fixture from 2026-11-06 to 2026-11-07, 3,122,414 bytes. | same |
| Westend relay chain #33,614,892 | Relay chain block. Was a fixture until 2026-11-07, 1,733,346 bytes. | `public_relay_network` (the finalized block) |

Offline, `one_session_replays_blocks_on_different_runtimes` replays blocks on two runtimes in one session, as `follow` does across an upgrade. To replay one of these blocks offline, save it again from an archive node:

```sh
polkadot-frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block 18985170 --save <dir>
polkadot-frame-trace block --offline <dir>
```
