//! Where a block trace and its metadata come from.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Args};
use frame_trace_decode::Metadata;

/// Arguments that select the block to trace.
#[derive(Args)]
#[command(group(ArgGroup::new("source").required(true).args(["rpc", "offline", "from_file", "from_json"])))]
pub struct SourceArgs {
    /// Recorded `state_traceBlock` response (JSON-RPC envelope or bare result).
    #[arg(
        long,
        value_name = "FILE",
        conflicts_with_all = ["rpc", "offline", "from_json"]
    )]
    pub from_file: Option<PathBuf>,
    /// Read saved block --json output without replay.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["rpc", "offline", "from_file", "save", "metadata", "block"])]
    pub from_json: Option<PathBuf>,
    /// Runtime metadata of the block's runtime: SCALE bytes (`.scale`), or hex as returned by
    /// `state_getMetadata` (`.hex`, or the JSON-RPC response).
    #[arg(long, value_name = "FILE", requires = "from_file")]
    pub metadata: Option<PathBuf>,
    /// Node RPC endpoint, for example wss://westend-asset-hub-rpc.polkadot.io.
    #[arg(
        long,
        value_name = "URL",
        requires = "block",
        conflicts_with = "offline"
    )]
    pub rpc: Option<String>,
    /// Block number, hash or `best` to trace with --rpc.
    #[arg(long, value_name = "NUMBER|HASH|best")]
    pub block: Option<String>,
    /// Replay a saved artifact with no network.
    #[arg(long, value_name = "DIR", conflicts_with = "block")]
    pub offline: Option<PathBuf>,
    /// Save the replay context and proofs for offline use.
    #[arg(long, value_name = "DIR", conflicts_with = "from_file")]
    pub save: Option<PathBuf>,
    /// Maximum concurrent proof requests (1–64; up to 32 keys per request).
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=64))]
    pub proof_concurrency: u8,
    /// Persistent authenticated proof nodes and compiled Wasm cache directory.
    #[arg(long, value_name = "DIR")]
    pub cache_dir: Option<PathBuf>,
    /// Disk cache limit in bytes; runtime code in use is kept.
    #[arg(long, default_value_t = 1073741824)]
    pub cache_max_bytes: u64,
    /// Allow unverified metadata from the node or saved replay.
    #[arg(long = "unauthenticated-metadata", alias = "rpc-metadata-fallback")]
    pub rpc_metadata_fallback: bool,
    /// Execute this Wasm instead of the authenticated :code, for example the same runtime
    /// built with `with-tracing`. Bypasses the code-hash check. Labelled on stderr, in JSON
    /// and in OTLP. Warns when its runtime version differs from the chain's. Cannot be
    /// combined with --save.
    #[arg(long, value_name = "WASM", conflicts_with_all = ["from_file", "from_json", "save"])]
    pub runtime_override: Option<PathBuf>,
}

/// Read a `--runtime-override` Wasm (plain or SDK-compressed) before any network work.
pub fn read_runtime_override(path: Option<&Path>) -> Result<Option<Vec<u8>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let wasm = std::fs::read(path)
        .with_context(|| format!("reading --runtime-override {}", path.display()))?;
    anyhow::ensure!(
        !wasm.is_empty(),
        "--runtime-override {} is empty",
        path.display()
    );
    frame_trace_replay::check_runtime_wasm(&wasm)
        .with_context(|| format!("--runtime-override {}", path.display()))?;
    eprintln!(
        "RUNTIME OVERRIDE: {} will execute instead of the chain's authenticated :code",
        path.display()
    );
    Ok(Some(wasm))
}

/// The JSON/OTLP label for a substitute runtime.
pub fn override_info(
    o: Option<frame_trace_replay::RuntimeOverride>,
    path: Option<&Path>,
) -> Option<frame_trace_decode::RuntimeOverrideInfo> {
    o.map(|o| frame_trace_decode::RuntimeOverrideInfo {
        path: path.map(|p| p.display().to_string()).unwrap_or_default(),
        code_hash: format!("{:?}", o.override_code_hash),
        chain_code_hash: format!("{:?}", o.chain_code_hash),
        version_differences: o.version_differences,
    })
}

/// A block trace and what is needed to decode it.
pub struct Loaded {
    pub traced: frame_trace_model::TracedBlock,
    pub metadata: Option<Metadata>,
    pub block_number: Option<u64>,
    pub execution: Option<frame_trace_decode::ExecutionInfo>,
    pub runtime_override: Option<frame_trace_replay::RuntimeOverride>,
}

pub fn load(args: &SourceArgs) -> Result<Loaded> {
    let runtime_override = read_runtime_override(args.runtime_override.as_deref())?;
    if let Some(dir) = &args.offline {
        let replay = frame_trace_replay::Artifact::load(dir)
            .and_then(|artifact| {
                frame_trace_replay::replay_with_session(
                    artifact,
                    None,
                    &frame_trace_replay::ReplaySession::with_cache_max_bytes(
                        args.cache_dir.as_deref(),
                        args.cache_max_bytes,
                    )?
                    .with_rpc_metadata_fallback(args.rpc_metadata_fallback)
                    .with_runtime_override(runtime_override),
                )
            })
            .map_err(|e| anyhow::anyhow!("{}: {e}", dir.join("artifact.json.zst").display()))
            .inspect_err(|_| {
                eprintln!("root check: NOT VERIFIED; RPC calls: 0");
            })?;
        return replay_loaded(replay, args.save.as_deref());
    }
    match (&args.from_file, &args.rpc) {
        (Some(trace), _) => file::load(trace, args.metadata.as_deref()),
        (None, Some(url)) => {
            let rpc = frame_trace_replay::rpc::Rpc::with_options(
                url,
                args.proof_concurrency as usize,
                std::time::Duration::from_secs(30),
                Default::default(),
            )
            .inspect_err(|_| {
                eprintln!("root check: NOT VERIFIED; RPC calls: 0");
            })?;
            let replay = frame_trace_replay::online_with_session(
                &rpc,
                args.block.as_deref().unwrap_or_default(),
                &frame_trace_replay::ReplaySession::with_cache_max_bytes(
                    args.cache_dir.as_deref(),
                    args.cache_max_bytes,
                )?
                .with_rpc_metadata_fallback(args.rpc_metadata_fallback)
                .with_runtime_override(runtime_override),
            )
            .inspect_err(|_| {
                eprintln!("root check: NOT VERIFIED; RPC calls: {}", rpc.count());
            })?;
            replay_loaded(replay, args.save.as_deref())
        }
        (None, None) => bail!("pass --rpc, --offline or --from-file"),
    }
}

mod file {
    use super::*;

    pub fn load(trace: &Path, metadata: Option<&Path>) -> Result<Loaded> {
        let json = std::fs::read_to_string(trace)
            .with_context(|| format!("reading {}", trace.display()))?;
        let trace_path = trace;
        let trace = frame_trace_model::rpc::parse_trace_block(&json)
            .with_context(|| format!("parsing {}", trace.display()))?;
        let metadata = metadata.map(load_metadata).transpose()?;
        Ok(Loaded {
            traced: frame_trace_model::TracedBlock::from_trace(&trace)
                .with_context(|| format!("converting trace from {}", trace_path.display()))?,
            metadata,
            block_number: None,
            execution: None,
            runtime_override: None,
        })
    }

    /// Accepts SCALE bytes (starting with the `meta` magic), a hex string (with or without
    /// `0x`), or a JSON-RPC response whose `result` is that hex string.
    fn load_metadata(path: &Path) -> Result<Metadata> {
        let context = || format!("reading metadata from {}", path.display());
        let raw = std::fs::read(path).with_context(context)?;
        if raw.starts_with(b"meta") {
            return Metadata::from_bytes(&raw).with_context(context);
        }
        let text = String::from_utf8(raw)
            .context("metadata is neither SCALE bytes nor text")
            .with_context(context)?;
        let text = text.trim();
        let hex = if text.starts_with('{') {
            let value: serde_json::Value = serde_json::from_str(text).with_context(context)?;
            value["result"]
                .as_str()
                .context("no string `result` in the JSON")
                .with_context(context)?
                .to_owned()
        } else {
            text.to_owned()
        };
        Metadata::from_hex(&hex).with_context(context)
    }
}

/// Both block and summary consume the same faithful measured result.
pub fn replay_loaded(replay: frame_trace_replay::Replay, save: Option<&Path>) -> Result<Loaded> {
    eprintln!(
        "root check: {}{}; RPC calls: {} (timed replay: {}); execution proof: {} bytes",
        if replay.root_check { "PASS" } else { "FAIL" },
        if replay.runtime_override.is_some() {
            " with RUNTIME OVERRIDE"
        } else {
            ""
        },
        replay.rpc_count,
        replay.measured_rpc_count,
        replay.proof_bytes
    );
    if let Some(dir) = save {
        replay.artifact.save(dir)?;
    }
    eprintln!(
        "fetching state #{}: {} proof request rounds; {} RPC calls",
        replay.block_number, replay.proof_round_trips, replay.rpc_count
    );
    eprintln!(
        "state root: computed {:?}; header {:?}",
        replay.computed_state_root, replay.expected_state_root
    );
    let metadata = if replay.artifact.metadata.is_empty() {
        None
    } else {
        match Metadata::from_hex(&replay.artifact.metadata) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                eprintln!(
                    "warning: metadata decoding failed: {error}; continuing with undecoded names (names require metadata V14+)"
                );
                None
            }
        }
    };
    Ok(Loaded {
        traced: frame_trace_model::TracedBlock::from_replay(&replay.capture)?,
        metadata,
        block_number: Some(replay.block_number),
        execution: Some(frame_trace_decode::ExecutionInfo {
            root_check: replay.root_check,
            proof_bytes: replay.proof_bytes,
            rpc_calls: replay.rpc_count,
            measured_rpc_calls: replay.measured_rpc_count,
            proof_rounds: replay.proof_round_trips,
        }),
        runtime_override: replay.runtime_override,
    })
}
