# Internal notes

The project was renamed from frame-trace to polkadot-frame-trace on 7 November 2026. Dated measurements keep the old name.

The pre-project research, experiments E1 and E2, and the upstream bug drafts were removed on 7 November 2026. They are in commit 2e5c243.

Material for people working on polkadot-frame-trace itself. User documentation is in [`docs/`](../docs/README.md).

| Path | What it is |
| --- | --- |
| [`SPEC.md`](SPEC.md) | The original design: goals, trace model, crate layout and roadmap. |
| [`STATUS.md`](STATUS.md) | Known limits, open decisions and upstream bugs. |
| [`RELEASING.md`](RELEASING.md) | How to cut a release: version bump, changelog, tag, draft release. |
| [`performance.md`](performance.md) | Current numbers: follow throughput, the memory limit, disk cache segments, and how to reproduce them. |

The replay artifact format is specified in [`crates/replay/ARTIFACT.md`](../crates/replay/ARTIFACT.md). Fixture provenance is in [`fixtures/replay/README.md`](../fixtures/replay/README.md).

## Add host functions for a chain

polkadot-frame-trace executes every runtime with one fixed set of host functions: the `HostFunctions`
tuple in [`crates/replay/src/lib.rs`](../crates/replay/src/lib.rs). It matches a stable2609
omni-node: `sp_io::SubstrateHostFunctions`, storage proof size, statement store and the
RFC-163 elliptic curve functions. Executors are built with missing imports disallowed, so a
runtime that imports anything else fails before it runs, with the error
`the runtime needs host functions polkadot-frame-trace does not have: <module:name>, ...`
(see `docs/troubleshooting.md`).

To support such a chain:

1. Find the crate that defines the functions. A `#[runtime_interface]` module exports a
   `HostFunctions` type; the names in the error are `ext_<interface>_<function>_version_<n>`.
2. Add the crate to `crates/replay/Cargo.toml` and update `Cargo.lock` (builds use
   `--locked`). Most SDK crates there are pinned with `=` to one SDK release; pin the new one
   the same way, to the version from that release. Two are not pinned exactly:
   `sp-panic-handler = "13.0.2"` and `sp-crypto-hashing` (workspace `"0.1.0"`) use caret
   ranges. See [`RELEASING.md`](RELEASING.md#polkadot-sdk-upgrades) for SDK upgrades.
3. Append its `HostFunctions` type to the tuple.
4. Replay a real block of that chain with `polkadot-frame-trace block --rpc <url> --block <n>` and
   check `root check: PASS`. Save it with `--save` if it should become a fixture
   (see [`fixtures/replay/README.md`](../fixtures/replay/README.md)).

Every added function is offered to every chain. Only add functions that the node of that
chain also provides, so polkadot-frame-trace never accepts a runtime the chain itself could not run.
The unit tests `unknown_host_import_error_names_the_import` and
`missing_host_function_names_stop_at_the_end_of_the_list`, the integration test
`missing_host_function_is_named_through_runtime_override` and the CLI test
`missing_host_function_fails_with_its_name_and_no_metadata_notice` cover the error path.
`fixtures/wasm/missing-host-function.wasm` reproduces it from the command line.

## Live tests

Tests that need a public node are `#[ignore]`d. Run them with
`cargo +1.93.0 test --workspace --locked -- --ignored`. Each reads its endpoint from an
environment variable and falls back to the public endpoint:

| Variable | Default | Tests |
| --- | --- | --- |
| `FRAME_TRACE_WESTEND_AH_RPC` | `wss://westend-asset-hub-rpc.polkadot.io` | `public_network` (best block), `runtime_upgrade_enactment_and_next_block_live` (the upgrade block #18,985,169 and the first block on the new code; needs archive state for #18,985,169) |
| `FRAME_TRACE_WESTEND_RPC` | `wss://westend-rpc.polkadot.io` | `public_relay_network` (finalized block) |

The `FRAME_TRACE_*` variables used by tests, CI and the build version kept their names after the rename: they are internal.

The nightly workflow (`.github/workflows/nightly.yml`) runs all three with the defaults.
Runtime upgrade blocks and relay chain blocks have no offline fixture, because their runtimes
would add 1.5 to 2.7 MB each to the repository (see
[`fixtures/replay/README.md`](../fixtures/replay/README.md)). These live tests are their only
tests, so run them when a change touches runtime loading, the session caches or block
execution.
