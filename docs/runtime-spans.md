# Runtime spans

frame-trace always records every storage operation. It can only nest those
operations under dispatch and hook spans when the runtime itself emits spans.
A runtime does that only when it is built with the `with-tracing` feature.

## What `with-tracing` adds

Without the feature, `sp-tracing` compiles its span macros to nothing in Wasm.
Storage operations then sit directly under frame-trace's phase spans.

With the feature, the Wasm calls the tracing host functions. frame-trace then sees:

- `execute_block`, `init_block`, `apply_extrinsic` and the final checks from `frame-executive`;
- one span per pallet hook, shown as `on_initialize: Balances` (also `on_finalize`, `on_idle`, `on_poll`);
- one span per dispatched call, for example `transfer_keep_alive` under `batch`.

The feature does not change storage behaviour. The same source built with and
without it writes the same state. It does make the Wasm larger and a little slower.
Do not put a traced runtime on a production chain.

## Cargo wiring

Add one feature to the runtime crate's `Cargo.toml`:

```toml
[features]
with-tracing = ["sp-io/with-tracing", "frame-executive/with-tracing"]
```

Both features turn on `sp-tracing/with-tracing`. Cargo enables a feature once for
the whole Wasm build. So the spans that FRAME generates for hooks and calls turn on
too. If your runtime uses `sp-tracing` directly, you can also add
`"sp-tracing/with-tracing"`. It is not required.

We checked this against the stable2609 crates: `sp-io` 49.0.0,
`frame-executive` 49.0.0 and `sp-tracing` 19.0.0. The first two each declare
`with-tracing = ["sp-tracing/with-tracing"]`. `frame-support` and `sp-runtime`
have no such feature.

The Individuality `next-people-paseo` runtime needs only this line
([patch](../experiments/e1/people-with-tracing.patch)):

```diff
 [features]
+with-tracing = ["sp-io/with-tracing", "frame-executive/with-tracing"]
 default = ["std"]
```

## Build the Wasm

Build the runtime crate with the feature. `substrate-wasm-builder` passes it on to
the Wasm build. For the People runtime:

```sh
cd individuality   # commit e69b079
git apply /path/to/frame-trace/experiments/e1/people-with-tracing.patch
SKIP_PALLET_REVIVE_FIXTURES=1 \
  cargo +1.93.0 build --release --locked -p next-people-paseo-runtime --features with-tracing
cp target/release/wbuild/next-people-paseo-runtime/next_people_paseo_runtime.compact.compressed.wasm \
  people-traced.wasm
```

If you set `CARGO_TARGET_DIR` outside the workspace, also set
`WASM_BUILD_WORKSPACE_HINT` to the workspace root. Without it, the Wasm builder can
resolve a different lock file.

## Using the traced Wasm

The traced Wasm has a different code hash from the chain's `:code`. To the chain,
it is a different runtime. There are two ways to use it.

**A local or dev chain.** Put the traced Wasm in the chain's genesis, for example
with `chain-spec-builder ... --runtime people-traced.wasm`. Every block then has
runtime spans. No special option is needed.

**A local override.** `block`, `summary` and `call` accept `--runtime-override WASM`.
frame-trace still fetches and checks the parent state, including `:code`. Then it
runs the given Wasm instead of `:code`. This skips the code-hash check on purpose.

```sh
target/release/frame-trace block --offline fixtures/replay/people-transfer \
  --runtime-override people-traced.wasm --json transfer.json
# RUNTIME OVERRIDE: people-traced.wasm will execute instead of the chain's authenticated :code
# RUNTIME OVERRIDE: executing 0xba01b1de… INSTEAD OF the authenticated :code 0xee817a9e…; ...
# runtime override: RuntimeVersion matches the chain's :code
# root check: PASS with RUNTIME OVERRIDE; ...
```

This block comes from a chain without tracing. With the override it has 106
runtime spans. Without it, it has none.

Before running, frame-trace reads both runtime versions. It warns if `spec_name`,
`spec_version`, `transaction_version`, `system_version` or the API list differ. It
also says when it could not read a version.

The block's storage-root check still runs. It passes when the override is the same
source built with `with-tracing`. A different runtime normally fails it. Any failure
then says that the override may be the cause. Calls have no root check, so read the
version warnings carefully.

A pass only shows that the substitute wrote the same final state. The spans and
timings describe the substitute. Metadata also comes from the substitute.

The override is labelled everywhere:

- stderr names the file and both code hashes before anything runs;
- JSON has `runtime_override` with `path`, `code_hash`, `chain_code_hash` and
  `version_differences`;
- `summary --from-json` and `report` print the override first, as
  "N of M block(s) used a runtime override", and report JSON lists those blocks;
- the OTLP root span name ends in `[runtime override]`. It has
  `frame.runtime.override = true` and both code hashes.

The file must be a Wasm module, plain or SDK-compressed. Other files are rejected
by name before any replay.

`--runtime-override` cannot be combined with `--save`. A saved artifact is always
replayed with the chain's own code.
