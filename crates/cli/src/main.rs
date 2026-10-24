//! `frame-trace`: show what a Polkadot SDK block did to storage, as OpenTelemetry traces.

use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use frame_trace_decode::{AnnotatedBlock, annotate};

use frame_trace_otel::Timing;

mod follow;
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
    /// Trace one block: export it over OTLP and/or write it as JSON.
    Block(BlockArgs),
    /// Print tables of storage use per phase and the busiest storage items.
    Summary(SummaryArgs),
    /// Replay new best blocks with a bounded queue and shared caches.
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
    /// Number of rows in the "top items" and "duplicate reads" tables.
    #[arg(long, default_value_t = 15)]
    top: usize,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Block(args) => block(args),
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
    if let Some(path) = &args.json {
        write_json(&block, path)?;
    }
    if let Some(endpoint) = &args.otlp {
        let spans = frame_trace_otel::export_otlp(&block, endpoint, Timing::default())
            .with_context(|| format!("exporting to {endpoint}"))?;
        eprintln!("exported {spans} spans to {endpoint}");
    }
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
    let loaded = source::load(args)?;
    let traced = loaded.traced;
    if loaded.metadata.is_none() {
        eprintln!("warning: no --metadata given, storage keys and extrinsics will not be named");
    }
    let mut block = annotate(&traced, loaded.metadata.as_ref());
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
    block.block_number = loaded.block_number;
    Ok(block)
}

fn write_json(block: &AnnotatedBlock, path: &PathBuf) -> Result<()> {
    let json = serde_json::to_string_pretty(block)?;
    if path.as_os_str() == "-" {
        writeln!(std::io::stdout(), "{json}")?;
    } else {
        fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Follow best headers at replay speed, retrying failed blocks while they are retained.
#[derive(Args)]
struct FollowArgs {
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
}

fn follow(args: FollowArgs) -> Result<()> {
    follow::run(args)
}
