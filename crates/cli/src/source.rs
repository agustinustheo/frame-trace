//! Where a block trace and its metadata come from.
//!
//! Today only recorded files are supported. To add a source, add its arguments to
//! [`SourceArgs`], a module next to [`file`] that returns a [`Loaded`], and a match arm in
//! [`load`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use frame_trace_decode::Metadata;
use frame_trace_model::rpc::BlockTrace;

/// Arguments that select the block to trace.
#[derive(Args)]
pub struct SourceArgs {
    /// Recorded `state_traceBlock` response (JSON-RPC envelope or bare result).
    #[arg(
        long,
        value_name = "TRACE.json",
        required_unless_present = "rpc",
        conflicts_with = "rpc"
    )]
    pub from_file: Option<PathBuf>,
    /// Runtime metadata of the block's runtime: SCALE bytes (`.scale`), or hex as returned by
    /// `state_getMetadata` (`.hex`, or the JSON-RPC response).
    #[arg(long, value_name = "FILE")]
    pub metadata: Option<PathBuf>,
    /// Node RPC endpoint, for example ws://127.0.0.1:9944 (not implemented yet).
    #[arg(long, value_name = "URL", requires = "block")]
    pub rpc: Option<String>,
    /// Block number or hash to trace with --rpc.
    #[arg(long, value_name = "NUMBER|HASH")]
    pub block: Option<String>,
}

/// A block trace and what is needed to decode it.
pub struct Loaded {
    pub trace: BlockTrace,
    pub metadata: Option<Metadata>,
    pub block_number: Option<u64>,
}

pub fn load(args: &SourceArgs) -> Result<Loaded> {
    match (&args.from_file, &args.rpc) {
        (Some(trace), _) => file::load(trace, args.metadata.as_deref()),
        (None, Some(url)) => rpc::load(url, args.block.as_deref().unwrap_or_default()),
        (None, None) => bail!("pass --from-file or --rpc"),
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
            trace,
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

mod rpc {
    use super::*;

    /// TODO(rpc source): fetch over JSON-RPC instead of reading files. Planned as `crates/source`
    /// (see SPEC.md): `chain_getBlockHash` for a number, `state_traceBlock(hash, "state", "", "")`,
    /// `state_getMetadata` at the parent hash, `chain_getHeader` for the block number. Return the
    /// same [`Loaded`], so nothing else changes. Keep the OTLP exporter outside any async
    /// runtime: it uses a blocking HTTP client.
    pub fn load(url: &str, block: &str) -> Result<Loaded> {
        bail!(
            "--rpc is not implemented yet (asked for block {block} from {url}); record the trace and use --from-file"
        )
    }
}
