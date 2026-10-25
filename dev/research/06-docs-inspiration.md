# Docs inspiration for frame-trace

Research for rewriting the README and docs. Audience: runtime and pallet developers, and node operators debugging a slow or heavy block. Goal: a reader gets a trace in Jaeger in under five minutes, then learns what each number means.

Caveat: page contents were extracted by a summarising fetch tool. Quotes below are close to the source text but were not checked character by character. Re-check any quote before reusing it publicly.

## 1. What the sources do

### Sentry (product docs, Rust SDK, Trace View, Insights, Trace Explorer)

- **Rust SDK README** (github.com/getsentry/sentry-rust): opens with the logo, the H1 "Sentry SDK for Rust", then badges (CI, codecov, crates.io and docs.rs per crate). It has no hero image, no install snippet and no code example. It is about 300 words and sends the reader to the docs site. Lesson: a README can be a signpost. The docs carry the tutorial.
- **Rust SDK docs page** (docs.sentry.io/platforms/rust/): one intro line, "On this page, we get you up and running with Sentry's Rust SDK." Then fixed sections: Install, Configure, Verify Setup. Verify Setup triggers a deliberate `panic!` so the user sees a result. No tabs. Bold "Important:" and "Integrations:" notes for warnings. Lesson: the first result is a test event the user can see.
- **Sentry root README** (github.com/getsentry/sentry): "Users and logs provide clues. Sentry provides answers." Then a row of product screenshots (traces, trace explorer, insights, and so on). No feature bullet list. Lesson: screenshots do the selling.
- **Trace View** (docs.sentry.io/product/sentry-basics/tracing/trace-view/): no screenshots and no callouts. The waterfall is described in prose. Terms are defined in passing: a root is "a transaction without any parents", and an orphan trace is one whose transactions cannot be connected to the root. Lesson: this page is weaker than its product, and the gap shows.
- **Trace Explorer** (docs.sentry.io/product/explore/trace-explorer/): "Key Concepts" comes first, then "Practical Examples". Span is defined as "A span represents an individual operation within a trace." Duration is defined as "The time the operation took, used to measure performance." Sampling has its own section with a "How It Works" subsection. Lesson: define the nouns before the tasks, then give tasks.
- **Insights** (docs.sentry.io/product/insights/): dashboard names are noun phrases ("Queries and Query Details"). The one imperative title is "Triage Performance Issues". The example "Find slow database queries" I expected was not on the page I fetched, so I cannot cite it. Lesson: name pages after the job, and prefer nouns for reference pages.

### Honeycomb, Grafana Tempo, Jaeger, OpenTelemetry (observability basics)

- **Honeycomb concepts**: a six-item list of linked concepts (OpenTelemetry, Distributed Tracing, Services, High Cardinality, Instrumentation, eBPF), each one line. No diagrams and no definitions on the index page. Lesson: a concept index should be a menu, not a lecture.
- **Grafana Tempo getting started**: four numbered steps (plan, deploy, instrument, collect). The page never says what the user sees at the end. It defines a trace as "the lifecycle of a request as it passes through a set of applications". Its one diagram is the pipeline. Lesson: do not stop at the setup. Say what the user sees.
- **Jaeger getting started**: starts with one `docker run` command and then "open http://localhost:16686". No screenshots on that page. It points to a separate Introduction page for what a trace is. Lesson: the single-command start is good. The gap is the missing "what you see" step.
- **Jaeger frontend UI docs**: configuration reference. The trace view (timeline, span tree, span details) is barely explained. Lesson: a reference page for UI settings is fine, but the trace view needs its own explanation somewhere.
- **OpenTelemetry traces concept**: "Spans are the building blocks of Traces." "Attributes are key-value pairs that contain metadata that you can use to annotate a Span..." The page has no image. It uses JSON span examples where `parent_id` shows the hierarchy. Lesson: define a span and an attribute in one sentence each, with a concrete example.

### Developer tools (tokio-console, samply, cargo-flamegraph, k6)

- **tokio-console README**: title, four badges, then two static PNGs with a caption: "wow! whoa! it's like `top(1)` for tasks!". It compares the tool to something the reader already knows. Install is `cargo install --locked tokio-console`. The first command is `tokio-console`, and the README explains what it connects to by default. Lesson: compare to a familiar tool in the caption.
- **samply README**: first sentence "samply is a command line CPU profiler which uses the Firefox profiler as its UI." Install has three methods. The first command appears right after the intro: `samply record ./my-application my-arguments`. About 900 words. Lesson: one-sentence definition, then the command.
- **cargo-flamegraph README**: hero image of a flamegraph first, then Quick Start, then installation. About 2,000 words, most of them educational material after the usage section. Lesson: lead with the picture and the command. Move theory to a separate page.
- **k6 results output** (now under grafana.com/docs/k6): defines `http_req_duration` as "the end-to-end time of all requests (that is, the total latency)". It does not define `p(95)`, and it shows no sample output. Lesson: every metric needs a one-line meaning, and examples should show real output.

### Blockchain analogues (Foundry, Tenderly)

- **Foundry `cast run` / `forge test -vvvv`**: the reference page did not fetch cleanly. A search summary of the Foundry docs says each trace line shows gas in brackets, `Contract::function`, the call type, the return value or revert reason, and indentation for call depth. I have not verified this against the page itself. Lesson: a trace line is easiest to learn when its format is documented as a template.
- **Tenderly debugger** (docs.tenderly.co/debugger): the docs say the debugger shows "every call made during a transaction's life cycle". Call-info fields are listed as bullets (function name, opcodes, addresses, decoded inputs and outputs, gas stats) but none is defined. There are five screenshots. Lesson: list the fields, and define the ones that a developer will misread.

## 2. Patterns that recur

1. The README is a shop window: a picture, one sentence, one command, a link to the docs.
2. Each tutorial ends with what the reader should see.
3. Nouns are defined once, early, in a Concepts section, then used the same way everywhere.
4. Numbers come with a meaning and a unit next to them.
5. Reference pages list flags and fields as tables. Guides name a task.
6. Troubleshooting is organised by symptom, using the error text as the heading.

## 3. Recommendation for frame-trace

### README outline (target 120 to 170 lines, under 600 words; the quick start is visible on the first screen)

1. **H1 `frame-trace`** and one sentence: "Replay one block of a Polkadot SDK chain on your machine and see every storage read and write as a trace in Jaeger or Tempo."
2. **Hero image**: an annotated Jaeger screenshot of a `Utility.batch` extrinsic (see section 5). The caption is one line.
3. **What you get**, three or four bullets. Each bullet names a result, not a feature: "Every storage read, marked cache or trie." "Proof size per call." "Decoded storage names, such as `System.Account`."
4. **Five-minute start**: `docker run` for Jaeger, one `frame-trace block` command against a public RPC, then "Open http://127.0.0.1:16686 and select service `frame-trace`." End with a short block of the expected terminal output (text, not an image).
5. **Where to go next**: three links, to the first trace tutorial, to the guides index, and to the reference.
6. **Requirements**: Rust toolchain version, RPC endpoint, and the archive-node note for older blocks.
7. **Licence and contributing**: two lines each.

Move out of the README: the discovery and proof-concurrency paragraphs, the `follow` internals, the cache-directory details and the "Reports across blocks" section. They belong in guides, concepts and reference.

### Docs navigation (mdBook SUMMARY.md)

```
# Summary

[Introduction](README.md)

# Getting started
- [Install and check the build](getting-started/install.md)
- [Your first trace in Jaeger](getting-started/first-trace.md)
- [Reading a trace](getting-started/reading-a-trace.md)

# Guides
- [Trace a block from an RPC node](guides/trace-a-live-block.md)
- [Replay a saved block offline](guides/replay-offline.md)
- [Find the calls that write the most storage](guides/find-heavy-writes.md)
- [Find reads that go to the trie instead of the cache](guides/find-trie-reads.md)
- [Measure proof size for a block](guides/measure-proof-size.md)
- [Trace a runtime API call](guides/trace-runtime-api.md)
- [Follow new blocks and catch reorgs](guides/follow-new-blocks.md)
- [Compare two blocks](guides/compare-blocks.md)
- [Send traces to Tempo](guides/send-to-tempo.md)
- [Reuse proofs across runs with a cache directory](guides/reuse-cache.md)

# Concepts
- [What a replay proves](concepts/what-a-replay-proves.md)
- [Read origin: cache or trie](concepts/read-origin.md)
- [Proof bytes](concepts/proof-bytes.md)
- [How predictions speed up fetching](concepts/predictions.md)

# Reference
- [Commands and flags](reference/commands.md)
- [Span names and attributes](reference/spans.md)
- [JSON output](reference/json-output.md)
- [Saved block format](reference/saved-blocks.md)

# Troubleshooting
- [No traces in Jaeger](troubleshooting/no-traces.md)
- [Missing proof](troubleshooting/missing-proof.md)
- [Storage root mismatch](troubleshooting/root-mismatch.md)
- [Old block fails to fetch](troubleshooting/archive-node.md)
- [First run is slow](troubleshooting/first-run-slow.md)
```

Existing files `docs/runtime-spans.md` and `docs/json-schema.md` map to Reference. Keep the names, move them under `reference/`.

Guide titles should name the job, not the tool. Good: "Find the calls that write the most storage". Avoid: "Storage analysis" or "Using the write filter".

### Writing rules, with before and after from the current README

1. **Lead with the result.** Before: "Replay a Polkadot SDK block and view its storage accesses in Jaeger or Tempo." After: "Replay one block of a Polkadot SDK chain on your machine and see every storage read and write as a trace in Jaeger or Tempo." (Same facts, and the reader knows what they get.)
2. **Put the command in the first screen.** Model: samply's first command appears right after its intro.
3. **Show what the reader should see.** Before: the README ends the Jaeger step with the URL. After: "You should see one trace named `Core_execute_block`. Click it to open the timeline." (Tempo's getting-started page has this gap, so avoid it.)
4. **One term per concept.** Choose "read" and "write", "origin" with values `cache` and `trie`, and "proof bytes". Do not alternate with "access", "hit", "fetch" or "lookup" in the same guide.
5. **Define a term at first use, then link.** Before: "Discovery batches up to 32 predicted storage keys per proof." After: "frame-trace asks the node for up to 32 storage keys per request, eight requests at a time by default. Tune this with `--proof-concurrency`. See [How predictions speed up fetching](concepts/predictions.md)."
6. **Give every number a meaning, unit and source.** Model: k6's "the end-to-end time of all requests (that is, the total latency)". Before: "The measured run uses a warm cache and zero RPC." After: "A warm-cache run makes zero RPC calls. Each `proof bytes` value is the size of the storage proof for that call, in bytes."
7. **Explain what a number does not mean.** Sentry's sampling section explains how extrapolated counts work. For frame-trace, say that proof bytes are not the same as the block's total weight, if that is true.
8. **Short sentences.** Aim for twenty words or fewer. Before: "Each replay prints the computed and header state roots, their comparison and RPC count." After: "Each run prints two state roots and whether they match. It also prints the number of RPC calls it made."
9. **Limits and defaults go in reference, not in prose.** Before: "Discovery ... uses eight concurrent requests by default (`--proof-concurrency 1..64`)." After (guide): "Raise `--proof-concurrency` on a fast connection." After (reference table): `--proof-concurrency` default 8, range 1 to 64.
10. **Hide mechanisms the reader does not act on.** Before: "speculative outputs are never exported or accepted." After (concepts page only): "Predictions only speed up fetching. Only the authenticated run's results are exported."
11. **One idea per paragraph, three sentences maximum in the README.** cargo-flamegraph's theory sections are long for a reason, but they belong in a separate page.
12. **One warning per page, bold label.** Sentry uses "**Important:**" and "**Integrations:**". Use "**Warning:**" for the archive-node requirement and nothing else.
13. **Troubleshooting headings are the error text.** Before: a paragraph about proof problems. After: `### Missing proof` followed by "What it means", "Why it happens", "Fix".
14. **Name guides by the job.** Before: "Following blocks" (noun). After: "Follow new blocks and catch reorgs" (imperative).
15. **Avoid "simply", "just", "easily", "obviously".** Remove the word and check the sentence still reads.

### Screenshots and diagrams worth making

1. **Hero (README)**: an annotated Jaeger view of the existing `Utility.batch` trace. Numbered callouts: (1) the extrinsic root span, (2) the three `transfer_keep_alive` calls, (3) a storage read tagged `cache`, (4) a storage read tagged `trie`, (5) a write, (6) the proof-bytes attribute, (7) the duration bar. Keep the original `docs/images/jaeger-batch.png` as the unannotated reference. Create a new file for the annotated version.
2. **Reading a trace (getting started)**: a single annotated figure of the Jaeger trace page with four labelled regions: the span tree, the timeline, the span details, and the tags. Each label uses the same terms as the Concepts pages.
3. **Read origin comparison (concepts)**: two side-by-side span detail panels for the same key, one from cache and one from trie, with the differing attribute circled.
4. **Pipeline diagram (concepts or README)**: a simple four-box flow: RPC node, authenticated proofs, local runtime execution, OTLP export to Jaeger or Tempo. One diagram, no more.
5. **Tempo variant (guide)**: one screenshot of the same trace in Grafana Tempo, showing it is the same data. Optional.
6. **Terminal output (README and getting started)**: a text block, not an image. Text is searchable and copyable.

Avoid: a chart of performance numbers in the README, animated GIFs of terminal sessions (they go stale when flags change), and any screenshot that has no callouts.

## 4. Next steps

1. Write `getting-started/first-trace.md` first. Test it on a clean machine. If it takes longer than five minutes, cut steps.
2. Annotate the hero screenshot.
3. Write the Concepts glossary before the guides, so the guides can link to it.
4. Move the README's internal paragraphs into the matching guide or concept page.
5. Add a docs-build check to CI, the same way polkadot-linter does (mdBook, `book.toml`, `SUMMARY.md`).

## Sources

- https://github.com/getsentry/sentry-rust
- https://docs.sentry.io/platforms/rust/
- https://docs.sentry.io/product/sentry-basics/tracing/trace-view/
- https://docs.sentry.io/product/insights/
- https://docs.sentry.io/product/explore/trace-explorer/
- https://github.com/getsentry/sentry
- https://docs.honeycomb.io/get-started/basics/observability/concepts/
- https://grafana.com/docs/tempo/latest/getting-started/
- https://www.jaegertracing.io/docs/latest/getting-started/
- https://www.jaegertracing.io/docs/latest/frontend-ui/
- https://opentelemetry.io/docs/concepts/signals/traces/
- https://github.com/tokio-rs/console
- https://github.com/mstange/samply
- https://github.com/flamegraph-rs/flamegraph
- https://grafana.com/docs/k6/latest/get-started/results-output/
- https://getfoundry.sh/forge/tests/traces/ (search result summary only)
- https://docs.tenderly.co/debugger
