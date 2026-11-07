# Releasing

polkadot-frame-trace has two release channels, like Rust's stable and nightly:

| Channel | Built from | Published as | `--version` prints |
| --- | --- | --- | --- |
| Stable | a `vX.Y.Z` tag | a GitHub release, by hand from a draft | `polkadot-frame-trace X.Y.Z` |
| Nightly | `main`, each night | the `Nightly` pre-release on the tag `nightly`, replaced each night | `polkadot-frame-trace X.Y.Z-nightly.YYYYMMDD+<short commit>` |

Both build the same four targets with the same packaging, in `.github/workflows/build.yml`.
`release.yml` and `nightly.yml` call it, so the two cannot drift.

Stable releases also publish the five crates to crates.io. Nightly builds and pre-releases do not.
Users install a release binary, a nightly binary, or build from Git; see `docs/getting-started/install.md`.

## Stable releases

A pushed `v*` tag runs `.github/workflows/release.yml`.
It checks that the tag matches the `polkadot-frame-trace` version in `Cargo.toml`, runs rustfmt, clippy and the tests, builds the binaries and creates a **draft** GitHub release.
Then it publishes the crates to crates.io, so the crates are public before you publish the draft.

1. Set the new version in `[workspace.package]` in `Cargo.toml`, and in the `version` of the four `polkadot-frame-trace-*` entries in `[workspace.dependencies]`. Then run `cargo build` so `Cargo.lock` follows.
2. In `CHANGELOG.md`, rename `## [X.Y.Z] - Unreleased` to `## [X.Y.Z] - YYYY-MM-DD` and check the entries. The release notes are this section. The workflow fails while the heading still says `Unreleased` or the date is not a calendar date.
3. Commit, and merge to `main`.
4. Tag the merged commit and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "polkadot-frame-trace X.Y.Z"
   git push origin vX.Y.Z
   ```

5. Wait for the Release workflow. Open the draft release, check the four archives and `SHA256SUMS`, then click **Publish release**.

### crates.io

After the draft release is created, the `crates` job of the Release workflow publishes the workspace with `cargo publish --workspace --locked`.
Cargo publishes the crates in dependency order: `polkadot-frame-trace-model`, `-decode`, `-replay`, `-otel`, then `polkadot-frame-trace`.
It uses the crates.io API token in the repository secret `CARGO_REGISTRY_TOKEN`.

- It runs only for versions without a suffix. `0.2.0-rc.1` gets a draft pre-release and nothing on crates.io.
- It runs only for a tag push to `agustinustheo/polkadot-frame-trace`. A run started by hand with **Run workflow**, or in a fork, publishes nothing. To publish after a failure, re-run the tag's run.
- It asks crates.io for each crate's version first and leaves out the ones already there. A re-run after a partial failure publishes the rest.
- crates.io versions are permanent. A wrong one can only be yanked, so the job runs on a tag that passed the checks and built.

The packages hold the sources, `Cargo.toml`, `LICENSE`, the README for `polkadot-frame-trace` and `ARTIFACT.md` for `polkadot-frame-trace-replay`.
They leave out `tests/`, `examples/`, `fixtures/` and `dev/`: the tests need the fixtures, which stay in the repository.
Check a change to the packaging with:

```sh
cargo +1.93.0 package --workspace --locked --no-verify --list
cargo +1.93.0 publish --workspace --dry-run --locked
```

### What the release contains

| File | Built on |
| --- | --- |
| `polkadot-frame-trace-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | `ubuntu-22.04`, for an older glibc |
| `polkadot-frame-trace-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz` | `ubuntu-22.04-arm` |
| `polkadot-frame-trace-X.Y.Z-aarch64-apple-darwin.tar.gz` | `macos-15` |
| `polkadot-frame-trace-X.Y.Z-x86_64-apple-darwin.tar.gz` | `macos-15-intel` |
| `SHA256SUMS` | SHA-256 of each archive |

Each archive holds the `polkadot-frame-trace` binary, `README.md` and `LICENSE`.
Each archive also has a build provenance attestation. Check one with `gh attestation verify <archive> --repo agustinustheo/polkadot-frame-trace`.

### Re-running a release

`gh release create` fails if a release for the tag already exists.
Delete the draft first, then re-run the failed run.
To start a new run, open the Release workflow in the Actions tab, click **Run workflow** and choose the tag, not a branch, under **Use workflow from**.
A run started from a branch fails, so the provenance attestation always names the tag's commit.

A version with a suffix, such as `0.2.0-rc.1`, becomes a draft pre-release.

## Nightly builds

`.github/workflows/nightly.yml` runs at 03:17 UTC each day, and by hand from the Actions tab with **Run workflow**.
Only one nightly runs at a time; a second one waits for the first.

1. The `test` job checks out the run's commit and runs rustfmt, clippy, the offline tests, the ignored live tests and a live replay of the latest finalized Westend Asset Hub block. Live failures can come from the public endpoints. The run's commit is the head of `main` when the run was triggered, even if the run waited in the queue while `main` moved. The provenance attestation names that commit, so the workflow builds it and checks it built it.
2. If they pass and the published `Nightly` release is not complete and built from this commit, the `build` job builds the four archives with `build.yml`. The archives are named `polkadot-frame-trace-nightly-<target>.tar.gz`, so their download links stay the same. The version comes from `FRAME_TRACE_BUILD_VERSION`, which only this workflow sets; local builds and `release.yml` builds print the crate version.
3. The `publish` job writes `SHA256SUMS` and attests build provenance. It uploads the new release in full as a draft under a temporary tag name, which creates no tag. Then it deletes the old `Nightly` release and its assets, moves the `nightly` tag to the built commit, and publishes the draft as the `Nightly` pre-release on that tag. If any of this fails, it deletes the draft. It is never marked as the latest release. Its notes give the date, the commit, the run and a line pointing stable users to the latest `v*` release.

The run still tests but does not publish if a published, not draft, `Nightly` release exists, has all four archives and `SHA256SUMS`, and its tag points to the run's commit.
Otherwise it publishes, so a night whose publish failed is repaired by the next one, even if `main` did not change.
If the upload fails, the old release and tag stay as they were.
To publish anyway, run the workflow by hand with **force** checked.
A run started from a branch other than `main` tests but never publishes, and never touches the failure issue.

Moving the `nightly` tag does not start `release.yml`: it runs only for tags matching `v*`, and tags pushed with `GITHUB_TOKEN` start no workflows.

### When a nightly fails

The `notify` job runs after every nightly for `main`, scheduled or started by hand.
If any job failed or was cancelled, it comments on the open issue **Nightly build failed** with the run link, the failed jobs and the date. If no such issue is open, it opens one with the label `nightly-failure`, and creates the label if it is missing.
The next nightly that passes, including one that skips publishing because `main` did not change, comments on the issue and closes it.
It only uses an issue it opened itself: one by `github-actions[bot]` whose body holds the hidden marker `<!-- frame-trace-nightly -->`. An issue a person opens with the same title and label is left alone.
This issue is the only notification. Watch the repository's issues, or the label, to hear about failures.

Each job has only the permissions it needs: `contents: read` to test and build, `contents: write`, `id-token: write` and `attestations: write` to publish, and `issues: write` to report.

## Polkadot SDK upgrades

The replay crate pins most Polkadot SDK crates (`sc-executor`, `sp-io`, `sp-runtime`, `sp-trie` and the others in `crates/replay/Cargo.toml`) with `=` to the versions of one stable release, today `polkadot-stable2609`.
Two use caret ranges: `sp-panic-handler = "13.0.2"` in `crates/replay/Cargo.toml`, a patch newer than the 13.0.0 in stable2609, and `sp-crypto-hashing`, workspace `"0.1.0"` in `Cargo.toml`. They change rarely.
Dependabot groups the Polkadot SDK crates in one pull request. Move the pinned crates together, to one stable release.

Then run the offline tests and the live tests (see [Live tests](README.md#live-tests)):

```sh
cargo +1.93.0 test --workspace --locked
cargo +1.93.0 test --workspace --locked -- --ignored
```

The saved blocks in `fixtures/replay/` are chain data, so an upgrade does not change them.
If an offline test fails with a missing proof node, the new code reads state the fixture did not save.
Re-record that fixture with its command in `fixtures/replay/README.md`; the `call-people-*` fixtures are regenerated offline with the `record_calls` example.
`fixtures/wasm/missing-host-function.wasm` is built from hand-written WAT and does not depend on the SDK.

Then update every place that names the release or the crate versions:

- `docs/reference/support.md`, the release in **Host functions**;
- `docs/troubleshooting.md`, the missing host functions section;
- `docs/guides/see-runtime-calls.md`, the crate versions the `with-tracing` feature was checked against;
- `dev/README.md`, the release in **Add host functions for a chain**;
- `crates/replay/src/lib.rs`, the comment on `HostFunctions`;
- `crates/replay/ARTIFACT.md`, the release and the SDK source links;
- `fixtures/replay/README.md`, the host crates the fixtures run with.
