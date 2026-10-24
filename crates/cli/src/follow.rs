//! Bounded follow scheduling. Only completed replays become done; failures remain retryable.
use crate::{FollowArgs, Timing, source};
use anyhow::{Context, Result};
use frame_trace_decode::annotate;
use frame_trace_replay::{
    ReplaySession,
    rpc::{CancellationToken, Rpc},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Queued,
    InFlight,
    Done,
    Failed,
}
struct Entry {
    state: State,
    attempts: u32,
    retry_at: Instant,
}
enum Event {
    Started(String),
    Finished(String, bool),
}

pub fn run(args: FollowArgs) -> Result<()> {
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    ctrlc::set_handler(move || {
        if signal.is_cancelled() {
            std::process::exit(130);
        }
        signal.cancel();
    })?;
    let rpc = Rpc::with_options(
        &args.rpc,
        args.proof_concurrency as usize,
        Duration::from_secs(30),
        cancel.clone(),
    )?;
    let session = ReplaySession::new(args.cache_dir.as_deref())?;
    let (send, receive) = mpsc::sync_channel::<String>(8);
    let (events, completed) = mpsc::channel();
    let receive = Arc::new(Mutex::new(receive));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let receive = receive.clone();
            let rpc = rpc.fork();
            let session = session.clone();
            let otlp = args.otlp.clone();
            let events = events.clone();
            thread::spawn(move || {
                while !rpc.cancelled() {
                    let hash = match receive.lock() {
                        Ok(rx) => match rx.recv_timeout(Duration::from_millis(100)) {
                            Ok(hash) => hash,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(_) => break,
                        },
                        Err(_) => break,
                    };
                    if rpc.cancelled() {
                        break;
                    }
                    let _ = events.send(Event::Started(hash.clone()));
                    let start = Instant::now();
                    let result = (|| -> Result<()> {
                        let loaded = source::replay_loaded(
                            frame_trace_replay::online_with_session(&rpc, &hash, &session)?,
                            None,
                        )?;
                        let mut block = annotate(&loaded.traced, loaded.metadata.as_ref());
                        block.block_number = loaded.block_number;
                        if let Some(endpoint) = &otlp {
                            frame_trace_otel::export_otlp(&block, endpoint, Timing::default())?;
                        }
                        eprintln!(
                            "traced best block #{} {hash}: {} ops in {:.3}s",
                            loaded.block_number.unwrap_or(0),
                            block.totals.ops,
                            start.elapsed().as_secs_f64()
                        );
                        Ok(())
                    })();
                    if let Err(error) = &result {
                        eprintln!("failed block {hash}: {error:#}; will retry");
                    }
                    let _ = events.send(Event::Finished(hash, result.is_ok()));
                }
            })
        })
        .collect();
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut order = VecDeque::new();
    while !cancel.is_cancelled() {
        for event in completed.try_iter() {
            match event {
                Event::Started(hash) => {
                    if let Some(entry) = entries.get_mut(&hash) {
                        entry.state = State::InFlight;
                    }
                }
                Event::Finished(hash, passed) => {
                    if let Some(entry) = entries.get_mut(&hash) {
                        entry.state = if passed { State::Done } else { State::Failed };
                        entry.attempts += 1;
                        entry.retry_at = Instant::now()
                            + Duration::from_millis(
                                (500u64 << entry.attempts.min(6)).min(30_000)
                                    + (entry.attempts as u64 * 71 % 251),
                            );
                    }
                }
            }
        }
        for (hash, entry) in &mut entries {
            if entry.state == State::Failed
                && entry.retry_at <= Instant::now()
                && send.try_send(hash.clone()).is_ok()
            {
                entry.state = State::Queued;
                eprintln!("retrying block {hash}");
            }
        }
        let result = (|| -> Result<()> {
            let mut header = rpc.call("chain_getHeader", serde_json::json!([]))?;
            let mut hash = frame_trace_replay::header_hash(&header)?;
            let mut pending = Vec::new();
            for _ in 0..64 {
                if entries.contains_key(&hash) {
                    break;
                }
                pending.push(hash.clone());
                if entries.is_empty() {
                    break;
                }
                hash = header["parentHash"]
                    .as_str()
                    .context("parent hash missing")?
                    .to_owned();
                if entries.contains_key(&hash) {
                    break;
                }
                header = rpc.call("chain_getHeader", serde_json::json!([hash]))?;
            }
            if pending.len() == 64 {
                eprintln!("follow ancestry exceeded 64 blocks; sampling at replay speed");
            }
            for hash in pending.into_iter().rev() {
                match send.try_send(hash.clone()) {
                    Ok(()) => {
                        order.push_back(hash.clone());
                        entries.insert(
                            hash,
                            Entry {
                                state: State::Queued,
                                attempts: 0,
                                retry_at: Instant::now(),
                            },
                        );
                    }
                    Err(mpsc::TrySendError::Full(_)) => {
                        eprintln!(
                            "follow queue full; sampling at replay speed; unqueued blocks remain eligible"
                        );
                        break;
                    }
                    Err(_) => anyhow::bail!("replay workers stopped"),
                }
            }
            while entries.len() > 256 {
                let Some(pos) = order.iter().position(|h| {
                    entries
                        .get(h)
                        .is_some_and(|e| matches!(e.state, State::Done | State::Failed))
                }) else {
                    break;
                };
                if let Some(hash) = order.remove(pos)
                    && entries
                        .remove(&hash)
                        .is_some_and(|e| e.state == State::Failed)
                {
                    eprintln!("failed block {hash} expired from 256-block retry history");
                }
            }
            Ok(())
        })();
        if let Err(error) = result
            && !cancel.is_cancelled()
        {
            eprintln!("follow poll failed: {error:#}; retrying");
        }
        let counts = [State::Queued, State::InFlight, State::Done, State::Failed]
            .map(|state| entries.values().filter(|e| e.state == state).count());
        eprintln!(
            "follow: queued={} in-flight={} done={} failed={}",
            counts[0], counts[1], counts[2], counts[3]
        );
        for _ in 0..20 {
            if cancel.is_cancelled() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    // Cancellation drops outstanding async I/O. Do not wait for non-cancellable SDK compilation
    // or collector I/O: process shutdown terminates those worker threads. A second Ctrl-C exits.
    drop(send);
    for worker in workers {
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
    eprintln!("follow stopped");
    Ok(())
}
