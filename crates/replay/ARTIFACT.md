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
| `metadata` | Hex SCALE metadata fetched at the parent hash, for decoding only |
| `proof_nodes` | Array of hex-encoded raw trie nodes and external values, deduplicated by Blake2 hash |

Nodes share one content-addressed namespace. SDK `KeySpacedDB` supplies the child
namespace on misses; child requests strip that keyspace and use the prefixed
`:child_storage:default:` key with `state_getChildReadProof`. The top-level child-root
entry is loaded first using `state_getReadProof`. SDK traversal authenticates child
nodes against that entry and the published parent state root. An RPC proof's `at`
must equal the selected parent hash.

The runtime's `:code` and `:heappages` are obtained through the authenticated trie.
Metadata and runtime-version JSON never influence execution. The original header
hash is checked before seals are removed for execution, as in SDK block replay.
Every execution uses a fresh overlay, recorder and `ProofSizeExt`. Loading code uses
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
the same host functions and proof-size extension. Discovery authenticates missing
reads; the measured pass disables RPC. A stored result must match re-execution.
The optional oracle is a separate `state_call` response, never execution input.
Offline replay can compare against those recorded bytes without contacting a node.
Calls do **not** assert a new block root. They authenticate reads relative to the
selected header's state root; metadata and node oracle bytes are auxiliary data.

Follow predictions retain up to 4,096 top-trie keys/miss paths per successful
replay, across the last three completions. This includes full storage keys and
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
