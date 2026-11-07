//! `polkadot-frame-trace`: show what a Polkadot SDK block did to storage, as OpenTelemetry traces.

use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use polkadot_frame_trace_decode::AnnotatedBlock;

#[macro_use]
mod log;
mod call;
mod follow;
mod report;
mod source;
mod summary;

/// The version `--version` prints. The nightly workflow sets `FRAME_TRACE_BUILD_VERSION`
/// at build time, for example `0.1.0-nightly.20261106+abc1234`; other builds print the crate version.
const VERSION: &str = build_version(option_env!("FRAME_TRACE_BUILD_VERSION"));

const fn build_version(set: Option<&'static str>) -> &'static str {
    match set {
        Some(version) if !version.is_empty() => version,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Parser)]
#[command(name = "polkadot-frame-trace", version = VERSION, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compare storage use across saved blocks or a range of finalized blocks.
    #[command(
        after_help = "Examples:\n  polkadot-frame-trace report block-1.json block-2.json\n  polkadot-frame-trace report --rpc URL --last 10"
    )]
    Report(report::ReportArgs),
    /// Trace storage use and the result of a runtime API call.
    #[command(
        after_help = "Examples:\n  polkadot-frame-trace call --rpc URL --at finalized --method Core_version --data 0x --check-state-call\n  polkadot-frame-trace call --offline fixtures/replay/call-people-nonce --summary"
    )]
    Call(call::CallArgs),
    /// Trace storage use in one block and export it to JSON or a trace viewer.
    #[command(
        after_help = "Examples:\n  polkadot-frame-trace block --rpc URL --block finalized --otlp\n  polkadot-frame-trace block --offline fixtures/replay/people-traced --json block.json"
    )]
    Block(BlockArgs),
    /// Show storage use by block phase and storage item.
    #[command(
        after_help = "Examples:\n  polkadot-frame-trace summary --rpc URL --block finalized\n  polkadot-frame-trace summary --from-json block.json"
    )]
    Summary(SummaryArgs),
    /// Trace new finalized blocks as they arrive.
    #[command(
        after_help = "Examples:\n  polkadot-frame-trace follow --rpc URL --otlp\n  polkadot-frame-trace follow --rpc URL --best"
    )]
    Follow(FollowArgs),
}

#[derive(Args)]
struct BlockArgs {
    #[command(flatten)]
    source: source::SourceArgs,
    /// OTLP/HTTP endpoint (bare --otlp uses http://127.0.0.1:4318).
    #[arg(long, value_name = "URL", num_args = 0..=1, default_missing_value = "http://127.0.0.1:4318")]
    otlp: Option<String>,
    /// Write the decoded trace as JSON to this file (`-` for stdout).
    #[arg(long, value_name = "FILE")]
    json: Option<PathBuf>,
}

#[derive(Args)]
struct SummaryArgs {
    #[command(flatten)]
    source: source::SourceArgs,
    /// Maximum rows in each item, extrinsic, hook and per-phase duplicate table.
    #[arg(long, default_value_t = 15)]
    top: usize,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) if error.downcast_ref::<follow::Reported>().is_some() => {
            std::process::ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("Error: {error:#}");
            std::process::ExitCode::from(if error.downcast_ref::<OracleMismatch>().is_some() {
                3
            } else {
                1
            })
        }
    }
}

#[derive(Debug)]
struct OracleMismatch;
impl std::fmt::Display for OracleMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("state_call check: MISMATCH")
    }
}
impl std::error::Error for OracleMismatch {}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Report(args) => report::run(args),
        Command::Block(args) => block(args),
        Command::Call(args) => call::run(args),
        Command::Follow(args) => follow(args),
        Command::Summary(args) => {
            let block = load(&args.source)?;
            print!("{}", summary::render(&block, args.top));
            Ok(())
        }
    }
}

fn block(args: BlockArgs) -> Result<()> {
    let block = load(&args.source)?;
    emit(&block, args.json.as_deref(), args.otlp.as_deref(), None)?;
    if args.json.is_none() && args.otlp.is_none() {
        let t = &block.totals;
        println!(
            "block {}: {} phases, {} storage operations ({} reads, {} writes, {} duplicate reads). \
             Use --otlp or --json to export it.",
            block.block_hash,
            block.phases.len(),
            t.ops,
            t.reads,
            t.writes,
            t.duplicate_reads
        );
    }
    Ok(())
}

fn load(args: &source::SourceArgs) -> Result<AnnotatedBlock> {
    if let Some(path) = &args.from_json {
        return report::read_trace(path);
    }
    let loaded = source::load(args)?;
    let traced = loaded.traced;
    if loaded.metadata.is_none() {
        eprintln!("notice: metadata unavailable; storage keys and extrinsics will not be named");
    }
    let mut block = polkadot_frame_trace_decode::annotate_at_block(
        &traced,
        loaded.metadata.as_deref(),
        loaded.block_number,
    );
    let undecoded = block
        .phases
        .iter()
        .filter(|p| p.call_error.is_some())
        .count();
    if undecoded > 0 {
        eprintln!(
            "warning: {undecoded} extrinsic(s) did not decode with this metadata; \
             is it the metadata of the block's runtime?"
        );
    }
    block.execution = loaded.execution;
    block.runtime_override =
        source::override_info(loaded.runtime_override, args.runtime_override.as_deref());
    Ok(block)
}

fn write_json(block: &AnnotatedBlock, path: &std::path::Path) -> Result<()> {
    let json = serde_json::to_string_pretty(block)?;
    if path.as_os_str() == "-" {
        writeln!(std::io::stdout(), "{json}")?;
    } else {
        fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Shared JSON/OTLP/terminal output for call and block traces.
fn emit(
    trace: &AnnotatedBlock,
    json: Option<&std::path::Path>,
    otlp: Option<&str>,
    top: Option<usize>,
) -> Result<()> {
    if let Some(path) = json {
        write_json(trace, path)?;
    }
    if let Some(endpoint) = otlp {
        let shown = polkadot_frame_trace_model::redact::endpoint(endpoint);
        let spans = polkadot_frame_trace_otel::export_otlp(trace, endpoint, Default::default())
            .with_context(|| format!("exporting to {shown}"))?;
        eprintln!("exported {spans} spans to {shown}");
    }
    if let Some(top) = top {
        let summary = summary::render(trace, top);
        if json.is_some_and(|p| p.as_os_str() == "-") {
            eprint!("{summary}");
        } else {
            print!("{summary}");
        }
    }
    Ok(())
}

/// Follow finalized headers at replay speed; opt into reversible best heads with --best.
#[derive(Args)]
struct FollowArgs {
    /// Follow best heads instead of finalized heads; report reorgs and reverted traces.
    #[arg(long)]
    best: bool,
    /// Node HTTP(S) or WS(S) safe RPC endpoint.
    #[arg(long, value_name = "URL")]
    rpc: String,
    /// OTLP/HTTP endpoint (bare --otlp uses http://127.0.0.1:4318).
    #[arg(long, value_name = "URL", num_args = 0..=1, default_missing_value = "http://127.0.0.1:4318")]
    otlp: Option<String>,
    /// Blocks replayed at once (1–16).
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=16))]
    workers: u8,
    /// Maximum concurrent RPC requests across all workers (1–64; up to 32 proof keys per request).
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u8).range(1..=64))]
    proof_concurrency: u8,
    /// Longest wait in seconds for one RPC response; each request gets four tries.
    #[arg(long, value_name = "SECONDS", default_value = "30", value_parser = source::rpc_timeout)]
    rpc_timeout: std::time::Duration,
    /// Persistent proof-node and compiled Wasm cache directory.
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Disk cache limit in bytes; runtime code in use is kept.
    #[arg(long, default_value_t = 1073741824)]
    cache_max_bytes: u64,
    /// Allow unverified metadata from the node or saved replay.
    #[arg(long = "unauthenticated-metadata", alias = "rpc-metadata-fallback")]
    rpc_metadata_fallback: bool,
}

fn follow(args: FollowArgs) -> Result<()> {
    follow::run(args)
}

#[cfg(test)]
mod argument_tests {
    use super::*;

    #[test]
    fn version_is_the_crate_version_unless_the_build_sets_one() {
        assert_eq!(build_version(None), env!("CARGO_PKG_VERSION"));
        assert_eq!(build_version(Some("")), env!("CARGO_PKG_VERSION"));
        assert_eq!(
            build_version(Some("0.1.0-nightly.20261106+abc1234")),
            "0.1.0-nightly.20261106+abc1234"
        );
        let error = Cli::try_parse_from(["polkadot-frame-trace", "--version"])
            .err()
            .expect("--version exits early");
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
        assert_eq!(
            error.to_string(),
            format!("polkadot-frame-trace {VERSION}\n")
        );
    }

    #[test]
    fn follow_workers_default_and_limits() {
        let Command::Follow(args) =
            Cli::try_parse_from(["polkadot-frame-trace", "follow", "--rpc", "ws://node"])
                .expect("default follow arguments")
                .command
        else {
            panic!("follow command");
        };
        assert_eq!(args.workers, 8);
        assert_eq!(args.proof_concurrency, 16);
        assert_eq!(args.rpc_timeout, std::time::Duration::from_secs(30));
        for workers in ["1", "4", "16"] {
            assert!(
                Cli::try_parse_from([
                    "polkadot-frame-trace",
                    "follow",
                    "--rpc",
                    "ws://node",
                    "--workers",
                    workers
                ])
                .is_ok()
            );
        }
        for workers in ["0", "17", "256", "-1", "many"] {
            assert!(
                Cli::try_parse_from([
                    "polkadot-frame-trace",
                    "follow",
                    "--rpc",
                    "ws://node",
                    "--workers",
                    workers
                ])
                .is_err()
            );
        }
    }
}
