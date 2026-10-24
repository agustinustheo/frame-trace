# E1: real People chain and `state_traceBlock`

Experiment run on 2026-10-25 on macOS arm64. All chains are real local omni-node databases executing People Wasm, with manual dev block authoring and immediate finalization. No relay chain, Zombienet, bootstrap or ZK proofs are required. All submitted extrinsics have decoded on-chain success events; sudo calls also have successful inner results.

## Findings that affect frame-trace

1. Pass **all four parameters**, including explicit empty strings: `[hash, "state", "", ""]`. Omitting the optional filters returns **zero events** in this SDK version.
2. Observed storage keys, Get/Put/Append hex bytes and trace hashes have **no `0x` prefix**. A storage-key filter beginning with `0x` matches nothing. The RPC block-hash argument still uses `0x`; source-only `Hash`/`ChildHash` results use the hash type's own Debug format instead of HexDisplay.
3. Even empty storage filters discard events without a `key`. The RPC therefore does **not return every host storage event**: `ClearPrefix`, `ChildKill`, `ChildClearPrefix` and storage-root events are dropped. `NextKey` is not instrumented in the first place. Runtime `Call`/`Return` events cannot be used as RPC boundaries either. The official inherent-only all-target fixture contains native `clear_prefix_version_2` and `root_version_2` spans, yet neither corresponding state event survives.
4. `parentId` is null in every observed event and span, including a runtime built with `with-tracing`. Span arrays are unordered and have no event offsets, entry/exit records, attributes or timing. Runtime spans alone cannot reconstruct a dispatch/hook tree containing storage events.
5. The official pinned omni-node **authors signed People blocks but cannot replay them** in dev mode: JSON-RPC error 4001, with a storage-root mismatch. Inherent-only blocks replay. The normal parachain path has a proof-aware replay executor; the dev path passes `None`. Wiring the existing executor into dev mode **restores successful replay of every case**, with both runtime variants. The five-line scratch-node diff is `omni-dev-proof-replay.patch`; official failures are retained separately. No RPC filtering/serialization code or runtime transaction behavior was changed.

## Versions and provenance

| Component | Version |
| --- | --- |
| Individuality source | `e69b079ef93b6b0224ca81b96d4385f6dd4c8e22` |
| Runtime package | `next-people-paseo-runtime` 0.1.0 |
| Runtime version | spec `next-people-paseo`, specVersion 3004000, transactionVersion 6, stateVersion 1 |
| SDK release | `polkadot-stable2606-2`, commit `72284b37a234f8a1565f2043ddbf8280a69fcf29` |
| Official omni-node | `1.24.2-72284b37a23`, aarch64-apple-darwin |
| Scratch proof-aware node | Same version string, same SDK tag, plus `omni-dev-proof-replay.patch` |
| Rust | `cargo +1.93.0`, wasm32-unknown-unknown |
| Node.js | 20.20.0 |
| polkadot-js | `@polkadot/api` 17.0.2; keyring/util-crypto 14.0.3 |

Official binary SHA-256:

```text
1fa97ad532da1f74eb51031f260fa48402a4bbbdaedd1198b2f65d936e9d37b1  polkadot-omni-node-aarch64-apple-darwin
cf05112e772a9a355384ce5b6950b9a074fb7fe061a89b012a8c62e79242570e  chain-spec-builder-aarch64-apple-darwin
```

Untraced Wasm SHA-256: `ddb7cf17b26d8aebf4b3e2572ab557818e911d7bf1d474473d11b50253270500`.
Traced Wasm SHA-256: `685f48ea8e82a27fdd95983cea5edc0e02b63ecd30b8c6b93290261dfc43720b`.
Scratch proof-aware binary SHA-256: `f8368897447837e575c48f4185e3da93b1c0fed5bbcff2bc6e7f1dc471bff738`.

Both final runtime builds use the committed Individuality dependency lock. Set `WASM_BUILD_WORKSPACE_HINT` to the actual workspace when using an external `CARGO_TARGET_DIR`: an initial attempt without it caused `substrate-wasm-builder` to resolve an independent Wasm lock despite the outer `--locked`. That attempt was rebuilt before the final fixtures were captured. The only runtime source change adds feature forwarding to `sp-io` and `frame-executive`; `sp-io` forwards to `sp-tracing`.

## Fixture format

Each chain directory contains `manifest.json`, `runtime-version.json`, `node-version.json`, `rpc-methods.json`, `measurements.json` and generated `analysis.json`. Each recorded block directory contains:

- `block.json`: `chain_getBlock` result, including encoded extrinsics.
- `header.json`: `chain_getHeader` result.
- `metadata.hex`: full `state_getMetadata(block_hash)` result, including `0x`; these fixtures use metadata V14 (489,775 decoded bytes).
- `extrinsics.json`, `events.json`: metadata-decoded calls and on-chain events.
- `trace-*.json`: the RPC **result payload**, preserving `{"blockTrace": ...}`. If the RPC fails, the file instead contains its complete JSON-RPC error envelope.
- `summary.json`: event indices for phase markers, observed methods/keys, filter counts and replay comparison.
- `probe-storage.json` for the raw storage write/deletion cases.

Measurement byte counts are the original compact HTTP response **including** the JSON-RPC envelope, not the size of the pretty-printed fixture. Latencies cover the complete local HTTP request, response transfer and text read. They are wall-clock RPC latency, not storage-operation timing, and were measured while builds sometimes ran on the same machine.

The **primary successful test fixtures** are `fixtures/next-people-paseo/` and `fixtures/next-people-paseo-traced/`, captured with the scratch proof-aware node. Both have blocks 1–8: inherents, transfer, remark, batch, `Sudo.sudo(PeopleLite.increase_attestation_allowance(Bob, 7))`, `Sudo.sudo(PeopleLite.clear_attestation_allowance(Bob))`, a 4 KiB raw write and prefix deletion. Every trace variant succeeds; no fallback runtime was used.

The official-binary comparison datasets are `fixtures/next-people-paseo-official/` and `fixtures/next-people-paseo-traced-official/`, with the same eight cases. The first block succeeds; the seven signed blocks preserve real replay errors. See each manifest for hashes and block numbers. Each chain also has `provenance.json` and a node log.

## Wire shape and exact storage fields

The RPC result has an enum wrapper and camelCase fields:

```json
{
  "blockTrace": {
    "blockHash": "<64 hex digits, no 0x>",
    "parentHash": "<64 hex digits, no 0x>",
    "tracingTargets": "state",
    "storageKeys": "",
    "methods": "",
    "spans": [],
    "events": [
      {
        "target": "state",
        "data": {"stringValues": {"key": "...", "value": "None", "value_encoded": "00", "ext_id": "...", "method": "Put"}},
        "parentId": null
      }
    ]
  }
}
```

A real traced span from `next-people-paseo-traced-official/1/trace-runtime.json`:

```json
{"id":52,"parentId":null,"name":"apply_extrinsic","target":"frame_executive","wasm":true}
```

There are no span values in this response, so the executive's encoded-extrinsic attribute is lost. Events have only `target`, `data.stringValues` and `parentId`; their name and level are lost. Every value inside `stringValues`, including booleans, is a JSON string.

Exact keys in **observed RPC events**:

| `method` | Complete `stringValues` key set | Value representation |
| --- | --- | --- |
| `Get` | `ext_id`, `key`, `method`, `result`, `result_encoded` | `Some(<hex>)` or `None`; encoded option in the other field |
| `Put` | `ext_id`, `key`, `method`, `value`, `value_encoded` | `Some(<hex>)` or `None`; `None` is deletion, not a separate `Clear` method |
| `Exists` | `ext_id`, `key`, `method`, `result` | `"true"` or `"false"`; it **does** include its result |
| `Append` | `ext_id`, `key`, `method`, `value` | Plain hex of the appended encoded item, not the resulting complete stored value |

For example, the inherent-only fixture contains:

```json
{"method":"Get","result":"Some(8259b700446e6578742d70656f706c652d706173656f)","result_encoded":"018259b700446e6578742d70656f706c652d706173656f"}
{"method":"Put","value":"Some(01000000)","value_encoded":"0101000000"}
{"method":"Put","value":"None","value_encoded":"00"}
{"method":"Exists","result":"false"}
```

These are excerpts of `stringValues`, omitting the key and externalities id for readability. `*_encoded` uses an option tag followed by opaque raw value bytes: `00` for None, `01` followed by the bytes for Some. There is **no SCALE Vec length prefix** between the option tag and the value.

The following schemas are **source-verified, not exercised by these People calls**. Do not treat them as recorded RPC coverage. They refer to the pinned node's `sp-state-machine` host code, not to assumptions about future SDK versions:

| Host method | Exact logged string keys | RPC visibility in this version |
| --- | --- | --- |
| `Hash` | `ext_id`, `key`, `method`, `result` | Can pass key/method filters; result is Debug-formatted `Option<H::Out>`, not `HexDisplay` and not the returned encoded `Vec<u8>` |
| `ChildGet` | `ext_id`, `child_info`, `key`, `method`, `result` | Can pass; `Some(<hex>)`/`None`, no `result_encoded` |
| `ChildHash` | `ext_id`, `child_info`, `key`, `method`, `result` | Can pass; Debug-formatted `Option<H::Out>`, as for `Hash` |
| `ChildExists` | `ext_id`, `child_info`, `key`, `method`, `result` | Can pass; `"true"`/`"false"` |
| `ChildPut` | `ext_id`, `child_info`, `key`, `method`, `value` | Can pass; `Some(<hex>)`/`None`, no `value_encoded` |
| `ClearPrefix` | `ext_id`, `method`, `prefix` | Dropped: no `key` |
| `ChildClearPrefix` | `ext_id`, `child_info`, `method`, `prefix` | Dropped: no `key` |
| `ChildKill` | `ext_id`, `child_info`, `method` | Dropped: no `key` |
| `StorageRoot` | `ext_id`, `method`, `storage_root`, `cached` | Dropped: no `key` |
| `ChildStorageRoot` | `ext_id`, `child_info`, `method`, `storage_root`, `cached` | Dropped: no `key` |
| `IndexTransaction` | `ext_id`, `method`, `index`, `tx_hash`, `size` | Dropped: no `key` |
| `RenewTransactionIndex` | `ext_id`, `method`, `index`, `tx_hash` | Dropped: no `key` |
| `NextKey`, child next-key | No tracing event | Not observable |

`HexDisplay` has two formats. Debug (`?`) prints the complete bytes; Display (`%`) prints at most the first and last 512 bytes with `...` between when input length is at least 1027 bytes. Consequently `Get.result` and `Put.value` retain complete bytes, while their `*_encoded` counterparts, `Append.value`, keys and child ids can be truncated. Do not infer a large value's length from the truncated field. In `next-people-paseo/7/trace-state.json`, key `3a6672616d652d74726163652d65313a01` has a 4,096-byte value: `value` is the complete 8,198-character `Some(a5a5...)` string, while `value_encoded` is only 2,051 characters and contains an ellipsis. `7/probe-storage.json` confirms the full value. The same result is recorded with tracing enabled.

The next block successfully executes `System.kill_prefix` under sudo. `8/probe-storage.json` is JSON null, confirming deletion, and `8/events.json` confirms dispatch success. Its all-target trace contains two native `clear_prefix_version_2` spans (normal initialization plus the submitted call), while `8/trace-clear-prefix-method.json` has zero events. This is a real deletion omitted from the storage event stream, not an unexercised operation.

## Ordering, extrinsic boundaries and phases

Event array order is emission order from the synchronous replay subscriber. Post-filters preserve that order. Replaying the same block gives equal events after ignoring `ext_id`; `verify.py` checks this. This is ordered **host access**, including overlay reads and attempted operations, not a database I/O log or an atomic diff. Transaction rollback/commit host spans can exist under all-target capture, but they have no positions in the event array.

Span arrays are drained from a hash map. Their array positions have no execution-order meaning. Span ids increase at creation, but provide neither exit points nor a mapping into events. All observed parent ids remain null even when runtime hooks and calls are active. The block subscriber stores explicit parents only; its enter/exit callbacks do nothing. No timestamp, duration, cache/DB source or per-operation proof-size field is returned.

`ext_id` is a random **u16 externalities-instance id**, displayed as its two little-endian bytes in hex. It is not the extrinsic index, span id, globally unique identifier or stable replay id. One replay's storage events share it across all extrinsics; another replay normally changes it. Collisions are possible.

Known marker keys:

```text
:extrinsic_index       3a65787472696e7369635f696e646578
System::ExecutionPhase 26aa394eea5630e07c48ae0c9558cef7ff553b5a9862a516939d82b3d3d8661a
```

In `next-people-paseo-official/1/trace-state.json`, zero-based event indices are:

| Index | Write | Interpretation |
| --- | --- | --- |
| 4 | ExecutionPhase `Some(02)` | Initialization |
| 5 | extrinsic index `Some(00000000)` | Index 0 is installed during initialization; this alone does not end initialization |
| 145 | ExecutionPhase `Some(0000000000)` | ApplyExtrinsic(0); initialization finished |
| 197 | extrinsic index `Some(01000000)` | Extrinsic 0 finished; next index is 1 |
| 198 | ExecutionPhase `Some(0001000000)` | ApplyExtrinsic(1) |
| 223 | extrinsic index `Some(02000000)` | Extrinsic 1 finished; next index is 2 |
| 224 | ExecutionPhase `Some(0002000000)` | ApplyExtrinsic(2), a terminal next-index marker here: block body has only indices 0 and 1 |
| 250 | extrinsic index `None` | End-of-extrinsics bookkeeping |
| 252 | ExecutionPhase `Some(01)` | Finalization |
| 302 | ExecutionPhase `None` | System finalization clears the phase; storage work still follows |
| 314 | Last returned event | Replay tail, after phase removal |

Phase discriminants are `00 + u32 LE` = ApplyExtrinsic(i), `01` = Finalization, `02` = Initialization. The initial events 0–3 are initialization setup before the phase write. Use the whole pre-ApplyExtrinsic(0) prefix as initialization, including that setup. `note_applied_extrinsic` advances the index **after** an extrinsic, not at its dispatch entry. Cross-check indices against the block body to avoid inventing a final extra extrinsic. Keep finalization bookkeeping between terminal ApplyExtrinsic(N), index deletion, Phase::Finalization and the final tail in an explicitly defined policy. A phase deletion does not mark the end of the trace.

The inherent-only body contains `ParachainSystem.set_validation_data` and `Timestamp.set`. Signed calls normally occupy index 2. A `Utility.batch` is one outer extrinsic with multiple transfers; the index must stay 2 throughout the nested calls. Per-block `summary.json` records all marker events, and decoded events retain the actual ApplyExtrinsic phase.

Concrete batch evidence: `next-people-paseo/4/summary.json` writes ApplyExtrinsic(2) at event 228 and advances to index 3 at event 441 / ApplyExtrinsic(3) at 442. There is no index change within those three transfers. `4/events.json` has three `Balances.Transfer` events and `Utility.BatchCompleted`, all at ApplyExtrinsic(2). It enters finalization at event 448 and clears the phase at 500.

## Runtime tracing

With targets `state` or `state,runtime,frame,pallet,indiv`, the original runtime produces zero spans. With targets `""`, even the original runtime produces native host-interface spans such as `sp_io::storage::get_version_1`, allocator and hashing spans (`wasm:false`), plus `block_trace::trace_block`. Thus “no spans without with-tracing” is only true for the selected runtime target prefixes.

The tracing runtime's inherent-only block produces 91 Wasm spans with the broad target list, including executive initialization/apply/final-check spans, pallet hooks and `Timestamp.set`. It produces zero spans with only target `state`. The default target list omits `indiv` and captures fewer runtime spans. Despite `wasm:true`, all parents are null. Storage events also remain unparented. The complete all-target capture includes additional native tracing-transport and host-interface spans.

The successful traced batch has 96 runtime spans. In `next-people-paseo-traced/4/trace-runtime.json`, ids 57–60 are `pallet_utility::pallet / batch` followed by three `pallet_balances::pallet / transfer_keep_alive` spans, each with `parentId:null`. They appear in arbitrary array positions. The 515 storage events also all have null parents. Successful grant/clear blocks expose the respective `indiv_pallet_people_lite::pallet` dispatch spans.

## Measurements

Representative explicit `state` calls on the successful untraced chain; complete measurements for every variant and both runtimes are in each chain's `measurements.json`:

| Block / work | Events | Compact HTTP response bytes | RPC latency (ms) |
| --- | ---: | ---: | ---: |
| 1 / inherents | 315 | 90,002 | 4.209 |
| 2 / transfer | 418 | 132,055 | 3.297 |
| 3 / remark | 391 | 117,300 | 2.891 |
| 4 / three-transfer batch | 515 | 169,498 | 3.794 |
| 5 / PeopleLite grant | 412 | 125,393 | 3.624 |
| 6 / PeopleLite clear | 411 | 125,012 | 3.574 |
| 7 / 4 KiB write | 406 | 153,858 | 4.047 |
| 8 / prefix deletion | 405 | 123,483 | 3.579 |

For the traced batch, broad runtime targets return 96 spans plus 515 events in 178,999 bytes / 3.808 ms. Capturing all targets expands that to 654,834 bytes / 6.234 ms. These are individual local observations, not throughput benchmarks or a claim about production latency.

## Unsafe RPC, pruning and errors

`fixtures/e1-probes/safe/trace-safe.json` records:

```json
{"error":{"code":4003,"message":"RPC call is unsafe to be called externally"}}
```

This was a loopback-only node explicitly started with `--rpc-methods safe`. The `author_insertKey` control was denied too. The check is in the outer RPC implementation (`substrate/client/rpc/src/state/mod.rs`), before `state_full.rs`; inspecting only the backend misses it. `--rpc-methods unsafe` works; `auto` on localhost may allow it but was not separately tested.

The default-pruning probe omitted `--state-pruning`, used 50 ms dev blocks and waited for at least 300 finalized blocks. Block #1 initially traced successfully. At finalized #302, replay failed with error 4001 containing `State already discarded for <genesis hash>`. A direct old-state read also failed, while a recent block still traced. Full error envelopes, before/after traces and head headers are in `fixtures/e1-probes/default-pruning/`. Replay needs the **parent's** state; retaining a block body and header is insufficient. Archive mode avoids this failure.

The official signed-block mismatch is a different 4001 error. `next-people-paseo-official/2/execute-error.json` records a direct `Core_execute_block` failure, and the diagnostic node log says `Storage root must match that calculated.` The actual transfer and its success event are present in `block.json` and `events.json`. Error 4001's generic “Cannot resolve a block range” wording does not imply the problem is pruning.

Default `--rpc-max-response-size` is 15 MiB; this run raises it to 64 MiB. Both HTTP and WS limits are configured by that option. Server-side filters run **after replay**, so they reduce payload size but do not avoid block execution or initial trace collection. The value strings can include account data. No live metrics, duration or exact cache-I/O claims should be derived from these fixtures.

The response-limit probe runs the proof-aware node with `--rpc-max-response-size 1` and submits 120 writes of 4,096 bytes each. `fixtures/e1-probes/response-limit/trace-full.json` returns JSON-RPC error `-32008`, `Response is too big`, data `Exceeded max limit of 1048576` (HTTP 200). Filtering to one exact key returns one event in 10,710 bytes. The respective RPC latencies are 22.197 and 21.075 ms: filtering does not eliminate the replay work. The fixture also retains its block, header and metadata. polkadot-js fell back to legacy V14 metadata initialization because its newer metadata runtime-API response exceeded the same limit.

## Reproduce

Run commands in a shell with Rust 1.93.0 and its Wasm target already installed. These paths keep node data, binaries, target files and clones under the authorized scratch directory. `run-node.py` owns each node and stops it in `finally`, including when its client command fails.

```bash
export E1=/Users/theo/Projects/parity/frame-trace/experiments/e1
export SCRATCH=/tmp/pm-audit/e1
export PEOPLE=/tmp/pm-audit/individuality-community
mkdir -p "$SCRATCH/bin" "$SCRATCH/client"

# Reuse the supplied shallow clone at e69b079; otherwise clone this revision.
git -C "$PEOPLE" rev-parse HEAD

gh release download polkadot-stable2606-2 --repo paritytech/polkadot-sdk \
  --pattern polkadot-omni-node-aarch64-apple-darwin \
  --pattern polkadot-omni-node-aarch64-apple-darwin.sha256 \
  --pattern chain-spec-builder-aarch64-apple-darwin \
  --pattern chain-spec-builder-aarch64-apple-darwin.sha256 \
  --dir "$SCRATCH/bin"
(cd "$SCRATCH/bin" && shasum -a 256 -c polkadot-omni-node-aarch64-apple-darwin.sha256)
(cd "$SCRATCH/bin" && shasum -a 256 -c chain-spec-builder-aarch64-apple-darwin.sha256)
chmod +x "$SCRATCH/bin/"*-aarch64-apple-darwin

cp "$E1/client-package.json" "$SCRATCH/client/package.json"
cp "$E1/client-package-lock.json" "$SCRATCH/client/package-lock.json"
npm ci --prefix "$SCRATCH/client" --no-audit --no-fund
export NODE_PATH="$SCRATCH/client/node_modules"

# Keep the Wasm builder on the committed lock even with an external target directory.
(cd "$PEOPLE" && SKIP_PALLET_REVIVE_FIXTURES=1 \
  CARGO_TARGET_DIR="$SCRATCH/target-people" WASM_BUILD_WORKSPACE_HINT="$PEOPLE" \
  cargo +1.93.0 build --release --locked -p next-people-paseo-runtime)
cp "$SCRATCH/target-people/release/wbuild/next-people-paseo-runtime/next_people_paseo_runtime.compact.compressed.wasm" \
  "$SCRATCH/people-untraced.wasm"

"$SCRATCH/bin/chain-spec-builder-aarch64-apple-darwin" -c "$SCRATCH/people-untraced.json" create \
  --chain-name 'E1 People' --chain-id e1-people -t development \
  --relay-chain paseo --para-id 1502 \
  --properties tokenSymbol=PAS,tokenDecimals=10,ss58Format=42 \
  --runtime "$SCRATCH/people-untraced.wasm" named-preset development

DIAGNOSE=1 LOG="$SCRATCH/node-official.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-untraced.json" -- \
  node "$E1/record.cjs" next-people-paseo-official

# Add tracing feature forwarding in the scratch clone only (apply once).
git -C "$PEOPLE" apply "$E1/people-with-tracing.patch"
(cd "$PEOPLE" && SKIP_PALLET_REVIVE_FIXTURES=1 \
  CARGO_TARGET_DIR="$SCRATCH/target-people" WASM_BUILD_WORKSPACE_HINT="$PEOPLE" \
  cargo +1.93.0 build --release --locked -p next-people-paseo-runtime --features with-tracing)
cp "$SCRATCH/target-people/release/wbuild/next-people-paseo-runtime/next_people_paseo_runtime.compact.compressed.wasm" \
  "$SCRATCH/people-traced.wasm"
"$SCRATCH/bin/chain-spec-builder-aarch64-apple-darwin" -c "$SCRATCH/people-traced.json" create \
  --chain-name 'E1 People traced' --chain-id e1-people-traced -t development \
  --relay-chain paseo --para-id 1502 \
  --properties tokenSymbol=PAS,tokenDecimals=10,ss58Format=42 \
  --runtime "$SCRATCH/people-traced.wasm" named-preset development
RPC_PORT=19947 LOG="$SCRATCH/node-traced-official.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-traced.json" -- \
  node "$E1/record.cjs" next-people-paseo-traced-official

# Build the dev-mode proof-aware executor wiring in a separate stable-tag clone.
git clone --depth 1 --branch polkadot-stable2606-2 \
  https://github.com/paritytech/polkadot-sdk.git "$SCRATCH/polkadot-sdk-stable"
git -C "$SCRATCH/polkadot-sdk-stable" apply "$E1/omni-dev-proof-replay.patch"
(cd "$SCRATCH/polkadot-sdk-stable" && SKIP_WASM_BUILD=1 \
  CARGO_TARGET_DIR="$SCRATCH/target-node" CARGO_BUILD_JOBS=8 \
  cargo +1.93.0 build --release --locked -p polkadot-omni-node)

OMNI_NODE="$SCRATCH/target-node/release/polkadot-omni-node" LOG="$SCRATCH/node-proof-aware.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-untraced.json" -- \
  node "$E1/record.cjs" next-people-paseo
RPC_PORT=19947 OMNI_NODE="$SCRATCH/target-node/release/polkadot-omni-node" \
  LOG="$SCRATCH/node-traced-proof-aware.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-traced.json" -- \
  node "$E1/record.cjs" next-people-paseo-traced
RPC_PORT=19948 RPC_MAX_RESPONSE_SIZE=1 OMNI_NODE="$SCRATCH/target-node/release/polkadot-omni-node" \
  LOG="$SCRATCH/node-response-limit.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-untraced.json" -- node "$E1/response-limit.cjs"

# Safe RPC and default pruning are independently exercised on the official binary.
RPC_PORT=19945 BLOCK_TIME=100 RPC_METHODS=safe LOG="$SCRATCH/node-safe.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-untraced.json" -- \
  python3 "$E1/probe.py" safe /Users/theo/Projects/parity/frame-trace/fixtures/e1-probes/safe
RPC_PORT=19946 BLOCK_TIME=50 STATE_PRUNING=default LOG="$SCRATCH/node-pruning.log" \
  python3 "$E1/run-node.py" "$SCRATCH/people-untraced.json" -- \
  python3 "$E1/probe.py" pruning /Users/theo/Projects/parity/frame-trace/fixtures/e1-probes/default-pruning

python3 "$E1/provenance.py"
python3 "$E1/verify.py"
```

`record.cjs` uses polkadot-js to sign real calls with `//Alice`, checks their canonical block bodies and dispatch-success events, then uses raw HTTP JSON-RPC for fixture capture. Legacy submit-and-watch notifications were unreliable with immediate dev finalization, so submission inclusion is verified by scanning canonical blocks. Each trace variant records its precise request parameters and response size/latency. Block timestamps, signatures, hashes, externalities ids and span array order vary on reruns; the scripts reproduce the experiment, not identical blocks.

## Pinned source references

- [RPC response types](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/primitives/rpc/src/tracing.rs).
- [Block subscriber, prefix filters and result conversion](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/client/tracing/src/block/mod.rs).
- [Host storage events and externalities id](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/primitives/state-machine/src/ext.rs).
- [HexDisplay truncation](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/primitives/core/src/hexdisplay.rs).
- [Outer unsafe-RPC gate](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/substrate/client/rpc/src/state/mod.rs).
- [Dev-mode replay wiring](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/cumulus/polkadot-omni-node/lib/src/nodes/aura.rs) and [parachain replay executor](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2606-2/cumulus/client/service/src/lib.rs).

Phase markers and transaction-extension behavior were also checked against `frame-system` 48.0.0, `frame-executive` 48.0.0 and `cumulus-pallet-weight-reclaim` 0.12.0 in the local crates.io sources used by the People runtime.
