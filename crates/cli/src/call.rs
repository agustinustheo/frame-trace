//! Trace one runtime API call, using the same model/exporters as block replay.
use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::Args;
use frame_trace_decode::{AnnotatedCall, Metadata, annotate};
use frame_trace_replay::{
    Artifact, ReplaySession, call,
    rpc::{Rpc, hex0, unhex},
};

#[derive(Args)]
pub struct CallArgs {
    /// Safe node RPC endpoint.
    #[arg(long, required_unless_present = "offline", conflicts_with = "offline")]
    rpc: Option<String>,
    /// State block number, hash, best (default), or finalized.
    #[arg(long, requires = "rpc", conflicts_with = "offline")]
    at: Option<String>,
    /// Runtime API export, for example AccountNonceApi_account_nonce.
    #[arg(long, required_unless_present = "offline", conflicts_with = "offline")]
    method: Option<String>,
    /// Hex SCALE-encoded arguments (0x for no arguments).
    #[arg(long, required_unless_present_any = ["args", "offline"], conflicts_with_all = ["args", "offline"])]
    data: Option<String>,
    /// JSON positional array or named object, encoded using V15+ runtime API metadata.
    #[arg(long, conflicts_with = "offline")]
    args: Option<String>,
    /// Replay a saved call without creating an RPC client.
    #[arg(long)]
    offline: Option<PathBuf>,
    /// Save a self-contained call artifact.
    #[arg(long)]
    save: Option<PathBuf>,
    /// Compare with state_call at the identical block hash.
    #[arg(long, requires = "rpc", conflicts_with = "offline")]
    oracle: bool,
    /// Print storage summary tables.
    #[arg(long)]
    summary: bool,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long, num_args = 0..=1, default_missing_value = "http://127.0.0.1:4318")]
    otlp: Option<String>,
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=64))]
    proof_concurrency: u8,
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 1073741824)]
    cache_max_bytes: u64,
}

pub fn run(args: CallArgs) -> Result<()> {
    let session =
        ReplaySession::with_cache_max_bytes(args.cache_dir.as_deref(), args.cache_max_bytes)?;
    let (artifact, rpc) = if let Some(dir) = &args.offline {
        (Artifact::load(dir)?, None)
    } else {
        let rpc = Rpc::with_options(
            args.rpc.as_deref().context("--rpc required")?,
            args.proof_concurrency as usize,
            Duration::from_secs(30),
            Default::default(),
        )?;
        let mut artifact = call::prepare(
            &rpc,
            args.at.as_deref().unwrap_or("best"),
            args.method.as_deref().context("--method required")?,
        )?;
        let method = args.method.as_deref().context("--method required")?;
        let data = if let Some(data) = &args.data {
            unhex(data)?
        } else {
            let metadata = Metadata::from_hex(&artifact.metadata)
                .context("--args needs runtime API metadata; use --data SCALE hex")?;
            let values =
                serde_json::from_str(args.args.as_deref().context("--data or --args required")?)?;
            metadata.encode_runtime_args(method, &values)?
        };
        artifact.call.as_mut().context("missing call context")?.data = hex0(&data);
        (artifact, Some(rpc))
    };
    let replay = call::replay(artifact, rpc, &session, args.oracle)?;
    if let Some(dir) = &args.save {
        replay.artifact.save(dir)?;
    }
    let metadata = Metadata::from_hex(&replay.artifact.metadata)
        .inspect_err(|e| {
            eprintln!("warning: metadata decoding failed: {e}; using hex/unnamed storage");
        })
        .ok();
    let context = replay
        .artifact
        .call
        .as_ref()
        .context("missing call context")?;
    let decoded = metadata.as_ref().and_then(|md| {
        md.decode_runtime_result(&context.method, &replay.result)
            .inspect_err(|e| {
                eprintln!("warning: {e}; returning hex");
            })
            .ok()
    });
    let mut trace = annotate(
        &frame_trace_model::TracedBlock::from_call(&replay.capture)?,
        metadata.as_ref(),
    );
    trace.block_number = Some(replay.block_number);
    trace.call = Some(AnnotatedCall {
        method: context.method.clone(),
        data: context.data.clone(),
        result: hex0(&replay.result),
        decoded: decoded.clone(),
        oracle_match: replay.oracle_match,
        proof_bytes: replay.proof_bytes,
        state_root: replay.artifact.parent_header["stateRoot"]
            .as_str()
            .unwrap_or_default()
            .into(),
    });
    for phase in &mut trace.phases {
        phase.kind = "call";
        phase.label = context.method.clone();
    }
    eprintln!(
        "call {} at #{}: {} storage ops; result: {}",
        context.method,
        replay.block_number,
        trace.totals.ops,
        decoded.as_deref().unwrap_or(&hex0(&replay.result))
    );
    eprintln!(
        "root check: NOT APPLICABLE (runtime API call); authenticated state proofs; RPC calls: {} (measured: {}); execution proof: {} bytes; proof rounds: {}",
        replay.rpc_count, replay.measured_rpc_count, replay.proof_bytes, replay.proof_round_trips
    );
    if let Some(matched) = replay.oracle_match {
        eprintln!(
            "state_call oracle: {}{}",
            if matched { "MATCH" } else { "MISMATCH" },
            if args.offline.is_some() {
                " (recorded)"
            } else {
                ""
            }
        );
    }
    if let Some(path) = &args.json {
        super::write_json(&trace, path)?;
    }
    if let Some(endpoint) = &args.otlp {
        let spans = frame_trace_otel::export_otlp(&trace, endpoint, Default::default())?;
        eprintln!("exported {spans} spans to {endpoint}");
    }
    if args.summary {
        print!("{}", super::summary::render(&trace, 15));
    }
    Ok(())
}
