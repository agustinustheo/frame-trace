# Status

Last updated 7 November 2026.

polkadot-frame-trace replays blocks and runtime API calls from safe RPC, with the root check, and exports every storage read and write. The [README](../README.md) lists the features, [support](../docs/reference/support.md) lists the tested chains, and [CHANGELOG.md](../CHANGELOG.md) lists what 0.1.0 adds.

## Limits

- Replay times are measured on the tracing machine, with tracing on. They show where time goes, not node database latency. Benchmarks stay the source of truth for weights.
- `follow` keeps up with Westend Asset Hub on 8 workers in our runs, but a slow node or large blocks may fill its queue. See [performance.md](performance.md#follow-throughput-with-8-workers).
- The host functions are fixed at build time. See [Add host functions for a chain](README.md#add-host-functions-for-a-chain).
- Plain `#[pallet::constant]` values are compiled into the runtime and never touch storage, so they are not traced.

## Decisions for you

1. **Constants (spec phase 4).** A macro prototype on branch `flight/constants-tracing` (commit d21a382) was dropped: it needs runtime source changes and a `with-tracing` build. Inline `ConstU32` values stayed invisible. The other option lists constant values from metadata per pallet, but cannot show which ones a block read. See [SPEC.md](SPEC.md#phases).
2. **PolkaMeter integration (spec phase 5).** Attach a block trace to each load-test step. This changes the PolkaMeter repo.
3. **First release, v0.1.0.** The release workflow is ready. Follow [RELEASING.md](RELEASING.md).

## Upstream

| Bug | Issue | Fix PR | State (6 November 2026) |
| --- | --- | --- | --- |
| `polkadot-omni-node --dev` can't trace blocks with signed extrinsics | [#13594](https://github.com/paritytech/polkadot-sdk/issues/13594) | [#13665](https://github.com/paritytech/polkadot-sdk/pull/13665) | Merged 2 November 2026 |
| `state_traceBlock` drops keyless events with an empty filter | [#13595](https://github.com/paritytech/polkadot-sdk/issues/13595) | [#13666](https://github.com/paritytech/polkadot-sdk/pull/13666) | Merged 2 November 2026 |

polkadot-frame-trace does not depend on either fix. The drafts are in `dev/upstream/` at commit 2e5c243.
