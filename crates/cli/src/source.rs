//! Where a block trace and its metadata come from.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Args};
use polkadot_frame_trace_decode::Metadata;

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
    #[arg(long, value_name = "FILE", requires = "from_file", conflicts_with_all = ["rpc", "offline", "from_json"])]
    pub metadata: Option<PathBuf>,
    /// Node RPC endpoint, for example wss://westend-asset-hub-rpc.polkadot.io.
    #[arg(
        long,
        value_name = "URL",
        requires = "block",
        conflicts_with = "offline"
    )]
    pub rpc: Option<String>,
    /// Block number, hash, `best` or `finalized` to trace with --rpc.
    #[arg(long, value_name = "NUMBER|HASH|best|finalized", requires = "rpc", conflicts_with_all = ["from_file", "from_json", "offline"])]
    pub block: Option<String>,
    /// Replay a saved artifact with no network.
    #[arg(long, value_name = "DIR", conflicts_with = "block")]
    pub offline: Option<PathBuf>,
    /// Save the replay context and proofs for offline use.
    #[arg(long, value_name = "DIR", conflicts_with = "from_file")]
    pub save: Option<PathBuf>,
    /// Maximum concurrent RPC requests (1–64; up to 32 proof keys per request).
    #[arg(long, default_value_t = 8, requires = "rpc", conflicts_with_all = ["from_file", "from_json", "offline"], value_parser = clap::value_parser!(u8).range(1..=64))]
    pub proof_concurrency: u8,
    /// Longest wait in seconds for one RPC response; each request gets four tries.
    #[arg(long, value_name = "SECONDS", default_value = "30", value_parser = rpc_timeout, requires = "rpc", conflicts_with_all = ["from_file", "from_json", "offline"])]
    pub rpc_timeout: Duration,
    /// Persistent authenticated proof nodes and compiled Wasm cache directory.
    #[arg(long, value_name = "DIR", conflicts_with_all = ["from_file", "from_json"])]
    pub cache_dir: Option<PathBuf>,
    /// Disk cache limit in bytes; runtime code in use is kept.
    #[arg(long, default_value_t = 1073741824, conflicts_with_all = ["from_file", "from_json"])]
    pub cache_max_bytes: u64,
    /// Allow unverified metadata from the node or saved replay.
    #[arg(long = "unauthenticated-metadata", alias = "rpc-metadata-fallback", conflicts_with_all = ["from_file", "from_json"])]
    pub rpc_metadata_fallback: bool,
    /// Execute this Wasm instead of the authenticated :code, for example the same runtime
    /// built with `with-tracing`. Bypasses the code-hash check. Labelled on stderr, in JSON
    /// and in OTLP. Warns when its runtime version differs from the chain's. Cannot be
    /// combined with --save.
    #[arg(long, value_name = "WASM", conflicts_with_all = ["from_file", "from_json", "save"])]
    pub runtime_override: Option<PathBuf>,
}

/// Parses `--rpc-timeout`, in seconds.
pub fn rpc_timeout(text: &str) -> std::result::Result<Duration, String> {
    let secs: f64 = text
        .parse()
        .map_err(|_| format!("{text} is not a number of seconds"))?;
    if !(secs > 0.0 && secs <= 3600.0) {
        return Err("must be more than 0 and at most 3600 seconds".into());
    }
    Ok(Duration::from_secs_f64(secs))
}

/// Longest a replay should keep running the runtime. `follow` exits 1 when one does; the
/// other commands warn once. Real blocks take seconds, and compiling a new runtime a minute.
pub const REPLAY_DEADLINE: Duration = Duration::from_secs(600);

/// Warns once if still alive after [`REPLAY_DEADLINE`]. The runtime cannot be interrupted, so
/// the replay goes on until it ends or the user stops it.
pub struct SlowWarning {
    _done: std::sync::mpsc::Sender<()>,
}
pub fn warn_if_slow(what: String) -> SlowWarning {
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let _ = std::thread::Builder::new()
        .name("replay-deadline".into())
        .spawn(move || {
            if wait.recv_timeout(REPLAY_DEADLINE) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                say!(
                    "warning: {what} has run for more than 10 minutes; the runtime may never finish. Press Ctrl-C to stop it"
                );
            }
        });
    SlowWarning { _done: done }
}

/// Warns once when `url` is plain `ws://` or `http://` to a host other than this machine:
/// anyone on the way can read, or change, what is sent.
pub fn warn_unencrypted(flag: &str, url: &str) {
    let shown = polkadot_frame_trace_model::redact::endpoint(url);
    let Some(authority) = shown
        .strip_prefix("ws://")
        .or_else(|| shown.strip_prefix("http://"))
    else {
        return;
    };
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    let local = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !local {
        let secure = if flag == "--otlp" {
            "https://"
        } else {
            "wss:// or https://"
        };
        say!("warning: {flag} {shown} is not encrypted; use {secure} for a remote host");
    }
}

/// A node client. `timeout` bounds each try; a request gets four tries within four times it.
pub fn rpc(
    url: &str,
    concurrency: u8,
    timeout: Duration,
    cancel: polkadot_frame_trace_replay::rpc::CancellationToken,
) -> Result<polkadot_frame_trace_replay::rpc::Rpc> {
    warn_unencrypted("--rpc", url);
    Ok(polkadot_frame_trace_replay::rpc::Rpc::with_options(
        url,
        usize::from(concurrency),
        timeout * 4,
        cancel,
    )?)
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
    polkadot_frame_trace_replay::check_runtime_wasm(&wasm)
        .with_context(|| format!("--runtime-override {}", path.display()))?;
    say!(
        "RUNTIME OVERRIDE: {} will execute instead of the chain's authenticated :code",
        path.display()
    );
    Ok(Some(wasm))
}

/// The JSON/OTLP label for a substitute runtime.
pub fn override_info(
    o: Option<polkadot_frame_trace_replay::RuntimeOverride>,
    path: Option<&Path>,
) -> Option<polkadot_frame_trace_decode::RuntimeOverrideInfo> {
    o.map(|o| polkadot_frame_trace_decode::RuntimeOverrideInfo {
        path: path.map(|p| p.display().to_string()).unwrap_or_default(),
        code_hash: format!("{:?}", o.override_code_hash),
        chain_code_hash: format!("{:?}", o.chain_code_hash),
        version_differences: o.version_differences,
    })
}

/// A block trace and what is needed to decode it.
pub struct Loaded {
    pub traced: polkadot_frame_trace_model::TracedBlock,
    pub metadata: Option<Arc<Metadata>>,
    pub block_number: Option<u64>,
    pub execution: Option<polkadot_frame_trace_decode::ExecutionInfo>,
    pub runtime_override: Option<polkadot_frame_trace_replay::RuntimeOverride>,
}

pub fn load(args: &SourceArgs) -> Result<Loaded> {
    let _slow = warn_if_slow(match &args.block {
        Some(block) => format!("the replay of block {block}"),
        None => "the replay".into(),
    });
    let runtime_override = read_runtime_override(args.runtime_override.as_deref())?;
    if let Some(dir) = &args.offline {
        let replay = polkadot_frame_trace_replay::Artifact::load(dir)
            .and_then(|artifact| {
                polkadot_frame_trace_replay::replay_with_session(
                    artifact,
                    None,
                    &polkadot_frame_trace_replay::ReplaySession::with_cache_max_bytes(
                        args.cache_dir.as_deref(),
                        args.cache_max_bytes,
                    )?
                    .with_rpc_metadata_fallback(args.rpc_metadata_fallback)
                    .with_runtime_override(runtime_override),
                )
            })
            .map_err(|e| anyhow::anyhow!("{}: {e}", dir.join("artifact.json.zst").display()))
            .inspect_err(|_| {
                say!("root check: NOT VERIFIED; RPC calls: 0");
            })?;
        return replay_loaded(replay, args.save.as_deref());
    }
    match (&args.from_file, &args.rpc) {
        (Some(trace), _) => file::load(trace, args.metadata.as_deref()),
        (None, Some(url)) => {
            let rpc = rpc(
                url,
                args.proof_concurrency,
                args.rpc_timeout,
                Default::default(),
            )
            .inspect_err(|_| {
                say!("root check: NOT VERIFIED; RPC calls: 0");
            })?;
            let replay = polkadot_frame_trace_replay::online_with_session(
                &rpc,
                args.block.as_deref().unwrap_or_default(),
                &polkadot_frame_trace_replay::ReplaySession::with_cache_max_bytes(
                    args.cache_dir.as_deref(),
                    args.cache_max_bytes,
                )?
                .with_rpc_metadata_fallback(args.rpc_metadata_fallback)
                .with_runtime_override(runtime_override),
            )
            .inspect_err(|_| {
                say!("root check: NOT VERIFIED; RPC calls: {}", rpc.count());
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
        let trace = polkadot_frame_trace_model::rpc::parse_trace_block(&json)
            .with_context(|| format!("parsing {}", trace.display()))?;
        let metadata = metadata.map(load_metadata).transpose()?.map(Arc::new);
        Ok(Loaded {
            traced: polkadot_frame_trace_model::TracedBlock::from_trace(&trace)
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
/// The decoded metadata of the last few runtimes. `follow` and `report` replay many blocks of
/// one runtime: they decode its metadata once, not once per block.
fn decoded(hex: &str) -> std::result::Result<Arc<Metadata>, polkadot_frame_trace_decode::Error> {
    static DECODED: Mutex<Vec<(String, Arc<Metadata>)>> = Mutex::new(Vec::new());
    let found = DECODED
        .lock()
        .ok()
        .and_then(|d| d.iter().find(|(h, _)| h == hex).map(|(_, m)| m.clone()));
    if let Some(metadata) = found {
        return Ok(metadata);
    }
    let metadata = Arc::new(Metadata::from_hex(hex)?);
    if let Ok(mut decoded) = DECODED.lock() {
        if decoded.len() >= 4 {
            decoded.remove(0);
        }
        decoded.push((hex.to_owned(), metadata.clone()));
    }
    Ok(metadata)
}

pub fn replay_loaded(
    replay: polkadot_frame_trace_replay::Replay,
    save: Option<&Path>,
) -> Result<Loaded> {
    say!(
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
    say!(
        "fetching state #{}: {} proof request rounds; {} RPC calls",
        replay.block_number,
        replay.proof_round_trips,
        replay.rpc_count
    );
    say!(
        "state root: computed {:?}; header {:?}",
        replay.computed_state_root,
        replay.expected_state_root
    );
    let metadata = if replay.artifact.metadata.is_empty() {
        None
    } else {
        match decoded(&replay.artifact.metadata) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                say!(
                    "warning: metadata decoding failed: {error}; continuing with undecoded names (names require metadata V14+)"
                );
                None
            }
        }
    };
    Ok(Loaded {
        traced: polkadot_frame_trace_model::TracedBlock::from_replay(&replay.capture)?,
        metadata,
        block_number: Some(replay.block_number),
        execution: Some(polkadot_frame_trace_decode::ExecutionInfo {
            root_check: replay.root_check,
            proof_bytes: replay.proof_bytes,
            rpc_calls: replay.rpc_count,
            measured_rpc_calls: replay.measured_rpc_count,
            proof_rounds: replay.proof_round_trips,
        }),
        runtime_override: replay.runtime_override,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_is_decoded_once_per_runtime() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/metadata/westend-v16.scale");
        let bytes = std::fs::read(path).expect("fixture");
        let hex: String = std::iter::once("0x".to_owned())
            .chain(bytes.iter().map(|b| format!("{b:02x}")))
            .collect();
        let first = decoded(&hex).expect("decodes");
        let second = decoded(&hex).expect("decodes");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(decoded("0x00").is_err());
        // Failures are not kept: the same text fails again, and the good entry stays.
        assert!(decoded("0x00").is_err());
        assert!(Arc::ptr_eq(&first, &decoded(&hex).expect("decodes")));
    }
}
