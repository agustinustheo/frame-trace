# 04 - individuality-community: local network setup for tracing experiments

Source: `/tmp/pm-audit/individuality-community` (HEAD `e69b079`, "Bump verifiable to 0.4.1 (#200)").
Read/grep only. Nothing was built or run. The stale `.cargo/mutants.toml` was ignored.

## 1. Node binaries, runtimes, build, SDK pin

**Binaries.** The repo has no node crate. No `[[bin]]` target and no `sc-cli`/`sc-service` dependency exists in any Cargo.toml. Two external binaries run the chains:

- Relay: `polkadot` (the relay bundle). `e2e/zombienet/network.toml` sets `default_command = "polkadot"` and `chain = "rococo-local"`.
- Collators: `polkadot-omni-node`. `network.toml` sets `command = "polkadot-omni-node"` for both parachains. Omni-node loads the runtime WASM from the chain spec.
- Both come from polkadot-sdk at the tag in `.github/env`: `POLKADOT_RELEASE_VERSION=polkadot-stable2606` and `POLKADOT_RELEASE_PATCH=-2`, so `polkadot-stable2606-2`. `e2e/zombienet/scripts/get_polkadot_binaries.sh` builds them from source. The relay is built with `cargo build --release --locked -p polkadot --features fast-runtime`. Omni-node is built with `SKIP_WASM_BUILD=1 cargo build --release --locked -p polkadot-omni-node`. The script uses `FORCE_SOURCE_BUILD=1`, because prebuilt release binaries lack the `fast_runtime_binary` that `rococo-local` needs.
- `chain-spec-builder` ships inside omni-node (`polkadot-omni-node chain-spec-builder`).

**Runtimes.** Two system-chain runtimes are in `runtimes/`:

- `next-people-paseo` (para 1502), which holds the individuality pallets.
- `next-asset-hub-paseo` (para 1500), which holds Scarcity (`Scarcity: indiv_pallet_scarcity = 58`), the AH-side members subscriber, and dotNS.

Build: `e2e/zombienet/scripts/01-build-runtimes.sh` runs `cargo +1.93.0 build --release --locked -p next-people-paseo-runtime -p next-asset-hub-paseo-runtime` with `SKIP_PALLET_REVIVE_FIXTURES=1`. The WASM is produced by `substrate-wasm-builder` through each runtime's `build.rs` (`runtimes/*/build.rs`). Output: `target/release/wbuild/<runtime>/<name>.compact.compressed.wasm`. CI release builds use `cargo build --profile production` (`.github/actions/build-e2e-runtime-wasm/action.yml`). There is no srtool step in this repo.

**SDK pin.** `Cargo.toml` pins crates.io versions only. There is no `git =` source and no `[patch]` section. Examples: `sp-io = "48.0.0"`, `sp-tracing = "19.0.0"`, `frame-support = "48.1.0"`, `frame-executive = "48.0.0"`, `substrate-wasm-builder = "34.0.1"`, `pallet-revive = "0.19.1"`. Toolchain: `rust-toolchain.toml` pins `1.93.0` with `wasm32-unknown-unknown`. `Cargo.lock` has zero `git+` sources. The SDK release tag lives only in `.github/env`.

## 2. Starting a local network

**Zombienet configs (in `e2e/zombienet/`):**

- `network.toml`: relay `rococo-local` with 4 validators (alice RPC 10000, bob 9955, charlie 9956, dave 9957). Relay args are `--network-backend=libp2p --rpc-max-response-size=50`. The genesis overlay `EnabledHostFunction = "EccRfc163"` is set. People (1502) collator on RPC 10010 and AH (1500) collator on RPC 10020. Both use `polkadot-omni-node` with args `--authoring=slot-based --enable-statement-store --force-authoring --network-backend=libp2p --rpc-max-connections=500 --rpc-max-response-size=50 -lxcm=debug`. The HRMP channels are opened at runtime, not at genesis.
- `network-snap.toml`: the same topology with `db_snapshot = "./.tools/snapshot/<node>.tgz"` per node and a pinned relay `chain_spec_path`.
- `../../docs/examples/people-network.toml` and `assethub-network.toml`: an alternate People-only config. Collator args include `-lparachain=debug,xcm=trace`.

**Scripts (`e2e/zombienet/scripts/`, numbered in order):**

1. `00-install-binaries.sh` installs the polkadot bundle, omni-node and `zombie-cli`. The cold build takes about 30 to 60 min.
2. `01-build-runtimes.sh` builds the two runtime WASMs.
3. `02-gen-specs.sh` runs `chain-spec-builder create --relay-chain rococo --para-id 1502|1500 ... named-preset local_testnet`. It patches the collator to `//People-collator` and `//Ah-collator`, then runs `build-spec --raw`.
4. `03-spawn.sh` runs `exec zombie-cli spawn network.toml --provider native --dir .tools/net`.
5. `04-bootstrap.sh` runs `scripts/initial-setup/start.sh local`. It runs the 31 scripts listed in `start.sh` `SCRIPT_NAMES`, and the long pole is ZK chunk upload (`07-add-zk-chunks.sh`). The README reports about 43 min end to end on an M4 Mac.
6. `05-snapshot.sh` and `06-spawn-from-snapshot.sh` capture and restore the post-bootstrap state (about 1.2 GB). `07-fetch-snapshot.sh` downloads the CI snapshot. Snapshots are only valid for the same specs and binaries.

**Justfile.** `e2e/justfile` wraps the scripts. `just net-up` runs `install-binaries build-runtimes gen-specs spawn`. Then `just bootstrap`, `just snapshot`, `just spawn-snap` and `just fetch-snapshot` are available.

**docker / chopsticks.** No Dockerfile or docker-compose exists in the repo. There are no committed chopsticks configs. `@acala-network/chopsticks` is a devDependency in `e2e/package.json`, used only by `e2e/tests/runtime-upgrade/*.test.ts` (`setupContext`). Those tests fork the live `wss://paseo-people-next-system-rpc.polkadot.io`.

**Fastest path to blocks with individuality pallets (inferred, not run):**

- Cold: `e2e/zombienet/scripts/00-install-binaries.sh`, `01-build-runtimes.sh`, `02-gen-specs.sh`, then `03-spawn.sh`. Use `zombie-cli spawn network.toml`. Only the People collator matters for block production. Bootstrap (`04`) is needed for HRMP, AH subscription and ZK chunks, not for People blocks. This is inferred from the comments, and I did not verify it.
- Warm: `07-fetch-snapshot.sh` plus `06-spawn-from-snapshot.sh`, with the same specs and binaries as the snapshot. It needs `gh auth`.
- Also possible: `docs/examples` `pnpm run node:people`, which builds the People runtime and spawns `docs/examples/people-network.toml`. It needs `zombienet setup polkadot` for the relay bundle.

## 3. Tracing hooks

**Feature passthrough (none in the repo).** No runtime Cargo.toml has a `with-tracing` feature, and no feature passes through to `sp-io`, `sp-tracing` or `frame-executive`. The people runtime (`runtimes/next-people-paseo/Cargo.toml`) has `frame-executive` and `sp-io` as normal deps. It does not depend on `sp-tracing` at all. The AH runtime has `sp-tracing` only under `[dev-dependencies]` (line 178). AH uses it in `src/staking/mod.rs:1193` (`sp_tracing::try_init_simple()`) and in `tests/tests.rs` (`.with_tracing()`). Features that exist upstream in the local registry (`~/.cargo/registry/src/github.com-25cdd57fae9f0462/`):

- `frame-executive-48.0.0`: `with-tracing = ["sp-tracing/with-tracing"]`
- `sp-io-48.0.0`: `with-tracing = ["sp-tracing/with-tracing"]`. `lib.rs:1871` gates the wasm-side init behind `all(substrate_runtime, feature = "with-tracing")`.
- `sp-tracing-19.0.0`: `with-tracing = ["codec/derive", "codec/full"]`.

To use it, a runtime feature would have to forward `with-tracing` to `frame-executive/with-tracing` and `sp-io/with-tracing`. Nothing like that exists here.

**Other flags already set.**

- Only `-lxcm=debug` (`network.toml:81,103`) and `-lparachain=debug,xcm=trace` (`docs/examples/people-network.toml:62`).
- `--rpc-max-response-size=50` and `--rpc-max-connections=500`.
- `--enable-statement-store`.
- No `--tracing-targets` or `--tracing-receiver`.
- No `state_traceBlock` use.
- No `--rpc-methods unsafe` or `--unsafe-rpc-external`.
- No `--enable-offchain-indexing`.

**Log-level caveat.** `01-build-runtimes.sh` does not pass `on-chain-release-build`. That feature sets `sp-api/disable-logging`, so the local WASM keeps its logging. The `on-chain-release-build` feature is used only for on-chain builds.

## 4. Extrinsics for tracing demos

Submission examples that already exist (not vitest):

- `scripts/initial-setup/07-add-zk-chunks.sh`: `ChunksManager.add_chunks` in `Utility.dispatch_as(Authorized)` under `Sudo.sudo`. Many page writes. Strongest demo candidate.
- `scripts/initial-setup/10-subscribe-ah-ring-root-updates.sh`: `MembersNotifier.subscribe`.
- `scripts/initial-setup/08a-setup-people-collection.sh`: `People.create_people_collection` (unsigned).
- `scripts/initial-setup/11-setup-poi-design-families.sh`: a `Utility.batch_all` of `ProofOfInk.add_design_family`.
- `scripts/initial-setup/12a-setup-attestation-invites.sh`: `grant_invites`.
- `scripts/initial-setup/09-override-onboarding-sizes.sh`: `Members.set_onboarding_size`.
- `docs/examples/*.ts` (run with `pnpm run <name>`): `game_participation.ts` (`Game.sign_up_with_account`, `report`), `setup-onboarding.ts`, `bootstrap-chunks.ts`, `airdrop.ts`, `subscriptions.ts`, `coinage.ts`, `dotns.ts`.

Pallet extrinsics with heavy storage access:

- People registration, in `pallets/people-multi/src/lib.rs` and `pallets/people-lite/src/lib.rs`: `People.under_alias`, `set_alias_account`, `force_recognize_personhood`, `PeopleLite.register_with_fee`, `attest`, `register_lite_consumer`.
- Proof-of-ink, in `pallets/proof-of-ink/src/lib.rs`: `apply`, `submit_evidence`, `judged`, `register_referred`, `commit`, `allocate_full`.
- Scarcity (on AH), in `pallets/scarcity/src/lib.rs`: `mint`, `transfer`, `define_item`, `create_collection`, `transfer_by_holder`.
- Members notifier, in `pallets/members-notifier/src/lib.rs`: `send_batch`, `enqueue_updates`, `send_init_page`, `request_replay`, `subscribe_whitelisted`.
- Members, in `pallets/members/src/lib.rs`: `*_authorized` ring builds and `process_maintenance`.

Test code that already builds these extrinsics:

- `runtimes/next-people-paseo/src/integration_tests/lite_people_free_tx.rs`: `register_lite_person` and `register_with_fee`.
- `runtimes/next-people-paseo/src/integration_tests/lite_people_game_flow.rs`.
- `runtimes/next-people-paseo/src/integration_tests/members_notifier_whitelist.rs`.
- `runtimes/next-asset-hub-paseo/tests/scarcity.rs`: scarcity transfers (emulator, Rust).
- `pallets/*/src/tests.rs`, for example `pallets/proof-of-ink/src/tests.rs` and `pallets/members-notifier/src/tests.rs`.

E2E vitest suites do not submit extrinsics. `e2e/suites/initialization-tests/src/network.test.ts` only reads storage. It checks HRMP channels, `MembersNotifier.Subscribers(1500)` and `MembersSubscriber.Subscription`. `e2e/tests/runtime-upgrade/*.test.ts` uses chopsticks against Paseo.

## 5. Benchmarking and storage-access tooling

- `agents/benchmarking.md` holds the weight rules. It requires each `Linear` component to vary the storage keys, and it asks for `integrity_test` checks via `OcwWeightBudget`.
- Each pallet has a `benchmarking.rs` (25 files). Weights live in `runtimes/*/src/weights/` (`indiv_pallet_*.rs`, plus `block_weights.rs`, `extrinsic_weights.rs`).
- `templates/frame-weight-template.hbs` and `templates/xcm-bench-template.hbs` are the weight templates.
- `.github/workflows/test.yml` runs `frame-omni-bencher v1 benchmark pallet` on the runtimes. `cmd-run.yml` runs the benchmark bot. It uses `.github/scripts/weight_deviation.py`.
- `support/src/parameters.rs:91` and `:182` say benchmark builds should keep the storage reads that production performs (the `BenchmarkMax` helper).
- No storage-access or `state_traceBlock`-style tracking exists in the repo. The only `trace` strings are in the `coinage` tests. `.github/scripts/zn-dump-diagnostics.sh` dumps zombienet logs, not storage traces.

## Open questions for the experiment

- Whether the People collator produces blocks without running bootstrap. Not verified.
- Whether `polkadot-omni-node` at `stable2606-2` supports `--tracing-targets` and `state_traceBlock` as expected. Check the omni-node help output after install.
