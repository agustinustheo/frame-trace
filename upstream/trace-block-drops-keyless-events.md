# `state_traceBlock` drops storage events without a `key`, even with an empty key filter

DRAFT. Not filed.

## Summary

The doc comment on `BlockExecutor` says an empty `storage_keys` means nothing is filtered out. In the code, an empty key filter still removes storage events that have no `key` field: `ClearPrefix`, `ChildClearPrefix`, `ChildKill`, `StorageRoot`, `ChildStorageRoot`, `IndexTransaction` and `RenewTransactionIndex`. A `System.kill_prefix` deletion is therefore missing from the trace. Related gaps: `next_storage_key` is never traced, parent ids are always null, and spans have no order or timing.

## Versions

- polkadot-omni-node 1.24.2-72284b37a23 (`polkadot-stable2606-2`).
- Line numbers: polkadot-sdk master `14f57c4e28081367fbfb21cf845eb34d043865d5` (2026-10-24).
- Observed on macOS arm64 with `next-people-paseo-runtime` 0.1.0, spec_version 3004000.

## Steps to reproduce

1. Run a node that can replay signed blocks. The official binary cannot (see the `--dev` replay draft). Use the proof-aware build from that work.
2. Submit `sudo(system.killPrefix(prefix, subkeys))` for a prefix that has stored keys. Note the block hash `H`.
3. Call `state_traceBlock(H, "state", "", "")`.
4. Search the result for `"method": "ClearPrefix"`.

## Expected

A `ClearPrefix` event whose `prefix` matches the call.

## Actual

No such event. The all-target trace contains the `clear_prefix_version_2` span but no storage event.

Fixtures under `/Users/theo/Projects/parity/frame-trace/fixtures/next-people-paseo/8/`: `probe-storage.json` is `null`, so the key is deleted. `trace-clear-prefix-method.json` (filter `ClearPrefix`) has zero events. `trace-all-targets.json` has 405 events and two `clear_prefix_version_2` spans, with no `ClearPrefix` event.

## Cause

`substrate/client/tracing/src/block/mod.rs`:

- Lines 331-335 keep an event only if `event_values_filter(e, "key", keys)` is true. That function (lines 359-366) returns false when there is no `key`. So `Some("")` removes keyless events.
- Lines 331-335 use `unwrap_or(false)`, so an omitted filter (`None`) drops every event. The E1 run saw zero events when filters were omitted.

Keyless emitters in `substrate/primitives/state-machine/src/ext.rs`: `kill_child_storage` (`ChildKill`, line 404), `clear_prefix` (`ClearPrefix`, line 423), `ChildClearPrefix` (line 452), `StorageRoot` (line 497), `ChildStorageRoot` (line 524), `IndexTransaction` (line 537) and `RenewTransactionIndex` (line 555).

Other gaps:

- `next_storage_key` (line 273) and `next_child_storage_key` (line 311) have no `trace!` call.
- Parent ids come only from explicit parents (`block/mod.rs:174` and `:206`). They were null in every run, including with a `with-tracing` runtime.
- Spans come from a `HashMap` via `.drain()` (line 323), so array order has no meaning. `enter` and `exit` (lines 217-219) are no-ops.

## Suggested fix

Minimal: keep all events when the key filter is empty.

```rust
.filter(|e| match self.storage_keys.as_deref() {
    // Empty means no key filtering, as the doc comment says.
    Some("") => true,
    Some(keys) => event_values_filter(e, "key", keys),
    None => false, // unchanged; see note
})
```

Optional: for non-empty filters, also match keyless `ClearPrefix` and `ChildClearPrefix` events on their `prefix`. A deletion overlaps a filter when one hex string is a prefix of the other. Treating `None` like `""` changes today's zero-event result, so it needs a maintainer decision. Optional and larger: record an enter sequence number per span, which changes the RPC response type.

## Related

- #7270 (open): `state_traceBlock` error on a Westend Coretime archive node. Its request uses `storageKeys` `""` and lists `ClearPrefix` and `ChildKill` in `methods`, which this code would drop. Not verified as the cause of that report.
- #9871 (merged 2025-10-10): "`trace_block`: Support overwriting `execute_block`". Surfaced by search; not reviewed for filter changes.

Searches: `clear_prefix tracing` returned nothing. `traceBlock`, `state_traceBlock` and `trace_block` returned no issue about keyless events.
