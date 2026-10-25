# Writing the frame-trace docs

Rules for anyone editing `README.md` or `docs/`. This file is not part of the book.

The model is Sentry's docs: the README is a shop window, the first page ends with the reader looking at their own trace, guides are named after jobs, and every number says what it means.

## Page types

| Kind | Reader is… | The page… |
| --- | --- | --- |
| Getting started | new | goes from nothing to a trace in Jaeger, showing what you should see at each step |
| Guide | doing a job | solves one task. The title is the job: "Find the reads that go to the trie" |
| Concept | trying to understand a number | explains one idea, what it means and what it does not mean |
| Reference | looking something up | tables of commands, flags, span attributes, JSON fields |
| Troubleshooting | stuck | one heading per error message, then what it means and the fix |

## Rules

1. **Result first.** Say what the reader gets before how frame-trace does it.
   - Before: "Self-replay uses safe RPC and authenticated parent-state proofs."
   - After: "Replay any block on your laptop and see every storage read and write it made."
2. **Show what the reader should see.** After every command, paste the real output, cut with `...` if long. After opening Jaeger, say what is on screen. Never invent output.
3. **One term per idea, defined once.** Use these words and no others:
   - **read** and **write**, not access, hit, fetch or lookup;
   - **cache** (the value was already in memory for this block), **trie** (frame-trace had to look it up in the chain state);
   - **proof bytes** (how much this read added to the storage proof);
   - **phase** (initialization, an extrinsic, on_poll, finalization);
   - **replay** (running the block again on your machine);
   - **root check** (the replay produced the same state root as the chain).
   Link to the concept page the first time a term appears on a page.
4. **Every number has a unit and a meaning.** "proof bytes: 332 (bytes this read added to the block's proof)". Also say what it does not mean: replay times are measured on your machine, not on the node.
5. **Short sentences.** Aim for 20 words or fewer. One idea per paragraph, at most three sentences in the README.
6. **No internal names.** Never write E1, E2, oracle, discovery run, measured run, ProofDb, overlay, backend_warm, synthetic, schema internals, crate names, or commit hashes in user pages. Use the plain words from rule 3. Raw JSON values (`overlay`, `backend_cold`) appear only in the JSON reference.
7. **Defaults and limits live in reference tables,** not in prose. A guide says "raise `--proof-concurrency` on a fast connection" and links to the table.
8. **Hide mechanisms the reader doesn't act on.** How proofs are fetched, predicted and cached belongs in one concept page at most.
9. **Commands are copyable.** Say where to run them. Use `frame-trace` as the command name; Getting started shows once how to install it. Placeholders look like `<block-number>`.
10. **At most one warning per page,** as a blockquote starting with **Warning:**, next to the step it applies to.
11. **Screenshots have callouts.** Every image is annotated with numbered labels that the text refers to. Terminal output is text, not images.
12. **No filler.** No "simply", "just", "easily", "note that", "it is important to", "in this guide we will". Headings in sentence case.
13. **Only document what exists.** Every flag and output in the docs must match `frame-trace <command> --help` and a real run.
