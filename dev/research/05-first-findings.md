# First findings from frame-trace

Run on 25 October 2026 (Asia/Ho_Chi_Minh). Source commit: `62e50438fca3e06683149819b66ff2db3e85f734`.

- **Replay works on public chains.** All 65 sampled blocks passed the real state-root check. This includes 42 Westend Asset Hub blocks, 20 Paseo People Next blocks, and three local People fixtures. Every measured replay made zero RPC calls.
- **Blocks with little user traffic still do substantial work.** Initialization and finalization account for 83.1% of Westend storage operations and 71.3% on People Next. These phase totals include framework bookkeeping as well as hooks.
- **Weight bookkeeping is a visible fixed cost.** Every sampled People Next block reads and writes `System::BlockWeight` 50 times during initialization. Westend does 72 reads and 71 writes. These are logical storage calls, often touching an overlay value.
- **Westend finalization maintains staking data even without a staking extrinsic.** `VoterList::ListNodes` averages 42.0 reads and 10.0 writes per block. Finalization contributes 56.4% of measured storage-call time.
- **Repeated reads are common.** Westend averages 183.6 per block; People Next averages 77.2. The fixture's three-transfer `Utility.batch` has 146 reads, including 129 repeats. Alice's account alone is read 48 times.
- **Parachain plumbing leads byte volume.** `System::ExtrinsicData` averages 12,427 bytes read per Westend block. `ParachainSystem::RelayStateProof` averages 11,409 bytes written. These are recorded value lengths, not database traffic.
- **The individuality fixtures mostly show idle costs.** `NftCredits.on_initialize` makes one read. All other individuality initialize/finalize hooks make zero storage calls in the traced fixture. No fixture or sampled live block contains a `PeopleLite` call. The sample cannot price registration, attestation, or ring maintenance.

## Samples and phase shares

| Sample | Finalized heights / fixture | Blocks | Signed extrinsics | Runtime |
| --- | --- | --- | --- | --- |
| Westend Asset Hub | 18,563,569–18,563,608; plus 18,563,565 and 18,563,535 | 42 | 3 | westmint 1025002 |
| Paseo People Next | 2,227,649–2,227,668 | 20 | 0 | next-people-paseo 3004000 |
| Offline People | transfer #2; traced batch #4; kill-prefix #8 | 3 | 3 | next-people-paseo 3004000 |

Westend's signed blocks are #18,563,535, #18,563,565, #18,563,595. Block bodies determined signed status, using the signed bit after the SCALE length prefix. A backward scan inspected 74 Westend bodies, 100 People Next bodies, and 180 standard Paseo People bodies. Neither People scan found a signed extrinsic. Only People Next was replayed for the public People sample.

| Sample | Phase | Ops/block | Ops share | Storage µs/block | Storage time share |
| --- | --- | --- | --- | --- | --- |
| Westend AH | Initialization | 224.1 | 39.8% | 998.8 | 21.2% |
| Westend AH | Inherents | 75.7 | 13.4% | 961.6 | 20.4% |
| Westend AH | Signed extrinsics | 7.7 | 1.4% | 52.0 | 1.1% |
| Westend AH | Post-inherent on_poll | 12.0 | 2.1% | 40.0 | 0.8% |
| Westend AH | Finalization | 243.7 | 43.3% | 2,658.1 | 56.4% |
| People Next | Initialization | 152.0 | 47.1% | 461.3 | 24.9% |
| People Next | Inherents | 78.6 | 24.4% | 648.5 | 35.0% |
| People Next | Signed extrinsics | 0.0 | 0.0% | 0.0 | 0.0% |
| People Next | Post-inherent on_poll | 13.9 | 4.3% | 50.0 | 2.7% |
| People Next | Finalization | 77.9 | 24.2% | 690.8 | 37.3% |
| People batch fixture | Initialization | 152.0 | 29.3% | 274.7 | 21.6% |
| People batch fixture | Inherents | 80.0 | 15.4% | 173.0 | 13.6% |
| People batch fixture | Signed extrinsics | 200.0 | 38.6% | 475.2 | 37.3% |
| People batch fixture | Post-inherent on_poll | 14.0 | 2.7% | 23.8 | 1.9% |
| People batch fixture | Finalization | 72.0 | 13.9% | 327.1 | 25.7% |

“Storage time” is the union of measured storage-operation intervals in each phase. Nested backend intervals count once. Shares divide by total measured storage time, not total block time. Compute between storage calls is excluded. The JSON has no explicit phase wall-time field. Public runtimes had no Wasm spans, so full execution-time shares cannot be established. `on_poll` stays separate from the first signed extrinsic, where it can be nested.

## Costs paid before or after calls

People Next initialization has 152 operations in every sampled block. Of these, 100 are `System::BlockWeight` reads/writes. Westend's baseline initialization has 222 operations, including 143 accesses to that item. Some blocks add a scheduler agenda read and clear. At #18,563,588, initialization rises to 286 operations and touches `Dap::LastIssuanceTimestamp`, `Dap::BudgetAllocation`, account balances, and issuance. This is extra phase work beyond the fixed weight bookkeeping. A duplicate flag does not imply that a value stayed unchanged: weight is updated between many reads.

The traced fixture identifies actual hook ownership. R/W means reads/writes. Clears count as writes. Times include each hook's own execution and tracing overhead.

| Pallet | Init R/W | Init span µs | Finalize R/W | Finalize span µs |
| --- | --- | --- | --- | --- |
| ParachainSystem | 6/9 | 40.25 | 19/15 | 114.38 |
| Authorship | 7/3 | 37.88 | 0/1 | 2.17 |
| Aura | 3/1 | 11.33 | 0/0 | 0.33 |
| CumulusAuraExt | 1/0 | 5.29 | 1/1 | 9.04 |
| PolkadotXcm | 2/2 | 10.12 | 0/0 | 24.71 |
| MessageQueue | 1/0 | 6.54 | 0/0 | 0.46 |
| TransactionPayment | 0/0 | 0.33 | 3/1 | 15.96 |

`ParachainSystem` is the largest storage-using hook in this fixture: 15 initialize operations and 34 finalize operations. `Authorship` initializes with 10. Hook totals exclude framework work outside those spans. The broader initialization phase has 152 operations, so attributing all of it to pallet hooks would overstate their work.

| Individuality pallet | Init R/W | Init span µs | Finalize R/W | Finalize span µs |
| --- | --- | --- | --- | --- |
| IndivRelayRandomness | 0/0 | 0.625 | 0/0 | 0.583 |
| NetworkSuffix | 0/0 | 0.334 | 0/0 | 0.417 |
| OriginRestriction | 0/0 | 0.458 | 0/0 | 0.458 |
| People | 0/0 | 0.375 | 0/0 | 0.333 |
| MobRule | 0/0 | 0.458 | 0/0 | 0.333 |
| ProofOfInk | 0/0 | 0.458 | 0/0 | 0.333 |
| Game | 0/0 | 0.500 | 0/0 | 0.333 |
| Score | 0/0 | 0.458 | 0/0 | 0.375 |
| NftCredits | 1/0 | 5.208 | 0/0 | 0.500 |
| DummyDim | 0/0 | 0.542 | 0/0 | 0.541 |
| PeopleLite | 0/0 | 0.500 | 0/0 | 0.375 |
| Resources | 0/0 | 0.500 | 0/0 | 0.417 |
| ChunksManager | 0/0 | 0.458 | 0/0 | 0.375 |
| Members | 0/0 | 0.375 | 0/0 | 0.541 |
| Coinage | 0/0 | 0.375 | 0/0 | 0.417 |
| MembersNotifier | 0/0 | 0.500 | 0/0 | 0.500 |
| Airdrop | 0/0 | 0.333 | 0/0 | 0.500 |
| Honour | 0/0 | 0.375 | 0/0 | 0.542 |
| PeopleAirdrops | 0/0 | 0.542 | 0/0 | 0.375 |

`NftCredits` reads `CreditBuffers(3)` in the traced fixture. Its 5.208 µs hook includes a 2.791 µs storage interval. Live People Next reads `CreditBuffers(block_number - 1)` once per block; for example `CreditBuffers(2227667)` at #2,227,668. Its live `on_poll` phase also reads the four-byte `Members::ActiveMembers` entry for `pop:polkadot.network/people` padded with five spaces. The live runtime lacks spans, so this is phase attribution rather than direct hook attribution.

## Repeated reads

| Sample | Phase | Repeated item | Extra reads/block |
| --- | --- | --- | --- |
| Westend AH | initialization | System::BlockWeight | 71.0 |
| Westend AH | finalization | VoterList::ListNodes | 30.0 |
| Westend AH | finalization | VoterList::Lock | 10.0 |
| Westend AH | finalization | VoterList::PendingRebag | 10.0 |
| Westend AH | finalization | Staking::Bonded | 10.0 |
| People Next | initialization | System::BlockWeight | 49.0 |
| People Next | finalization | ParachainSystem::RelevantMessagingState | 5.0 |
| People Next | finalization | System::Digest | 3.0 |
| People Next | initialization | System::Digest | 2.0 |
| People Next | finalization | System::BlockWeight | 2.0 |

These are extra read calls to the same full key within one phase. The traced batch has 47 extra reads of `System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)` (Alice). Bob has 15 extras and Charlie seven. Other batch repeats include `System::Number` (15), `System::ExecutionPhase` (12), and `System::EventCount` (12).

As a rough upper bound on avoidable repeated work, the observed repeats are 183.6/Westend block and 77.2/People Next block. Requiring no intervening write to that key lowers the estimates to 104.3 and 19.9. This still does not establish that any read can safely be removed. It mixes `get`, `exists`, and `hash`, and does not model runtime control flow or transaction rollback.

The export cannot say whether a read hit the overlay, a backend cache, or the trie. Repeated reads are often cached. Proof-recorder growth is not a cache-hit flag. No database I/O saving is claimed.

## Bytes and proof size

| Sample | Storage item | Reads/block | Writes/block | Read B/block | Written B/block |
| --- | --- | --- | --- | --- | --- |
| Westend AH | System::ExtrinsicData | 2.1 | 4.1 | 12,426.6 | 12,426.6 |
| Westend AH | ParachainSystem::RelevantMessagingState | 7.0 | 2.0 | 8,274.0 | 2,364.0 |
| Westend AH | VoterList::ListNodes | 42.0 | 10.0 | 2,508.0 | 1,140.0 |
| Westend AH | ParachainSystem::RelayStateProof | 0.0 | 1.0 | 0.0 | 11,409.0 |
| People Next | System::ExtrinsicData | 2.0 | 4.0 | 11,184.0 | 11,184.0 |
| People Next | ParachainSystem::RelevantMessagingState | 6.0 | 2.0 | 1,920.0 | 640.0 |
| People Next | ParachainSystem::UnincludedSegment | 3.0 | 3.0 | 996.5 | 658.0 |
| People Next | ParachainSystem::RelayStateProof | 0.0 | 1.0 | 0.0 | 10,177.0 |
| Batch fixture | System::Account | 74.0 | 9.0 | 5,200.0 | 720.0 |
| Batch fixture | System::ExtrinsicData | 3.0 | 6.0 | 1,851.0 | 1,851.0 |
| Batch fixture | System::BlockWeight | 60.0 | 58.0 | 724.0 | 692.0 |
| Batch fixture | ParachainSystem::RelayStateProof | 0.0 | 1.0 | 0.0 | 980.0 |

Concrete Westend examples include seven reads of `ParachainSystem::RelevantMessagingState` at #18,563,608. Each returned 1,182 bytes, for 8,274 bytes total. One voter-list key, `VoterList::ListNodes(5H8tQj8FUiu1A8gx1aMEa1Cuw8ReQefz2zfuBZWP6zPDhf8b)`, is read and existence-checked repeatedly during finalization. Its returned node value is 114 bytes. Full raw keys and ordered small extracts are in [key-examples.json](data/05/key-examples.json).

| Sample | Median execution proof B | Range B | Median replay ms |
| --- | --- | --- | --- |
| Westend AH | 91,313.0 | 85,500–100,957 | 12.7 |
| People Next | 13,412.5 | 13,080–13,543 | 5.6 |
| Fixture transfer | 6,479 | 6,479–6,479 | 4.2 |
| Fixture kill-prefix | 6,867 | 6,867–6,867 | 4.0 |
| Fixture batch | 6,586 | 6,586–6,586 | 5.3 |

Per-item proof sizes are not exported, so storage items cannot be ranked by proof contribution. The traced fixture's ordered `apply_extrinsic` spans add 495 bytes for validation data, 139 for timestamp, and 1,513 for the signed batch. These are marginal recorder growth, not standalone proofs. Nested dispatch spans show 166 added bytes in the first transfer and zero in the next two; previously recorded paths are reused.

## Typical extrinsics

Numbers are medians over occurrences. Each top-level row includes the signed extensions and execution bookkeeping attributed to that extrinsic. `on_poll` is excluded. Time is measured storage time, not complete dispatch time.

| Sample | pallet.call | n | Reads | Writes | Read B | Written B | Storage µs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Westend AH | ParachainSystem.set_validation_data | 42 | 23.0 | 26.0 | 1,355.0 | 26,256.0 | 936.4 |
| Westend AH | Timestamp.set | 42 | 14.0 | 12.0 | 59.0 | 72.0 | 71.7 |
| Westend AH | Balances.transfer_keep_alive | 3 | 81 | 27 | 3,537 | 929 | 665.6 |
| People Next | ParachainSystem.set_validation_data | 20 | 25.0 | 27.0 | 1,260.0 | 22,666.0 | 460.0 |
| People Next | Timestamp.set | 20 | 14.0 | 12.0 | 59.0 | 72.0 | 37.3 |
| Fixture transfer | Balances.transfer_keep_alive | 1 | 76 | 27 | 2,995 | 836 | 261.8 |
| Fixture kill-prefix | Sudo.sudo | 1 | 63 | 28 | 2,025 | 683 | 202.9 |
| Fixture batch | Utility.batch | 1 | 146 | 54 | 5,715 | 1,683 | 475.2 |

Inside the traced batch, the three `Balances.transfer_keep_alive` dispatch spans have reads/writes of 25/6, 31/11, and 25/6. They read 1,557, 1,103, and 1,557 bytes. They write 260, 444, and 260 bytes. Full dispatch times are 132.250, 135.583, and 119.083 µs. These exclude the outer transaction checks and fee processing. The full batch extrinsic span is 729.917 µs; its inner `Utility.batch` dispatch span is 434.125 µs.

## Other observations

- Westend clears `Revive::EthereumBlock` in initialization and writes an 836-byte value in finalization, including blocks with only the two parachain/timestamp inherents. EVM bookkeeping exists even when no EVM call is present.
- Initialization clears the `System::EventTopics` prefix. The kill-prefix fixture's `Sudo.sudo` reaches `System.kill_prefix`; it exposes `clear_prefix` plus backend iterator attempts. An attempt is not proof that a key was returned or deleted after overlay handling.
- `System::ExtrinsicData` carries the large validation-data inherent through block bookkeeping. Large byte totals can describe transient overlay values rather than persistent writes.
- The three fixtures contain a signed transfer, a signed sudo prefix-clear, and a signed three-transfer batch. They exercise the individuality runtime, but contain no individuality extrinsics. Idle hooks say little about a bootstrapped network under load.

## Method and limits

I read `README.md`, `SPEC.md`, and the replay-fixture notes. I built the pinned commit with the documented toolchain:

```sh
CARGO_TARGET_DIR=/tmp/pm-audit/ft-findings-target cargo +1.93.0 build --release --locked
```

I anchored each live selection to `chain_getFinalizedHead`, walked parent hashes, and inspected `chain_getBlock` bodies. I replayed the fixed hashes with the release binary's `block --json`, `--cache-dir /tmp/pm-audit/ft-findings/cache`, and `--proof-concurrency 2`. At most two replay processes ran together. This bounds proof concurrency to four across the sample.

The main endpoints were `wss://westend-asset-hub-rpc.polkadot.io` and `wss://paseo-people-next-system-rpc.polkadot.io`. Standard Paseo People worked at `wss://people-paseo.gatotech.network`, `wss://people-paseo.rotko.net`, and `wss://rpc.interweb-it.com/people-paseo`. Westend People worked at `wss://westend-people-rpc.polkadot.io` and `wss://people-westend-rpc.n.dwellir.com`. Endpoint candidates came from the [polkadot-js endpoint configuration](https://github.com/polkadot-js/apps/blob/master/packages/apps-config/src/endpoints/testingRelayPaseo.ts) and existing research; each listed success was checked directly.

I ran `summary` on both public sample chains and the traced fixture. Cached summary replays of the same public hashes needed three RPC calls and zero acquisition rounds; their measured runs still used zero RPC. Old fixture naming required `--rpc-metadata-fallback`. The CLI labelled those stored metadata names unauthenticated. State-root checks still passed with zero RPC. Public metadata came from authenticated local runtime execution, without that fallback.

Python scripts in scratch aggregated the exported JSON. They joined hook spans by parent ID, preserved nested `on_poll` ownership, grouped reads by full raw key, and summed known value lengths. Bytes omit absent or unknown lengths. Backend iterator attempts and storage-root operations are included in operation totals, but are not treated as ordinary reads/writes. Small extracts and summaries are retained under [data/05](data/05/); full traces, binaries, caches, and scripts remain in scratch.

All timing is on an Apple M4 Pro (arm64), with tracing overhead and a warm measured backend. Concurrent replay and other host load can affect these numbers. It is not node DB latency, block-production time, a benchmark weight, or a prediction of savings. Acquisition time is separate. This is a small, mostly idle, testnet sample with three deliberately selected signed Westend blocks and development fixtures. The public and fixture runtimes have different chain state. No code was edited, and nothing was committed or pushed.

## What would make this better in frame-trace

1. Add a block-range command and aggregation across blocks. This report needed scripts for phase shares, per-call medians, hook totals, bytes rankings, and recurring keys.
2. Let `summary` read `block --json` output. Today `summary --from-file` expects `state_traceBlock` JSON and fails on the CLI's own export with `missing field blockHash`. Replaying solely to make a summary repeats acquisition work unless proofs are cached.
3. Add timing, byte-sort, and per-hook tables to `summary`. Current tables show phase counts, items sorted by read/write counts, and phase duplicate keys. They omit the requested time and byte rankings.
4. Export phase timing intervals, signed/inherent classification, root-check status, total proof bytes, and acquisition statistics in block JSON. Root/proof/RPC facts currently need stderr; signed status needs block bodies. Distinguish framework phase work from actual hooks.
5. Expose per-read origin and duplicate scopes. Show overlay/backend/trie origin where the executor can establish it. Separate unchanged repeated reads from reads after writes, and offer per-hook and nested-call duplicate counts.
6. Add per-key marginal proof-node accounting, with a clear shared-node policy. Current proof growth can describe an extrinsic but cannot rank storage items by proof size.
7. Keep backend iteration attempts separate from writes in item tables. In the kill-prefix fixture, `summary --top 100` reports two writes to `System::EventTopics`, while JSON shows one `clear_prefix` and one `iteration`. It reports three writes to the custom `:frame-trace-e2:` prefix, where JSON shows one `clear_prefix` and two iterator attempts. Phase totals correctly keep those operations separate; the byte tables above follow JSON operation kinds.
8. Improve decoded fixed-byte map keys. `Members::ActiveMembers` renders as a truncated integer tuple even though the raw key contains readable ASCII. Show complete decoded bytes or text alongside the raw key. Explain old-fixture metadata fallback directly in fixture commands.
