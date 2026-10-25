//! Trace one runtime API call, using the same model/exporters as block replay.
use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::{ArgGroup, Args};
use frame_trace_decode::{AnnotatedCall, Metadata, annotate};
use frame_trace_replay::{
    Artifact, ReplaySession, call,
    rpc::{Rpc, hex0, unhex},
};

#[derive(Args)]
#[command(
    group(ArgGroup::new("source").required(true).args(["rpc", "offline"])),
    group(ArgGroup::new("call_input").args(["data", "args"]))
)]
pub struct CallArgs {
    /// Safe node RPC endpoint.
    #[arg(long, value_name = "URL", requires_all = ["method", "call_input"])]
    rpc: Option<String>,
    /// State block number, hash, best (default), or finalized.
    #[arg(
        long,
        value_name = "NUMBER|HASH|best|finalized",
        requires = "rpc",
        conflicts_with = "offline"
    )]
    at: Option<String>,
    /// Runtime API export, for example AccountNonceApi_account_nonce.
    #[arg(long, conflicts_with = "offline")]
    method: Option<String>,
    /// Hex SCALE-encoded arguments (0x for no arguments).
    #[arg(long, conflicts_with_all = ["args", "offline"])]
    data: Option<String>,
    /// JSON positional array or named object, encoded using V15+ runtime API metadata.
    #[arg(long, conflicts_with = "offline")]
    args: Option<String>,
    /// Replay a saved call with no network.
    #[arg(long, value_name = "DIR")]
    offline: Option<PathBuf>,
    /// Save a self-contained call artifact.
    #[arg(long)]
    save: Option<PathBuf>,
    /// Compare the result with the node's state_call at the same block hash.
    #[arg(
        long = "check-state-call",
        alias = "oracle",
        requires = "rpc",
        conflicts_with = "offline"
    )]
    oracle: bool,
    /// Print storage summary tables.
    #[arg(long)]
    summary: bool,
    /// Write the decoded trace as JSON to this file (`-` for stdout).
    #[arg(long, value_name = "FILE")]
    json: Option<PathBuf>,
    /// OTLP/HTTP endpoint (bare --otlp uses http://127.0.0.1:4318).
    #[arg(long, num_args = 0..=1, default_missing_value = "http://127.0.0.1:4318")]
    otlp: Option<String>,
    /// Maximum concurrent proof requests (1–64; up to 32 keys per request).
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=64))]
    proof_concurrency: u8,
    /// Persistent authenticated proof-node and compiled Wasm cache directory.
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Disk cache limit in bytes; runtime code in use is kept.
    #[arg(long, default_value_t = 1073741824)]
    cache_max_bytes: u64,
    /// Allow unverified metadata from the node or saved replay.
    #[arg(long = "unauthenticated-metadata", alias = "rpc-metadata-fallback")]
    rpc_metadata_fallback: bool,
    /// Execute this Wasm instead of the authenticated :code, for example the same runtime
    /// built with `with-tracing`. Bypasses the code-hash check. Labelled on stderr, in JSON
    /// and in OTLP. Warns when its runtime version differs from the chain's. Cannot be
    /// combined with --save.
    #[arg(long, value_name = "WASM", conflicts_with = "save")]
    runtime_override: Option<PathBuf>,
}

pub fn run(args: CallArgs) -> Result<()> {
    // Reject malformed user input before any network or runtime work.
    let raw_data = args
        .data
        .as_deref()
        .map(|s| unhex(s).context("--data: invalid SCALE hex"))
        .transpose()?;
    let json_args = args
        .args
        .as_deref()
        .map(|s| serde_json::from_str::<serde_json::Value>(s).context("--args: malformed JSON"))
        .transpose()?;
    let runtime_override = super::source::read_runtime_override(args.runtime_override.as_deref())?;
    let session =
        ReplaySession::with_cache_max_bytes(args.cache_dir.as_deref(), args.cache_max_bytes)?
            .with_rpc_metadata_fallback(args.rpc_metadata_fallback)
            .with_runtime_override(runtime_override);
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
        call::authenticate_metadata(&mut artifact, Some(rpc.clone()), &session)?;
        if let Ok(metadata) = Metadata::from_hex(&artifact.metadata) {
            metadata.validate_runtime_method(method)?;
        }
        let data = if let Some(data) = raw_data {
            data
        } else {
            anyhow::ensure!(
                !artifact.metadata.is_empty(),
                "--args needs locally executed runtime API metadata; use --data SCALE hex"
            );
            let metadata = Metadata::from_hex(&artifact.metadata)
                .context("--args needs runtime API metadata; use --data SCALE hex")?;
            metadata
                .encode_runtime_args(
                    method,
                    json_args.as_ref().context("--data or --args required")?,
                )
                .context("--args: cannot encode runtime API arguments")?
        };
        artifact.call.as_mut().context("missing call context")?.data = hex0(&data);
        (artifact, Some(rpc))
    };
    let replay = call::replay(artifact, rpc, &session, args.oracle)?;
    if let Some(dir) = &args.save {
        replay.artifact.save(dir)?;
    }
    let metadata = if replay.artifact.metadata.is_empty() {
        None
    } else {
        Metadata::from_hex(&replay.artifact.metadata)
            .inspect_err(|e| {
                eprintln!("warning: metadata decoding failed: {e}; using hex/unnamed storage");
            })
            .ok()
    };
    let context = replay
        .artifact
        .call
        .as_ref()
        .context("missing call context")?;
    let decoded = metadata.as_ref().filter(|md| {
        if md.version() < 15 { eprintln!("notice: V14 metadata has no runtime API types; displaying hex (use --data for arguments)"); false }
        else { true }
    }).and_then(|md| {
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
    trace.runtime_override =
        super::source::override_info(replay.runtime_override, args.runtime_override.as_deref());
    trace.call = Some(AnnotatedCall {
        method: context.method.clone(),
        data: context.data.clone(),
        result: hex0(&replay.result),
        decoded: decoded.clone(),
        oracle_match: replay.oracle_match,
        oracle_source: replay.oracle_match.map(|_| {
            if args.offline.is_some() {
                "recorded (unverified)".into()
            } else {
                "live".into()
            }
        }),
        proof_bytes: replay.proof_bytes,
        state_root: replay.artifact.parent_header["stateRoot"]
            .as_str()
            .unwrap_or_default()
            .into(),
    });
    for phase in &mut trace.phases {
        phase.kind = "call".into();
        phase.label = context.method.clone();
    }
    eprintln!(
        "call {} at #{}: {} storage operations; result: {}",
        context.method,
        replay.block_number,
        trace.totals.ops,
        preview(decoded.as_deref().unwrap_or(&hex0(&replay.result)))
    );
    eprintln!(
        "root check: NOT APPLICABLE (runtime API call); authenticated state proofs; RPC calls: {} (timed replay: {}); execution proof: {} bytes; proof request rounds: {}",
        replay.rpc_count, replay.measured_rpc_count, replay.proof_bytes, replay.proof_round_trips
    );
    if let Some(matched) = replay.oracle_match {
        eprintln!(
            "state_call check: {}{}",
            if matched { "MATCH" } else { "MISMATCH" },
            if args.offline.is_some() {
                " (recorded; unverified)"
            } else {
                ""
            }
        );
    }
    super::emit(
        &trace,
        args.json.as_deref(),
        args.otlp.as_deref(),
        args.summary.then_some(15),
    )?;
    if replay.oracle_match == Some(false) {
        return Err(super::OracleMismatch.into());
    }
    Ok(())
}

fn preview(value: &str) -> String {
    match value.char_indices().nth(256) {
        Some((cut, _)) => format!(
            "{}… ({} chars; full value in --json)",
            &value[..cut],
            value.chars().count()
        ),
        None => value.to_owned(),
    }
}
