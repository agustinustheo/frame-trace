# polkadot-omni-node `--dev` cannot replay blocks with signed extrinsics

DRAFT. Not filed.

## Summary

In `--dev` mode, `polkadot-omni-node` passes `tracing_execute_block: None`. The parachain path passes `ParachainTracingExecuteBlock`, which records storage proofs and registers `ProofSizeExt` during re-execution. Without it, `state_traceBlock` and direct `Core_execute_block` fail on any block with a signed extrinsic, with a storage-root mismatch. Inherent-only blocks replay correctly. Passing the existing executor to the dev path makes replay succeed in our tests.

## Versions

- polkadot-omni-node 1.24.2-72284b37a23 (`polkadot-stable2606-2`, `72284b37a234f8a1565f2043ddbf8280a69fcf29`), official aarch64-apple-darwin binary.
- Runtime `next-people-paseo-runtime` 0.1.0, spec_version 3004000, transaction_version 6, state_version 1.
- Line numbers: polkadot-sdk master `14f57c4e28081367fbfb21cf845eb34d043865d5` (2026-10-24). The stable tag is not in our clone.
- Rust 1.93.0, Node 20.20.0, `@polkadot/api` 17.0.2. macOS arm64 only.

## Steps to reproduce

1. Build the People runtime from `individuality` at `e69b079`: `cargo build --release --locked -p next-people-paseo-runtime`.
2. Create a dev chain spec with `chain-spec-builder` (`--relay-chain paseo --para-id 1502`, `named-preset development`).
3. Start the official binary:
   ```bash
   polkadot-omni-node --chain people.json --dev --dev-block-time 1000 --rpc-methods unsafe --tmp
   ```
4. With `@polkadot/api`, submit `balances.transferKeepAlive(Bob, 1)` signed by `//Alice`. Note the hash `H` of the block that contains it.
5. Call `state_traceBlock(H, "state", "", "")`.
6. Optional: call `Core_execute_block` through `state_call` at the parent of `H`.

Repeat step 5 for the inherent-only block 1. It succeeds.

## Expected

`{"blockTrace": ...}` with the storage events for block `H`.

## Actual

`state_traceBlock` returns error 4001:

```text
Cannot resolve a block range [...]. Dispatch error: Failed to collect traces and execute block:
RuntimeApiError(... AbortedDueToTrap ... wasm `unreachable` instruction executed ...)
```

Step 6 returns error 4003 with the same trap. The node log contains `Storage root must match that calculated.`

Fixtures under `/Users/theo/Projects/parity/frame-trace/fixtures/next-people-paseo-official/`: `2/trace-state.json`, `2/execute-error.json`, `node.log` line 36. Block 1 is in `1/`.

## Cause

Line numbers on master:

- `cumulus/polkadot-omni-node/lib/src/nodes/aura.rs:465` calls `spawn_tasks` inside `start_dev_node` (line 215). Line 478 sets `tracing_execute_block: None`.
- `cumulus/polkadot-omni-node/lib/src/common/spec.rs:601` passes `ParachainTracingExecuteBlock` on the normal path.
- `cumulus/client/service/src/lib.rs:620-631` records a proof and registers `ProofSizeExt`.
- `substrate/client/tracing/src/block/mod.rs:102-106` (`DefaultExecuteBlock`) only calls `execute_block`.

Likely mechanism, not confirmed by a trace: `StorageWeightReclaim` reads the proof size. It is a transaction extension, so it runs only for signed extrinsics, which fits the inherent-only pass. The SDK PoV reclaim guide also requires import recording for this (`docs/sdk/src/guides/enable_pov_reclaim.rs:45-56`).

## Suggested fix

Pass the existing executor in dev mode. The patch below targets stable2606-2. It applies to master with a 2-line offset (`git apply --check`). It is a scratch change and has not run through CI. A regression test should replay a dev block that contains a signed extrinsic.

```diff
--- a/cumulus/polkadot-omni-node/lib/src/nodes/aura.rs
+++ b/cumulus/polkadot-omni-node/lib/src/nodes/aura.rs
@@ -459,6 +459,7 @@
 		let database_path = config.database.path().map(|p| p.to_path_buf());
+		let tracing_client = client.clone();
 
 		let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
@@ -473,7 +474,9 @@
 			telemetry: telemetry.as_mut(),
-			tracing_execute_block: None,
+			tracing_execute_block: Some(Arc::new(
+				cumulus_client_service::ParachainTracingExecuteBlock::new(tracing_client),
+			)),
 		})?;
```

## Related

- #7270 (open): `state_traceBlock` "Cannot resolve a block range" on a Westend Coretime archive node. Same error family, parachain path.
- #8845 and #9214 (both closed, unmerged): proof recording for `state_traceBlock` on `StorageWeightReclaim` chains. #9214 was closed in favour of #9871.
- #9871 (merged 2025-10-10): "`trace_block`: Support overwriting `execute_block`". It added the override. The dev path still passes `None`.
- #12374 (merged 2026-08-03): same mechanism in `pallet-revive` replay, fixed through `state_callRecorded`. The omni-node dev path is unchanged.
- #5835 (closed): "Storage root must match" on parachain import. A different path.

Searches with no dev-mode match: `traceBlock`, `trace_block`, `storage root must match`, `omni-node dev replay`, `trace block dev mode`, `dev mode state_traceBlock`, `ProofSizeExt dev`.
