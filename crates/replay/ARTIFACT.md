# Replay artifact v1

A directory contains `artifact.json.zst`: one Zstandard-compressed UTF-8 JSON object.
It is portable and has no external paths. `Artifact::save` writes a complete temporary
file before replacing it. `Artifact::load` rejects unknown versions. It bounds decompressed JSON to 64 MiB,
proof count to 100,000 and decoded proof bytes to 24 MiB. Every file error includes
its path. Repository fixtures may use `blob:<blake2-256>` strings in metadata and
proof nodes, rehydrated from the adjacent `blobs/<hash>.zst` directory (16 MiB per
blob, verified by hash). Saved user artifacts never contain these references.

| Field | Meaning |
| --- | --- |
| `format_version` | Integer `1` |
| `block_hash` | 0x-prefixed Blake2-256 hash of the original SCALE header, including seals |
| `block` | `chain_getBlock.result.block`, with SDK u32 header and opaque extrinsics |
| `parent_header` | SDK JSON header; its hash must equal `block.header.parentHash` |
| `runtime_version` | `state_getRuntimeVersion` at the parent hash, for provenance |
| `metadata` | Hex SCALE metadata derived locally at the parent/state hash; stored bytes are auxiliary and re-derived before decoding |
| `proof_nodes` | Array of hex-encoded raw trie nodes and external values, deduplicated by Blake2 hash |

Nodes share one content-addressed namespace. SDK `KeySpacedDB` supplies the child
namespace on misses; child requests strip that keyspace and use the prefixed
`:child_storage:default:` key with `state_getChildReadProof`. The top-level child-root
entry is loaded first using `state_getReadProof`. SDK traversal authenticates child
nodes against that entry and the published parent state root. An RPC proof's `at`
must equal the selected parent hash.

The runtime's `:code` and `:heappages` are obtained through the authenticated trie.
Stored metadata and runtime-version JSON are never trusted execution inputs.
JSON call arguments are encoded using locally executed metadata before the call. The original header
hash is checked before seals are removed for execution, as in SDK block replay.
Every execution uses a fresh overlay and backend recorder. The stable2609 default
`ExecutionExtensions` factory is empty, then adds executor-backed
`ReadRuntimeVersionExt`: both import and ordinary `state_call` receive that reader.
Import with `enable_import_proof_recording` additionally supplies `ProofSizeExt`;
ordinary `state_call` supplies none. These are the two policies frame-trace mirrors.
No node-specific custom extension factory, keystore or offchain-worker capability
is inferred from RPC. Unsupported custom host/extension requirements fail explicitly.
See SDK [execution extensions](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2609/substrate/client/api/src/execution_extensions.rs),
[call executor](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2609/substrate/client/service/src/client/call_executor.rs)
and [import path](https://github.com/paritytech/polkadot-sdk/blob/polkadot-stable2609/substrate/client/service/src/client/client.rs). Loading code uses
a separate backend without that recorder. Full-key hints acquire a lookup's path
in one request; a per-replay miss gate deduplicates concurrent strict cache misses.
Predicted keys are batched 32 at a time, across a bounded connection pool (default
8). Discarded speculative runs can gather further missing paths; only strict runs
produce accepted roots/captures. Shared nodes are authenticated anew at each root.
The SDK executor's code-hash cache is reused in memory and Wasmtime caches compiled
modules on disk. Neither changes execution overlays or recorder contents.

Online replay runs discovery, disables fetching, and repeats `Core_execute_block`.
Offline replay does both passes with fetching disabled. Missing proof nodes are
specific hash/path errors, guarded against SDK abort-on-panic hooks. It never constructs a network client. Success means the runtime accepted
the block, including its own state and extrinsic-root checks; neither assertion
nor signature checking is disabled. This verifies the transition relative to the
provided header/state root. The captured computed root is also compared explicitly
with the header root and both are reported. It does not independently establish chain consensus.

The capture is execution output, not trusted artifact input. It retains every
`state` event, raw backend iteration attempts, span fields/parents, enter/exit
transitions and monotonic nanosecond offsets. Native host-call intervals time
storage operations; backend attempts use their own measured durations. With Wasm
tracing, `apply_extrinsic` spans also retain recorder size before and after entry.
No warm timing includes network work.

## Calls

`kind` defaults to `"block"` for legacy v1 artifacts; new block saves explicitly
write it. A call uses `kind: "call"` and a `call` object with `method`, hex SCALE
`data`, optional hex `result`, and optional hex node `oracle`. `block.header` and
`parent_header` both hold the selected **state header**, `block.extrinsics` is empty,
and `block_hash` must equal that header's hash. Runtime version, metadata and
proof acquisition are pinned to that hash. The duplicate header keeps v1's proof
container and bounded/atomic/blob I/O shared; it is not a block transition input.
Kind/payload mismatches and nonempty call block bodies are rejected.

Calls execute the requested export with `CallContext::Offchain`, fresh overlays,
the same host functions and the ordinary-call extension policy above. A backend
recorder reports proof bytes without exposing a proof-size extension to the runtime. Discovery authenticates missing
reads; the measured pass disables RPC. A stored result must match re-execution.
The optional oracle is a separate `state_call` response, never execution input.
Offline replay labels this comparison **recorded (unverified)**. Oracle bytes
are editable artifact input, not an authenticated node attestation; a comparison
only checks equality to those bytes. No network request is made.
Calls do **not** assert a new block root. They authenticate reads relative to the
selected header's state root. Metadata is re-executed locally (V16, V15, legacy V14)
for names and argument/result types. The code-hash cache retains proof dependencies
per state root. Only explicit `--rpc-metadata-fallback` allows unauthenticated RPC
or stored metadata when local execution is impossible.

Follow predictions retain up to 4,096 top-trie keys/miss paths per successful
replay, across the last three completions (the combined per-replay prediction set is also
capped at 4,096, and concat-map families at 256). Families are built once per replay. This includes full storage keys and
sibling-prefix requests from iteration/root calculation. Before loading runtime
code at the next state root, prefetch batches them with metadata storage prefixes
using the bounded proof pool. Covered paths (including proven absence) are skipped.
No guessed value is accepted: content hashes plus traversal from the new root
authenticate every used node. Child paths continue to use namespaced demand fetching.
A new identifier in a single-key `Blake2_128Concat`/`Twox64Concat` layout can also
predict related map reads observed in that history: the concat hash must match,
the encoded identifier is exactly 32 bytes, and at most 31 additional keys join
one proof request. Identifiers with equal byte widths may have different meanings;
wrong guesses cost bounded bandwidth and cannot supply unauthenticated values.
This also expands pending keys from discarded dry runs before resuming execution.
`proof_round_trips` counts sequential proof request groups (one per parallel group,
one per strict demand fetch); it excludes context RPC and retry attempts.

Artifacts export only nodes read by the measured execution and code/metadata
bootstrap. Speculative, prefetch and cache-coverage probes cannot add unused nodes
to the artifact. Shared acquisition caches may retain those nodes for future runs.
