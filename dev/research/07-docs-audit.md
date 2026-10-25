# 07 - Docs audit for the frame-trace rewrite

Read-only audit. Checked against the release CLI built from this tree (`cargo +1.93.0 build --release`, `CARGO_TARGET_DIR=/tmp/pm-audit/ft-docs-audit`), each subcommand's `--help`, the CLI source in `crates/cli/src`, and offline runs on the fixtures. No network calls were made. Nothing committed.

## 1. Inventory

Word counts from `wc -w`. "User" means it is written for someone running the tool. "Internal" means it records how the project was built or measured.

| File | Words | Purpose | Audience | Kind |
| --- | ---: | --- | --- | --- |
| `README.md` | 2,769 | Install, quick start, every command, read origins, reports, call, follow, cache, metadata | Tool users, but written as a changelog of the implementation | User (overloaded) |
| `STATUS.md` | 616 | Project status, how it was built, limits, "decisions for you" | Project owner | Internal |
| `SPEC.md` | 1,069 | Goals, reuse table, trace model, roadmap phases | Contributors | Internal (design) |
| `docs/json-schema.md` | 585 | JSON schema v2 for `block --json`, `call --json`, `report --json` | Tool integrators | Reference (user-facing) |
| `docs/runtime-spans.md` | 711 | Build a runtime with `with-tracing`, use `--runtime-override` | Runtime developers | How-to (user-facing), but includes experiment paths |
| `docs/images/jaeger-batch.png` | n/a | Jaeger screenshot of a `Utility.batch` trace (hero image, used by README) | Users | Asset |
| `fixtures/README.md` | 114 | Table of fixture folders; E1 archive unpack | Contributors | Internal |
| `fixtures/model/README.md` | 198 | Handcrafted `state_traceBlock` fixtures; legacy-JSON provenance | Contributors | Internal |
| `fixtures/model/real/README.md` | 135 | Provenance of two real People blocks | Contributors | Internal |
| `fixtures/metadata/README.md` | 169 | Where metadata `.scale` files came from | Contributors | Internal |
| `fixtures/replay/README.md` | 893 | Offline replay fixtures, sizes, "Round 3/Round 4" changelog | Contributors | Internal |
| `crates/replay/ARTIFACT.md` | 1,076 | Artifact v1 on-disk format and internal acquisition rules | Integrators and contributors | Reference (format spec), partly internal |
| `crates/replay/PERFORMANCE.md` | 3,531 | Benchmark logs across "Rounds 1 to 5", with test counts | Contributors | Internal (measurement log) |
| `experiments/e1/README.md` | 3,167 | E1 experiment: `state_traceBlock` on a local People chain | Contributors | Internal |
| `experiments/e2/README.md` | 2,996 | E2 experiment: self-replay from safe RPC, prior-art review | Contributors | Internal |
| `upstream/omni-node-dev-replay.md` | 574 | Draft bug report for polkadot-sdk (not filed) | Maintainers | Internal |
| `upstream/trace-block-drops-keyless-events.md` | 536 | Draft bug report for polkadot-sdk (not filed) | Maintainers | Internal |
| `research/01`-`06` | 5,113 / 2,800 / 3,021 / 1,189 / 2,958 / 2,355 | Research notes, first findings, docs inspiration (06, untracked) | Contributors | Internal |

Totals: the user-facing set (README, docs/*.md, ARTIFACT.md) is about 5,140 words (2,769 README, 1,296 in docs/, 1,076 ARTIFACT). The internal set is about 31,000 words, mostly research and experiments.

## 2. Problems, with quotes

### 2a. README.md (overall)

- **No quick result.** The first 181 lines (about 1,505 words, title through the block section) contain no `##` heading. The first command and its output are at the top, but the reader gets a wall before the "what it looks like" part. Two paragraphs in that span are over 190 words: "Names require metadata V14 or newer. Metadata for calls **and block names** come…" (221 words), and "An eight-block queue bounds pending work. Catch-up/reorg walks never precede…" (195 words).
- **Wall-of-text paragraphs (over 120 words each):** the `follow` queue paragraph; "Short terminal tables and complete JSON show phase shares…" (200 words); "Each call exports one trace rooted at `call <method> at #N`…" (174 words); "`proof_bytes_added` is positive growth of the SDK recorder's encoded-size estimate…" (168 words); "Each replay prints the computed and header state roots…" (158 words); "Hook spans are named `on_initialize: Balances`…" (152 words).
- **Missing "why should I care".** The README never says what a reader can decide from a trace. It starts with "Replay a Polkadot SDK block and view its storage accesses in Jaeger or Tempo" but gives no example of a finding (for example the STATUS.md fact that Alice's account was read 48 times in a 3-transfer batch, 42 of them from the overlay).
- **Commands without output.** All 7 `sh` blocks in the README show commands only. The Jaeger, summary, report and call examples have no sample output. The only sample output in user docs is in `docs/runtime-spans.md` (the `RUNTIME OVERRIDE` lines).
- **Jargon with no definition, in the first 40 percent of the README:** "warm cache and zero RPC", "discovery batches up to 32 predicted storage keys", "A strict authenticated discovery run follows; speculative outputs are never exported or accepted", "trie miss paths", "proof-concurrency", "Catch-up/reorg walks", "Ctrl-C jobs are cancelled, without failure wording", "`--rpc-metadata-fallback` spelling remains an alias", "the SDK recorder's encoded-size estimate".
- **Internal names in user text:** "E2's" and "E1's" appear in README ("Offline tests execute both E1's stable2606-2 People runtimes…"), in fixtures, and in `docs/runtime-spans.md` ("experiments/e1/people-with-tracing.patch", "`cd individuality # commit e69b079`"). "ProofDb" appears only in `experiments/e2/README.md` (line 14), not in the README, but is the kind of name that must not leak into user docs.
- **Stale or inconsistent wording:** `README.md` uses "warm" and "cold" in the "Read origins" section, while the CLI output says `backend_warm`/`backend_cold` in JSON and `warm`/`cold` in the terminal. The README also says "discovery run" and "measured run" without saying what they are.
- **Duplicated content:** the cache description (`--cache-dir`, `$XDG_CACHE_HOME`, eviction, `--cache-max-bytes`) appears in the `follow` paragraph and again in the "Reports across blocks" section. The "measured run / warm cache / zero RPC" idea is repeated in four places. The offline-replay and artifact limits (64 MiB, 100,000 nodes, 24 MiB) are in README and in `crates/replay/ARTIFACT.md`.
- **Mixed registers:** the same page carries a user quick start and a changelog ("Two workers share authenticated content-addressed nodes (bounded to 256 MiB)…", "The old `--rpc-metadata-fallback` spelling remains an alias", "The `workspace's [profile.dev] debug = 0` omits debug symbols…").

### 2b. Stale or wrong against the CLI

- **`SPEC.md` refers to a flag that does not exist.** The trace-model section says "The `trace-block` source has no timing…" and the phases section says the `state_traceBlock` path is kept "as a secondary source (`--source trace-block`)". `--source` is not in any `--help`. The secondary source is selected by `--from-file` (`block --from-file`, `summary --from-file`).
- **`SPEC.md` CLI block is out of date.** It lists `block`, `call`, `follow`, `summary`, `report` only with the basic flags. It omits `--runtime-override`, `--metadata`, `--cache-dir`, `--cache-max-bytes`, `--unauthenticated-metadata`, `--proof-concurrency`, `--sort-by`, `--last`, `--concurrency`, `--best`.
- **`STATUS.md` test count does not match the latest measurement.** `STATUS.md` line 34: "5 crates, 165 tests". The latest measurement in `crates/replay/PERFORMANCE.md` (Round 5) says "154 passed, 0 failed, 1 ignored". I did not run the test suite to resolve this.
- **`docs/json-schema.md` and `fixtures/model/README.md` name different commits.** The fixture file is `fixtures/model/pre-e86f740.json`, but the README says it was "generated with the unmodified 5a1963e CLI" and `docs/json-schema.md` says "generated by the actual 5a1963e CLI". One of the two names is wrong.
- **`fixtures/README.md` table is incomplete.** It lists `model/`, `metadata/`, `e1-probes/`, `e1-chains.tar.zst`. It does not list `fixtures/replay/` (the offline replay fixtures the README uses), nor `next-people-paseo*/`, which exist on disk.
- **`fixtures/replay/README.md` is a changelog.** It contains "Round 3 added `call-westend-dryrun`…", "Round 4 extends `people-traced`…", and "These fixtures were acquired on 2026-10-25."
- **CLI usage errors contradict the docs.** `frame-trace block` with no args exits 2 and says `required arguments were not provided: --from-file <TRACE.json>`. `frame-trace summary` does the same. The user docs and `--help` both say `--rpc` or `--offline` is the normal path. The error text suggests the recorded-file path is the only option. Verified: exit 2 for both.
- **Flag not shown in help.** `--rpc-metadata-fallback` is an alias (source: `crates/cli/src/main.rs:202`, `call.rs:57`, `source.rs:54`) and is documented only in README and `fixtures/replay/README.md`. It is not in any `--help`. That is fine for an alias, but the README should use the new spelling only.
- **Option ranges are in the source, not the help.** `--proof-concurrency` is enforced `1..=64` (`main.rs:193`, `call.rs:48`, `source.rs:45`, `report.rs:35`), and `report --concurrency` is `1..=4` (`report.rs:32`). Only `report` shows the range in `--help`. The README says "`--proof-concurrency 1..64`" for block/follow and "configurable 1–4" for report, so the docs are right but the help is incomplete.
- **Offline `call` prints a confusing notice.** Running `call --offline fixtures/replay/call-people-nonce --summary` prints: `notice: local metadata unavailable (invalid replay input: proof node 0x578f… missing/invalid at path 0x0d71…); using unnamed storage and hex results`. The README says this V14 fixture "intentionally exercises hex fallback", so the notice is expected, but "invalid replay input" makes it sound like a broken artifact.
- **`block --help` lists `--from-json`** as "Saved block --json output; summary reads this without replay". On `block`, `--from-json` works (I ran it), but its help text describes a `summary` behavior. Copy issue.
- **Terminal output uses two phase vocabularies in one table set.** Phase table: `extrinsic 2: Utility.batch` and `└ post inherents (on_poll)`. Phase-shares table in the same output: `inherents`, `signed_extrinsics`, `post_inherents`. The same phase has two names.
- **Terminal output abbreviations.** Headers `R overlay/warm/cold/?`, `W`, `iter`, `read B`, `written B`, `dup`, `proof B`, `storage µs`, `wall µs`, and a summary line `reads 312 = overlay 231 + backend 81 (warm 47, cold 34) + unknown 0`. No legend anywhere in user docs.

### 2c. Concept and terminology problems across docs

- **"E1", "E2", "experiment", "oracle"** are used as if the reader knows them. "oracle" appears 7 times in README (for example "The optional safe `state_call` oracle compares bytes at that exact hash"). Replace with "compares with the node's own `state_call`".
- **"warm/cold"** is used as a classification without definition in README and the output. The README says "`backend_warm` means a backend call used only trie nodes already recorded during this execution" in an 80-word sentence. The reader must hold four ideas at once.
- **"measured" vs "synthetic"** appear in README (8 times for "measured"), SPEC (3 times), and `docs/json-schema.md`. Terminal output says `frame.timing = measured` in OTLP attributes. No user-level definition.
- **"legacy"** is used 5 times in README and 2 times in `docs/json-schema.md` for "saved output that predates schema v2", not explained where the reader meets it.
- **"schema_version", "format_version"** and "artifact v1" are three version numbers with no mapping for the reader.
- **Two terms for one thing:** the README calls the same thing "block" "replay" "trace" and "run" depending on paragraph. "block trace", "trace", "replay" and "replay artifact" are used interchangeably.
- **"Discovery"** (README, ARTIFACT.md) means "first pass that fetches proofs". The README uses it as if it were a feature name. The word also appears in "A strict authenticated discovery run follows".
- **"Overlay"** (README 6 times, `docs/json-schema.md`) is never defined for a reader who has not read the SDK.
- **"Phase"** is used in the README and output without a definition of what counts as one (initialization, extrinsic N, post-inherents, finalization).

### 2d. Format and structure

- `README.md` has only three `##` headings after the title. About 1,500 words sit under no heading.
- README does not use tables for flags or outputs. The flags and their defaults (`--top 15`, `--cache-max-bytes 1 GiB`, `--proof-concurrency 8`, `--concurrency 2`, `--sort-by proof`) are scattered through paragraphs.
- No "what you see" section for Jaeger (the screenshot is the only visual, with no caption explaining the span names).
- Links to `SPEC.md`, `STATUS.md`, `crates/replay/PERFORMANCE.md`, `experiments/` and `fixtures/replay/README.md` appear in the body of the user README, mixed with usage.

## 3. Feature checklist (user-usable)

Commands:

1. `block --rpc URL --block best|NUMBER|HASH` — trace one block from a public RPC; works with any node's safe RPC, no unsafe methods.
2. `block --otlp [URL]` — export the trace to Jaeger or Tempo over OTLP/HTTP; bare `--otlp` defaults to `http://127.0.0.1:4318`.
3. `block --json FILE|-` — write the full decoded trace as JSON (schema v2); `-` writes to stdout.
4. `block --save DIR` / `block --offline DIR` — save a replay as a self-contained artifact and re-run it with no network.
5. `block --from-file TRACE.json` — read a recorded `state_traceBlock` response (secondary source, synthetic timestamps, no proof sizes).
6. `summary --rpc … --block …` / `summary --offline DIR` / `summary --from-json FILE` / `summary --from-file FILE` — terminal tables: storage per phase, top items by proof bytes, duplicate reads, per-extrinsic totals.
7. `summary --top N` — limit table rows (default 15).
8. `call --rpc URL --at best|finalized|NUMBER|HASH --method NAME --data HEX` or `--args JSON` — run a runtime API call against authenticated state; no block root check applies.
9. `call --oracle` — compare the local result with the node's `state_call` at the same block hash; exit code 3 on mismatch.
10. `call --summary` / `call --json` / `call --otlp` / `call --save DIR` / `call --offline DIR` — tables, JSON, OTLP, artifact, offline re-run.
11. `follow --rpc URL [--best] [--otlp]` — replay each new finalized block (or best head with `--best`), with reorg reporting and a queue status line (queued, in-flight, done, retryable, terminal, reverted, skipped, cancelled).
12. `report FILE… | --from-json FILE | --rpc URL --from N --to M | --last K` — aggregate many blocks; output per item, extrinsic, hook and phase; `--sort-by proof|ops|time`; `--json FILE|-`.
13. `report --concurrency 1..4` / `--proof-concurrency 1..64` — tune block and proof parallelism.

Key flags and options:

14. `--runtime-override WASM` — run a traced build of the runtime instead of the chain's `:code`; output is labelled everywhere; the state root check still applies.
15. `--unauthenticated-metadata` (alias `--rpc-metadata-fallback`) — allow unverified RPC metadata when local execution cannot produce it; output is labelled.
16. `--metadata FILE` — supply metadata for a recorded trace (`.scale` or hex).
17. `--cache-dir DIR` and `--cache-max-bytes N` — persist proof nodes and compiled Wasm; default cap 1 GiB; active runtimes stay pinned.
18. `--proof-concurrency N` — parallel proof requests (1 to 64, default 8, batches of 32 keys each).

Outputs:

19. OTLP spans in Jaeger or Tempo: one trace per block (or per call), phases as children, extrinsics and hooks as spans, storage operations as the leaves, with `db.collection.name` (pallet), `frame.timing`, `frame.phase.timing_available`, proof bytes.
20. Terminal summary with a root-check line: `root check: PASS; RPC calls: 0 (measured: 0); execution proof: 6586 bytes`, plus the computed and header state roots.
21. Terminal tables: storage per phase; phase shares; top items by proof bytes with read origin breakdown; top extrinsics; top hooks; duplicate reads.
22. JSON schema v2 (`docs/json-schema.md`): block, call, report JSON, with `execution`, `totals`, `phases`, `wall_time`, `origin`, `proof_bytes_added`, `runtime_override` fields.
23. Report aggregates: distributions (median, p95) of operations, proof bytes and time per extrinsic type, and per-block error list; exit code 1 if any input failed.
24. Offline artifacts: `--save` writes a self-contained directory; `--offline` replays it with no RPC client and no network.
25. Read origin per storage read: `overlay` (cache within the block), `backend_warm` (trie nodes already seen), `backend_cold` (new trie nodes).
26. Proof bytes per storage operation: marginal growth of the proof, summing to the execution proof (except after rollback, when over-attribution is reported).
27. Measured timings on the replay machine (not node latency); recorded `state_traceBlock` input has synthetic timing.
28. Runtime spans when the Wasm is built with `with-tracing`; without it, operations sit under phase spans.
29. Exit codes: 0 success, 1 execution/input/export error, 2 usage error, 3 oracle mismatch.

## 4. Concepts a reader must understand to read the output

Each concept, explained in one or two plain sentences.

- **Phase.** The part of a block the storage happened in: initialization (runs before the extrinsics), each extrinsic (one transaction, named like `Balances.transfer_keep_alive`), post-inherents (hooks that run after the mandatory inherents), and finalization (cleanup at the end of the block).
- **Overlay read.** A read answered from state that this block has already written or read during the same execution. It never touched the stored trie, so it is cheap and does not add proof bytes.
- **Warm read.** A read that went to the stored trie, but only through trie nodes already fetched earlier in the same execution. It adds little or no proof.
- **Cold read.** A read that went to the stored trie and needed trie nodes the execution had not seen yet. It adds proof bytes. "Cold" does not mean disk I/O.
- **Proof bytes.** The size of the trie proof data the execution needed. Each storage operation is charged the bytes it added, so the sum matches the block's total execution proof.
- **Duplicate read.** A second read of the same storage key within the same phase, including reads after writes. It is a sign of repeated work, not an error.
- **Measured vs synthetic timing.** Measured means the timer ran during a replay on your machine, including tracing overhead, so it shows where time goes, not node database latency. Synthetic means the timestamps were spaced evenly because the recorded input had no timing; ignore them for timing.
- **Root check.** After executing a block, the runtime checks that the state root it computed equals the root in the block header. A pass means the replay produced the same final state as the chain. A call has no root check, because a call does not produce a new block.
- **Runtime override.** Running a substitute Wasm (for example a build with `with-tracing`) instead of the chain's own runtime code. The root check still applies, so a pass shows that the substitute produced the same final state. Output is labelled as overridden, because spans and timings describe the substitute.
- **Metadata fallback.** If the replay cannot execute the runtime's metadata function locally, names fall back to unnamed storage keys and hex values. `--unauthenticated-metadata` allows a fallback from RPC metadata, which is not verified; output is labelled.
- **Authenticated proof.** Each trie node is checked against the block's state root as it is fetched. Without this, RPC data would be trusted.
- **Root mismatch (failure).** If the replay's computed state root differs from the header's root, the replay fails. It means the replay did not reproduce the chain (or a substitute runtime differs from the chain's); the error text says which root differed.
- **Oracle match (call).** Optional comparison of the local call result with the node's own `state_call` at the same block hash. Mismatch exits 3.
- **Recorded (unverified) oracle (offline call).** When re-running an artifact offline, the saved oracle bytes are compared, but nothing proves they came from the node.
- **Reorg.** In `follow --best`, a block that was on the best chain is replaced by a different block at the same height; the replay reports the reverted block.

## 5. Internal material to move out of the user docs path

Move out of the user path (keep in repo, under `research/`, `experiments/`, or `internal/`):

- `crates/replay/PERFORMANCE.md` (whole file; benchmark rounds, /tmp paths, test counts).
- `experiments/e1/`, `experiments/e2/` (READMEs, patches, scripts, `results/`).
- `upstream/` (draft bug reports, not filed).
- `research/01`–`06` (background research, prior art, first findings, docs inspiration).
- `STATUS.md` (project status and owner decisions; move the "What works" paragraph into the README intro).
- `SPEC.md` (design doc; keep a short "Why not state_traceBlock" paragraph in an explanation page, drop the phase plan and E1/E2 references).
- `fixtures/README.md`, `fixtures/model/README.md`, `fixtures/model/real/README.md`, `fixtures/metadata/README.md`, `fixtures/replay/README.md` (all describe fixture provenance and rounds; keep them next to the fixtures, not in the docs site).
- `crates/replay/ARTIFACT.md` — split: the public part (what an artifact contains, `--save`/`--offline`, size limits) goes to reference; the acquisition-internals part (follow prediction, concat-map inference, 4,096 key caps, proof round accounting) goes to internal notes.
- The E1/E2 and "individuality commit e69b079" steps in `docs/runtime-spans.md` (keep the Cargo feature line and the override command; move the People build steps to an internal note).

Keep in the user docs path: README (rewritten), `docs/json-schema.md` (reference), `docs/runtime-spans.md` (how-to, trimmed), and the new pages from the rewrite (quick start, reading a trace, concepts, guides, reference).

## Notes

- `research/06-docs-inspiration.md` is untracked (not part of this audit's output; left as-is).
- The user-facing docs have no "what you will see" example for `summary` or `report`, which the audit's checklist suggests adding.
- Claims about test counts and performance numbers were not re-measured; they are quoted from the source docs.
