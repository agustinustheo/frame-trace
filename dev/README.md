# Internal notes

Material for people working on frame-trace itself. User documentation is in [`docs/`](../docs/README.md).

| Path | What it is |
| --- | --- |
| [`SPEC.md`](SPEC.md) | The original design: goals, trace model, crate layout and roadmap. |
| [`STATUS.md`](STATUS.md) | Project status, how it was built, known limits and open decisions. |
| [`performance.md`](performance.md) | Measurement log across the fix rounds: replay times, follow throughput and test counts. |
| [`research/`](research/) | Background research: SDK internals, prior art, Rust OpenTelemetry, the Individuality setup, first findings from 65 blocks, docs inspiration and the docs audit. |
| [`experiments/e1/`](experiments/e1/README.md) | Experiment E1: `state_traceBlock` on a local People chain, with scripts and patches. `people-with-tracing.patch` adds the `with-tracing` feature to that runtime. |
| [`experiments/e2/`](experiments/e2/README.md) | Experiment E2: self-replay from safe RPC, which became the main design. Its results back the fixtures in `fixtures/replay/`. |
| [`upstream/`](upstream/) | Draft bug reports for polkadot-sdk. Not filed. |

The replay artifact format is specified in [`crates/replay/ARTIFACT.md`](../crates/replay/ARTIFACT.md). Fixture provenance is in [`fixtures/replay/README.md`](../fixtures/replay/README.md).

## Add host functions for a chain

frame-trace executes every runtime with one fixed set of host functions: the `HostFunctions`
tuple in [`crates/replay/src/lib.rs`](../crates/replay/src/lib.rs). It matches a stable2609
omni-node: `sp_io::SubstrateHostFunctions`, storage proof size, statement store and the
RFC-163 elliptic curve functions. Executors are built with missing imports disallowed, so a
runtime that imports anything else fails before it runs, with the error
`the runtime needs host functions frame-trace does not have: <module:name>, ...`
(see `docs/troubleshooting.md`).

To support such a chain:

1. Find the crate that defines the functions. A `#[runtime_interface]` module exports a
   `HostFunctions` type; the names in the error are `ext_<interface>_<function>_version_<n>`.
2. Add the crate to `crates/replay/Cargo.toml`, pinned with `=` to the version that matches
   the SDK release of the other `sp-*`/`sc-*` pins there, and update `Cargo.lock` (builds
   use `--locked`).
3. Append its `HostFunctions` type to the tuple.
4. Replay a real block of that chain with `frame-trace block --rpc <url> --block <n>` and
   check `root check: PASS`. Save it with `--save` if it should become a fixture
   (see [`fixtures/replay/README.md`](../fixtures/replay/README.md)).

Every added function is offered to every chain. Only add functions that the node of that
chain also provides, so frame-trace never accepts a runtime the chain itself could not run.
The unit test `unknown_host_import_error_names_the_import` and the integration test
`missing_host_function_is_named_through_runtime_override` cover the error path.
