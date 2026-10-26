# Handoff prompt for the next agent

Paste everything below the line into the new agent.

---

You are taking over **frame-trace**, a Rust CLI that replays a Polkadot SDK block (or a runtime API call) on the user's machine and shows every storage read and write as an OpenTelemetry trace. You work for Theo (agustinus@parity.io), a Parity engineer. Read this whole prompt before doing anything.

## Where things are

- Local repo: `~/Projects/parity/frame-trace` (branch `main`, HEAD `c53bec0` or later).
- GitHub: https://github.com/agustinustheo/frame-trace — **public**, Apache-2.0.
- Docs site (mdBook, deployed by `.github/workflows/docs.yml` on every push to `main` that touches docs): https://agustinustheodorus.com/frame-trace/
- Rust toolchain pinned to **1.93.0** (`rust-toolchain.toml`). The machine's default stable is older, so always use `cargo +1.93.0`.
- Push with SSH (`git push`); the `gh` token lacks the `workflow` scope, so pushes that touch `.github/workflows/` must go over SSH, which `origin` already uses.

Read these first, in order:
1. `README.md` — the user-facing pitch and five-minute start.
2. `dev/STATUS.md` — what works, limits, open decisions (partly dated; this prompt supersedes it).
3. `dev/SPEC.md` — design and phases.
4. `docs/STYLE.md` — the writing rules for README and docs. Follow them strictly.
5. `dev/research/` — 01 SDK internals, 02 prior art, 03 Rust OpenTelemetry, 04 individuality setup, 05 first findings from 65 real blocks, 06 docs inspiration (Sentry), 07 docs audit, 08 what Parity uses for observability.
6. `dev/experiments/e1` (why `state_traceBlock` is not enough) and `dev/experiments/e2` (the self-replay prototype that became the product).

## What frame-trace does

- **Self-replay from safe RPC.** It fetches the parent block's state on demand as authenticated trie proofs (`state_getReadProof`, `state_getChildReadProof`), loads the runtime from that verified state, and runs the block's real `Core_execute_block` with the Polkadot SDK executor (`sc-executor` 0.51, `sp-state-machine` 0.54, `sp-io` 49 — SDK stable2609). The storage-root check must pass, so a trace is a faithful copy of what the chain did. Works against any node, including public endpoints, with no unsafe RPC. RPC methods are allowlisted.
- **Capture.** A `tracing` subscriber records every `target: "state"` event (including the keyless ones the node's own `state_traceBlock` drops: ClearPrefix, child kills, storage roots), backend iteration (`next_key`), runtime spans with real parents, and timings. Two passes: fetch/prepare, then a final timed replay with zero RPC calls (a cache miss there is an error).
- **Read origin and proof bytes.** Each read is classified as cache (served from the block's own pending changes), trie (already in proof), or trie (new), and gets the proof bytes it added (marginal growth of the SDK recorder estimate; sums to the block's execution proof). In JSON these are `overlay` / `backend_warm` / `backend_cold`; in terminal and OTLP they are the plain words.
- **Decoding.** Metadata is computed locally by executing `Metadata_metadata_at_version` against the authenticated runtime (cached by code hash + dependency values); RPC metadata is only an explicit, labelled fallback. Keys decode to `System::Account(5Grw…)`, extrinsics to `Balances.transfer_keep_alive`, hooks to `on_initialize: Balances` (instances resolved in declaration order, `on_idle` rotation handled).
- **Phases.** Initialization, each extrinsic, `post inherents (on_poll)` (nested inside the first non-inherent extrinsic, or top-level when the block has only inherents), finalization. Rules and the exact `System::ExecutionPhase` / `:extrinsic_index` / `ExtrinsicData` write sequence are documented in `crates/model/src/phase.rs`.
- **Runtime override.** `--runtime-override <wasm>` replays a real block with the same runtime source built `with-tracing`, to get dispatch/hook spans; the root check still passes; the override is labelled everywhere (terminal, JSON, OTLP, report) and cannot be saved as an artifact; runtime versions are compared and warned.
- **Commands:** `block`, `call` (runtime API with optional `--check-state-call` against the node's `state_call`), `summary`, `report` (many blocks, streaming, bounded memory), `follow` (finalized heads by default, `--best` optional). Outputs: OTLP/HTTP (`--otlp`, default `http://127.0.0.1:4318`, headers via `OTEL_EXPORTER_OTLP_HEADERS`), JSON (`schema_version` 2, documented), terminal tables, saved offline artifacts (`--save` / `--offline`).

## Code layout

```
crates/model   trace model: op parsing, phases, stats, replay capture conversion
crates/decode  metadata-based key, extrinsic and hook decoding
crates/replay  self-replay: safe RPC client, proof DB and caches, executor, capture, artifacts, calls
crates/otel    OTLP export (opentelemetry 0.33 used directly, own HTTP exporter that detects partial rejection)
crates/cli     the frame-trace binary (block, call, summary, report, follow)
fixtures/      offline test data: replay artifacts (people-*, westend-asset-hub, call-*), shared blobs, goldens, legacy JSON
examples/      grafana-lgtm (recommended; grafana/otel-lgtm:0.11.16 like Parity's trinity-apps, with a provisioned dashboard), alloy-tempo (Alloy → Tempo → Grafana), jaeger
docs/          the mdBook site (getting started, guides, concepts, reference, troubleshooting, glossary)
dev/           internal: spec, status, research, experiments, upstream issue drafts, performance log, this handoff
```

Checks before any commit (all must pass): `cargo +1.93.0 fmt --check`, `cargo +1.93.0 clippy --workspace --all-targets --locked -- -D warnings`, `cargo +1.93.0 test --workspace --locked` (177 passed, 1 ignored network test at handoff), and `mdbook build` with no warnings if docs changed (mdbook 0.4.52, mdbook-mermaid 0.16.0; mermaid assets are committed). If you add a dependency, update `Cargo.lock` so `--locked` builds still work — a previous agent forgot.

## How Theo wants work done

- **Subagent tiers (strict):** Haiku 5.5 High for reading, research and summarising; GPT 6.1 Sol High (via T3 `delegate_task`, provider `codex`, model `gpt-6.1-sol`, `reasoningEffort: high`) or Claude Opus 5.5 Medium for writing code; reviews by BOTH Sol High and Haiku High, independently; Sol High for running tests and experiments; Opus 5.5 Medium for verifying. Never let a writer review or verify its own work. Never use Sonnet or Fable subagents. Say which tier each agent is.
- **Loop that worked:** writer → two independent reviews → one combined fix round → independent verifier (in a separate `git worktree` at the commit, own `CARGO_TARGET_DIR`) → commit. Check test counts yourself; agents have misreported them.
- **Commits:** never add Claude co-author or attribution lines to commits or PRs. Plain, descriptive commit messages.
- **Reuse, don't reinvent.** Prefer Polkadot SDK and ecosystem crates; do market research before building something new.
- **Docs:** follow `docs/STYLE.md` (Sentry-style: result first, real output after every command, one term per idea, no internal jargon, annotated screenshots, guides named after jobs). Theo called the first docs "horrible"; the current ones were rebuilt and verified command by command. Keep that bar.
- **Outward-facing actions need Theo's approval:** filing issues or PRs on other repos (especially `paritytech/*`), changing repo settings, publishing anything new. Never merge or undraft `paritytech/polkadot-pop-e2e` PRs.
- **Disk:** review agents each create multi-GB `target` dirs under `/tmp/pm-audit`; delete them when done.
- Theo uses voice-to-text; read requests charitably ("tri-state" meant `try_state`, "Polka meter" is PolkaMeter).

## Open items (none started)

1. **Upstream bug reports** (drafts in `dev/upstream/`, not filed — ask Theo first):
   - `omni-node-dev-replay.md`: polkadot-omni-node `--dev` cannot replay signed parachain blocks (storage-root mismatch) because the dev path passes no proof-aware executor. A 5-line patch is in `dev/experiments/e1/omni-dev-proof-replay.patch`.
   - `trace-block-drops-keyless-events.md`: `state_traceBlock` drops storage events without a `key` (ClearPrefix etc.), never traces `next_key`, and has null parent ids and no timing.
2. **Constants (spec phase 4).** Plain `#[pallet::constant]` values are compiled in and never touch storage. Tracing them needs an opt-in macro or an instrumented runtime. Needs a design discussion with Theo.
3. **PolkaMeter integration (spec phase 5).** PolkaMeter (`~/Projects/parity/polkameter`, github agustinustheo/polkameter) is Theo's load tester for Polkadot SDK chains. Idea: attach a block trace to each load-test step and link spans by extrinsic hash. Changes the PolkaMeter repo; coordinate with whoever works there.
4. **Performance.** `follow` replays about 6–9 blocks/minute on a public Westend Asset Hub endpoint, slower than the chain, so it samples near the tip. Most time is sequential proof round trips during the fetch pass; see `dev/performance.md`.
5. **Host functions** are fixed at build time; chains needing extra ones (EVM, BEEFY/MMR) fail with a clear "unsupported host function" error. Supporting them is per-chain work.
6. **Known small gaps:** the public-network test is `#[ignore]`; child-trie proofs are tested over a mock, not a real child-storage chain; runtime-upgrade blocks are untested; Tempo-in-cluster docs assume Alloy/Collector OTLP endpoints without client certificates.

## First steps for you

1. `cd ~/Projects/parity/frame-trace && git pull && cargo +1.93.0 test --workspace --locked` and confirm 177 passed.
2. Try the five-minute start in the README (Jaeger), then `examples/grafana-lgtm`, to see what users see.
3. Ask Theo which open item to take, unless he has already told you.
