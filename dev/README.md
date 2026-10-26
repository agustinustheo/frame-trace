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

- [HANDOFF.md](HANDOFF.md): a prompt that hands the project to another agent.
