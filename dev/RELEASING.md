# Releasing

A pushed `v*` tag runs `.github/workflows/release.yml`.
It checks that the tag matches the `frame-trace` version in `Cargo.toml`, runs rustfmt, clippy and the tests, builds the binaries and creates a **draft** GitHub release.
Nothing is public until you publish the draft.

1. Set the new version in `[workspace.package]` in `Cargo.toml`, then run `cargo build` so `Cargo.lock` follows.
2. In `CHANGELOG.md`, rename `## [X.Y.Z] - Unreleased` to `## [X.Y.Z] - YYYY-MM-DD` and check the entries. The release notes are this section.
3. Commit, and merge to `main`.
4. Tag the merged commit and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "frame-trace X.Y.Z"
   git push origin vX.Y.Z
   ```

5. Wait for the Release workflow. Open the draft release, check the four archives and `SHA256SUMS`, then click **Publish release**.

## What the release contains

| File | Built on |
| --- | --- |
| `frame-trace-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | `ubuntu-22.04`, for an older glibc |
| `frame-trace-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz` | `ubuntu-22.04-arm` |
| `frame-trace-X.Y.Z-aarch64-apple-darwin.tar.gz` | `macos-15` |
| `frame-trace-X.Y.Z-x86_64-apple-darwin.tar.gz` | `macos-15-intel` |
| `SHA256SUMS` | SHA-256 of each archive |

Each archive holds the `frame-trace` binary, `README.md` and `LICENSE`.
Each archive also has a build provenance attestation. Check one with `gh attestation verify <archive> --repo agustinustheo/frame-trace`.

## Re-running a release

`gh release create` fails if a release for the tag already exists.
Delete the draft first, then re-run the workflow, or start it from the Actions tab with the `tag` input.

## Polkadot SDK upgrades

The replay crate pins the Polkadot SDK crates with `=` to the versions of one stable release, today `polkadot-stable2609`.
Dependabot groups them in one pull request.
Move them together, then update the release named in `docs/reference/support.md` and the comment on `HostFunctions` in `crates/replay/src/lib.rs`.
