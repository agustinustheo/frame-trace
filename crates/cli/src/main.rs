//! `frame-trace`: show what a Polkadot SDK block did to storage, as OpenTelemetry traces.

use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use frame_trace_decode::{AnnotatedBlock, annotate};

use frame_trace_otel::Timing;

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
    /// Replay each new best block. Bounded queue and two replay workers.
    Follow(FollowArgs),
}

#[derive(Args)]
struct BlockArgs {
    #[command(flatten)]
    source: source::SourceArgs,
    /// OTLP/HTTP endpoint to export the trace to, for example http://127.0.0.1:4318.
    #[arg(long, value_name = "URL")]
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

#[derive(Args)]
struct FollowArgs {
    #[arg(long, value_name = "URL")]
    rpc: String,
    #[arg(long, value_name = "URL", num_args = 0..=1, default_missing_value = "http://127.0.0.1:4318")]
    otlp: Option<String>,
}

fn follow(args: FollowArgs) -> Result<()> {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    let running = Arc::new(AtomicBool::new(true));
    let signal = running.clone();
    ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))?;
    let (send, receive) = mpsc::sync_channel::<String>(8);
    let receive = Arc::new(Mutex::new(receive));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let receive = receive.clone();
            let url = args.rpc.clone();
            let otlp = args.otlp.clone();
            let running = running.clone();
            thread::spawn(move || {
                loop {
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    let hash = match receive.lock() {
                        Ok(rx) => match rx.recv() {
                            Ok(hash) => hash,
                            Err(_) => break,
                        },
                        Err(_) => break,
                    };
                    let result = (|| -> Result<()> {
                        let rpc = frame_trace_replay::rpc::Rpc::new(&url)?;
                        let loaded =
                            source::replay_loaded(frame_trace_replay::online(&rpc, &hash)?, None)?;
                        let mut block = annotate(&loaded.traced, loaded.metadata.as_ref());
                        block.block_number = loaded.block_number;
                        if let Some(endpoint) = &otlp {
                            frame_trace_otel::export_otlp(&block, endpoint, Timing::default())?;
                        }
                        eprintln!(
                            "traced best block #{} {}: {} ops",
                            loaded.block_number.unwrap_or(0),
                            hash,
                            block.totals.ops
                        );
                        Ok(())
                    })();
                    if let Err(error) = result {
                        eprintln!("skipped block {hash}: {error:#}");
                    }
                }
            })
        })
        .collect();
    // Poll the allowlisted chain_getHeader instead of using a subscription RPC outside the
    // replay allowlist. Walk new ancestry to cover blocks between polls and report best reorgs.
    let mut rpc = None;
    let mut seen = VecDeque::new();
    while running.load(Ordering::SeqCst) {
        let result = (|| -> Result<()> {
            if rpc.is_none() {
                rpc = Some(frame_trace_replay::rpc::Rpc::new(&args.rpc)?);
            }
            let client = rpc.as_ref().context("RPC connection unavailable")?;
            let mut header = client.call("chain_getHeader", serde_json::json!([]))?;
            let mut hash = frame_trace_replay::header_hash(&header)?;
            let mut pending = Vec::new();
            for _ in 0..64 {
                if seen.contains(&hash) {
                    break;
                }
                pending.push(hash.clone());
                if seen.is_empty() {
                    break;
                }
                let parent = header["parentHash"]
                    .as_str()
                    .context("parent hash missing")?
                    .to_owned();
                if seen.contains(&parent) {
                    break;
                }
                hash = parent;
                header = client.call("chain_getHeader", serde_json::json!([hash]))?;
            }
            if pending.len() == 64 {
                eprintln!("follow ancestry exceeded 64 blocks; older blocks skipped");
            }
            for hash in pending.into_iter().rev() {
                match send.try_send(hash.clone()) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_)) => {
                        eprintln!("skipped block {hash}: replay queue full")
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        anyhow::bail!("replay workers stopped")
                    }
                }
                seen.push_back(hash);
                if seen.len() > 256 {
                    seen.pop_front();
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("follow poll failed: {error:#}; reconnecting");
            rpc = None;
        }
        for _ in 0..20 {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    drop(send);
    for worker in workers {
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("replay worker panicked"))?;
    }
    Ok(())
}
