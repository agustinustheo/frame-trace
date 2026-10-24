//! Where a block trace and its metadata come from.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use frame_trace_decode::Metadata;

/// Arguments that select the block to trace.
#[derive(Args)]
pub struct SourceArgs {
    /// Recorded `state_traceBlock` response (JSON-RPC envelope or bare result).
    #[arg(
        long,
        value_name = "TRACE.json",
        required_unless_present_any = ["rpc", "offline"],
        conflicts_with_all = ["rpc", "offline"]
    )]
    pub from_file: Option<PathBuf>,
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
    /// Block number or hash to trace with --rpc.
    #[arg(long, value_name = "NUMBER|HASH")]
    pub block: Option<String>,
    /// Replay a saved artifact with no network.
    #[arg(long, value_name = "DIR", conflicts_with = "block")]
    pub offline: Option<PathBuf>,
    /// Save the replay context and proofs for offline use.
    #[arg(long, value_name = "DIR", conflicts_with = "from_file")]
    pub save: Option<PathBuf>,
}

/// A block trace and what is needed to decode it.
pub struct Loaded {
    pub traced: frame_trace_model::TracedBlock,
    pub metadata: Option<Metadata>,
    pub block_number: Option<u64>,
}

pub fn load(args: &SourceArgs) -> Result<Loaded> {
    if let Some(dir) = &args.offline {
        let replay = frame_trace_replay::offline(dir).inspect_err(|_| {
            eprintln!("root check: NOT VERIFIED; RPC calls: 0");
        })?;
        return replay_loaded(replay, args.save.as_deref());
    }
    match (&args.from_file, &args.rpc) {
        (Some(trace), _) => file::load(trace, args.metadata.as_deref()),
        (None, Some(url)) => {
            let rpc = frame_trace_replay::rpc::Rpc::new(url).inspect_err(|_| {
                eprintln!("root check: NOT VERIFIED; RPC calls: 0");
            })?;
            let replay =
                frame_trace_replay::online(&rpc, args.block.as_deref().unwrap_or_default())
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
        let trace = frame_trace_model::rpc::parse_trace_block(&json)
            .with_context(|| format!("parsing {}", trace.display()))?;
        let metadata = metadata.map(load_metadata).transpose()?;
        Ok(Loaded {
            traced: frame_trace_model::TracedBlock::from_trace(&trace)?,
            metadata,
            block_number: None,
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
        "root check: PASS; RPC calls: {} (measured: {}); execution proof: {} bytes",
        replay.rpc_count, replay.measured_rpc_count, replay.proof_bytes
    );
    if let Some(dir) = save {
        replay.artifact.save(dir)?;
    }
    Ok(Loaded {
        traced: frame_trace_model::TracedBlock::from_replay(&replay.capture)?,
        metadata: Some(Metadata::from_hex(&replay.artifact.metadata)?),
        block_number: Some(replay.block_number),
    })
}
