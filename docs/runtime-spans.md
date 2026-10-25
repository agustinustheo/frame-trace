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
with-tracing = ["frame-executive/with-tracing", "sp-io/with-tracing"]
```

Both features forward to `sp-tracing/with-tracing`. Cargo unifies that feature
for every crate in the Wasm build, so the span macros that FRAME generates for hooks
and calls are enabled as well. If your runtime depends on `sp-tracing` directly,
you may add `"sp-tracing/with-tracing"` too; it is not required.

This was checked against the stable2609 crates (`frame-executive` 49.0.0,
`sp-io` 49.0.0, `sp-tracing` 19.0.0). Each of the first two declares
`with-tracing = ["sp-tracing/with-tracing"]`. `frame-support` and `sp-runtime` have no
`with-tracing` feature of their own. The E1 patch for Individuality's
`next-people-paseo` runtime adds exactly this line
([`experiments/e1/people-with-tracing.patch`](../experiments/e1/people-with-tracing.patch)):

```diff
 [features]
+with-tracing = ["sp-io/with-tracing", "frame-executive/with-tracing"]
 default = ["std"]
```

## Build the Wasm

Build the runtime crate with the feature. `substrate-wasm-builder` passes it on to
the Wasm build. This is the E1 command for the People runtime:

```sh
cd individuality   # commit e69b079 in E1
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

The traced Wasm has a different code hash from the chain's `:code`. It is a
different runtime as far as the chain is concerned. There are two ways to use it.

**A local or dev chain.** Start the chain with the traced Wasm in its genesis, as E1 did
with `chain-spec-builder ... --runtime people-traced.wasm`. Every block then carries
runtime spans and replays with no special option. The `people-traced` fixture comes
from such a chain.

**A local override.** `block`, `summary` and `call` accept `--runtime-override WASM`.
frame-trace still fetches and authenticates the parent state, including `:code`.
It then executes the given Wasm instead of `:code`. This bypasses the code-hash check
on purpose, and frame-trace says so:

```sh
target/release/frame-trace block --offline fixtures/replay/people-transfer \
  --runtime-override people-traced.wasm --json transfer.json
# RUNTIME OVERRIDE: executed 0xba01b1de… INSTEAD OF the authenticated :code 0xee817a9e…;
# code-hash check BYPASSED (state proofs and any root check still apply)
# root check: PASS; ...
```

`people-transfer` was recorded on the untraced People chain. With the override it
has 106 runtime spans; without it, none. The block's storage-root check still
runs. When the override is the same source built with `with-tracing`, it passes.
When it is a different runtime, it normally fails with a root mismatch.

A pass only shows that the substitute produced the same final state. The spans and
timings describe the substitute's execution. Metadata also comes from the substitute.

The override is labelled everywhere:

- JSON has `runtime_override` with `path`, `code_hash` and `chain_code_hash`;
- the OTLP root span name ends in `[runtime override]` and has `frame.runtime.override = true`
  plus both code hashes.

`--runtime-override` cannot be combined with `--save`. A saved artifact holds the
chain's state and is replayed with the chain's own code.
