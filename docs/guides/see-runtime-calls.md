# See runtime calls inside extrinsics

frame-trace always records every storage operation.
It can only show the calls around them, such as `batch` and `transfer_keep_alive`, when the runtime reports them.
A runtime does that only when it is built with the `with-tracing` feature.

| Runtime | What you see under a phase |
| --- | --- |
| Normal build | Storage operations only, directly under the phase. |
| Built with `with-tracing` | `apply_extrinsic`, the dispatched calls such as `batch` and `transfer_keep_alive`, one span per hook such as `on_initialize: Balances`, and the storage operations inside each. |

[Reading a trace](../getting-started/reading-a-trace.md) shows a traced runtime.

## 1. Add the feature to your runtime

In the runtime crate's `Cargo.toml`:

```toml
[features]
with-tracing = ["sp-io/with-tracing", "frame-executive/with-tracing"]
```

Both features turn on `sp-tracing/with-tracing` for the whole Wasm build.
That also turns on the spans FRAME generates for hooks and calls.
This was checked against the stable2609 crates: `sp-io` 49.0.0, `frame-executive` 49.0.0 and `sp-tracing` 19.0.0.

The feature does not change what the runtime writes.
It makes the Wasm larger and a little slower.

> **Warning:** Do not deploy a traced runtime to a production chain. Use it only for local replays.

## 2. Build the Wasm

Build the runtime crate with the feature:

```sh
cargo build --release -p <runtime-crate> --features with-tracing
```

The Wasm is in `target/release/wbuild/<runtime-crate>/<runtime_crate>.compact.compressed.wasm`.
If you set `CARGO_TARGET_DIR` outside the workspace, also set `WASM_BUILD_WORKSPACE_HINT` to the workspace root.
Otherwise the Wasm builder can pick up a different lock file.

Build it from the same source and version as the runtime on chain.

## 3. Replay a real block with it

`--runtime-override <wasm>` runs your Wasm instead of the chain's runtime.
It works with `block`, `summary` and `call`.
frame-trace still fetches and checks all state, including the chain's runtime code, then runs your file instead.

For your own chain:

```sh
frame-trace block --rpc <url> --block finalized \
  --runtime-override <path-to-your-traced-runtime.wasm> --otlp
```

### Try it with the shipped People runtime

The repository ships a block from a People chain whose runtime has no tracing, `fixtures/replay/people-transfer`.
It also ships the same runtime built with `with-tracing`, inside the saved block `fixtures/replay/people-traced`.
It is the largest proof node in that file, so you can extract it with `zstd`, `jq` and `xxd`.
Run this from the `frame-trace` directory:

```sh
zstd -dc fixtures/replay/people-traced/artifact.json.zst \
  | jq -r '.proof_nodes | max_by(length) | ltrimstr("0x")' \
  | xxd -r -p > people-traced.wasm
```

That runtime was built from the Individuality `next-people-paseo` runtime with one extra feature line, [`dev/experiments/e1/people-with-tracing.patch`](https://github.com/agustinustheo/frame-trace/blob/main/dev/experiments/e1/people-with-tracing.patch).

Now replay the untraced block with it:

```sh
frame-trace block --offline fixtures/replay/people-transfer \
  --runtime-override people-traced.wasm --otlp
```

```text
RUNTIME OVERRIDE: people-traced.wasm will execute instead of the chain's authenticated :code
RUNTIME OVERRIDE: executing 0xba01b1de0c55e48c84c70a59381594c8540eb2482ac8e785908de120ce617074 INSTEAD OF the authenticated :code 0xee817a9ecd07f4c0f861dd6f2b5105fd316638b827d4d425ae062add4b651c1c; code-hash check BYPASSED (state proofs and any root check still apply)
runtime override: RuntimeVersion matches the chain's :code
replay #2: loading runtime and preparing state
root check: PASS with RUNTIME OVERRIDE; RPC calls: 0 (timed replay: 0); execution proof: 6479 bytes
fetching state #2: 0 proof request rounds; 0 RPC calls
state root: computed 0xa33960d3b1aadccf9197e0a77aebfea4075457c02800658639eb20fd5c6012c9; header 0xa33960d3b1aadccf9197e0a77aebfea4075457c02800658639eb20fd5c6012c9
exported 533 spans to http://127.0.0.1:4318
```

Without the override this block exports 428 spans.
With it, 533: the extra 105 are runtime calls and hooks.

## 4. Check the result

- `runtime override: RuntimeVersion matches the chain's :code` means the versions agree.
  If `spec_name`, `spec_version`, `transaction_version`, `system_version` or the API list differ, frame-trace prints a warning for each.
- `root check: PASS with RUNTIME OVERRIDE` means your Wasm wrote exactly the same state as the chain's runtime.
  That is expected when it is the same source with tracing.
  A different runtime normally fails the [root check](../concepts/what-a-replay-proves.md), and the error says the override may be the cause.
- Runtime API calls have no root check, so read the version warnings carefully.

A pass only shows that your Wasm wrote the same final state.
The spans, timings and metadata come from your Wasm.

## How the override is labelled

Every output says a substitute ran:

- stderr names the file and both code hashes before anything runs;
- the Jaeger root span name ends in `[runtime override]` and has `frame.runtime.override = true`;
- JSON has a `runtime_override` object, see [JSON output](../reference/json.md);
- `summary --from-json` and `report` start with `RUNTIME OVERRIDE: 1 of 1 block(s) used a runtime override`.

## Limits

- The file must be a Wasm module, plain or SDK-compressed. Other files are rejected before any replay.
- `--runtime-override` cannot be combined with `--save`. A saved block always replays with the chain's own runtime.

## On a local chain

On a dev or local chain you control, you can put the traced Wasm in the genesis instead.
Then every block has runtime spans and you need no override.
