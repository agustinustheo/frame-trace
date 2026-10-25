//! `frame-trace`: show what a Polkadot SDK block did to storage, as OpenTelemetry traces.

use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use frame_trace_decode::AnnotatedBlock;

mod call;
mod follow;
mod report;
mod source;
mod summary;

#[derive(Parser)]
#[command(name = "frame-trace", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Aggregate saved block JSON files or a finalized block range.
    Report(report::ReportArgs),
    /// Trace a runtime API call against authenticated state proofs.
    Call(call::CallArgs),
    /// Trace one block: export it over OTLP and/or write it as JSON.
    Block(BlockArgs),
    /// Print tables of storage use per phase and the busiest storage items.
    Summary(SummaryArgs),
    /// Replay new finalized blocks with a bounded queue and shared caches.
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
        f.write_str("state_call oracle MISMATCH")
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
            "block {}: {} phases, {} storage ops ({} reads, {} writes, {} duplicate reads). \
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
    let mut block = frame_trace_decode::annotate_at_block(
        &traced,
        loaded.metadata.as_ref(),
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
        let spans = frame_trace_otel::export_otlp(trace, endpoint, Default::default())
            .with_context(|| format!("exporting to {endpoint}"))?;
        eprintln!("exported {spans} spans to {endpoint}");
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
    /// Maximum concurrent proof requests; each request batches up to 32 keys.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(1..=64))]
    proof_concurrency: u8,
    /// Persistent proof-node and compiled Wasm cache directory.
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Combined disk cache limit in bytes; active runtime modules remain pinned.
    #[arg(long, default_value_t = 1073741824)]
    cache_max_bytes: u64,
    /// Allow UNAUTHENTICATED metadata: RPC online, stored artifact metadata offline.
    #[arg(long = "unauthenticated-metadata", alias = "rpc-metadata-fallback")]
    rpc_metadata_fallback: bool,
}

fn follow(args: FollowArgs) -> Result<()> {
    follow::run(args)
}
