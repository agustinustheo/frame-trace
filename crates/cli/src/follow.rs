//! Bounded, finality-aware replay sampling. Every accepted job reaches a terminal event.
use crate::{FollowArgs, source};
use anyhow::{Context, Result};
use frame_trace_otel::Timing;
use frame_trace_replay::{
    ReplaySession,
    rpc::{CancellationToken, Rpc},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const QUEUE: usize = 8;
const MAX_ATTEMPTS: u32 = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Queued,
    InFlight,
    Done,
    Failed,
    Terminal,
    Reverted,
    Skipped,
    Cancelled,
}
struct Entry {
    number: u64,
    state: State,
    attempts: u32,
    retry_at: Instant,
    reverted: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Job {
    hash: String,
    number: u64,
    reverted: Arc<AtomicBool>,
}
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Retryable,
    Terminal,
    Skipped,
    Cancelled,
}
enum Event {
    Started(String, Arc<AtomicBool>),
    Finished(String, Arc<AtomicBool>, Outcome),
}

#[derive(Debug)]
struct Skipped(&'static str);
impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Skipped {}

fn current_entry<'a>(
    entries: &'a mut BTreeMap<String, Entry>,
    hash: &str,
    generation: &Arc<AtomicBool>,
) -> Option<&'a mut Entry> {
    entries
        .get_mut(hash)
        .filter(|entry| Arc::ptr_eq(&entry.reverted, generation))
}

// The catch covers acquisition, conversion, annotation and export, not just the SDK executor.
fn job_outcome(run: impl FnOnce() -> Result<()>) -> Outcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(Ok(())) => Outcome::Passed,
        Ok(Err(error)) => {
            if error.downcast_ref::<Skipped>().is_some() {
                eprintln!("replay job skipped: {error}");
                return Outcome::Skipped;
            }
            if matches!(
                error.downcast_ref::<frame_trace_replay::Error>(),
                Some(frame_trace_replay::Error::Cancelled)
            ) {
                eprintln!("replay job cancelled");
                return Outcome::Cancelled;
            }
            let retry = error
                .downcast_ref::<frame_trace_replay::Error>()
                .is_some_and(|e| e.is_transient())
                || error.downcast_ref::<frame_trace_otel::Error>().is_some();
            eprintln!(
                "replay job failed: {error:#}; {}",
                if retry { "transient" } else { "terminal" }
            );
            if retry {
                Outcome::Retryable
            } else {
                Outcome::Terminal
            }
        }
        Err(_) => {
            eprintln!("replay job panicked; terminal failure");
            Outcome::Terminal
        }
    }
}
fn finish(entry: &mut Entry, outcome: Outcome) {
    entry.attempts += 1;
    entry.state = if outcome == Outcome::Skipped {
        State::Skipped
    } else if outcome == Outcome::Cancelled {
        State::Cancelled
    } else if entry.reverted.load(Ordering::SeqCst) {
        State::Reverted
    } else {
        match outcome {
            Outcome::Passed => State::Done,
            Outcome::Skipped => State::Skipped,
            Outcome::Cancelled => State::Cancelled,
            Outcome::Retryable if entry.attempts < MAX_ATTEMPTS => State::Failed,
            _ => State::Terminal,
        }
    };
    entry.retry_at = Instant::now()
        + Duration::from_millis(
            (500u64 << entry.attempts.min(6)).min(30_000) + (entry.attempts as u64 * 71 % 251),
        );
}
fn number(header: &serde_json::Value) -> Result<u64> {
    let text = header["number"].as_str().context("header number missing")?;
    Ok(u64::from_str_radix(text.trim_start_matches("0x"), 16)?)
}

/// Bound both startup/reorg ancestry and catch-up to the recent tip. No pre-start backfill.
fn ancestry(
    mut header: serde_json::Value,
    floor: Option<u64>,
    known: &BTreeMap<String, Entry>,
    mut parent: impl FnMut(&str) -> Result<serde_json::Value>,
) -> Result<Vec<(String, u64)>> {
    let tip = number(&header)?;
    let lower = floor
        .unwrap_or(tip)
        .max(tip.saturating_sub(QUEUE as u64 - 1));
    let mut pending = Vec::new();
    loop {
        let height = number(&header)?;
        let hash = frame_trace_replay::header_hash(&header)?;
        if height < lower
            || known
                .get(&hash)
                .is_some_and(|e| !e.reverted.load(Ordering::SeqCst))
        {
            break;
        }
        pending.push((hash, height));
        if height <= lower {
            break;
        }
        let hash = header["parentHash"]
            .as_str()
            .context("parent hash missing")?;
        if known
            .get(hash)
            .is_some_and(|e| !e.reverted.load(Ordering::SeqCst))
        {
            break;
        }
        let fetched = parent(hash)?;
        anyhow::ensure!(
            frame_trace_replay::header_hash(&fetched)? == hash,
            "ancestry header hash mismatch"
        );
        anyhow::ensure!(
            number(&fetched)?.checked_add(1) == Some(height),
            "ancestry number mismatch"
        );
        header = fetched;
    }
    Ok(pending)
}
fn report_reorg(pending: &[(String, u64)], tip: u64, entries: &mut BTreeMap<String, Entry>) {
    let fork = pending
        .iter()
        .filter_map(|(hash, number)| {
            entries
                .iter()
                .any(|(old, e)| {
                    old != hash && e.number == *number && !e.reverted.load(Ordering::SeqCst)
                })
                .then_some(*number)
        })
        .chain(
            entries
                .values()
                .any(|e| e.number > tip && !e.reverted.load(Ordering::SeqCst))
                .then(|| tip.saturating_add(1)),
        )
        .min();
    if let Some(fork) = fork {
        for (hash, entry) in entries.iter_mut().filter(|(_, e)| e.number >= fork) {
            if !entry.reverted.swap(true, Ordering::SeqCst) {
                eprintln!(
                    "reorg: reverted block #{} {hash} (previous state {:?}; previously traced={})",
                    entry.number,
                    entry.state,
                    entry.state == State::Done
                );
                entry.state = State::Reverted;
            }
        }
    }
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
    let session =
        ReplaySession::with_cache_max_bytes(args.cache_dir.as_deref(), args.cache_max_bytes)?
            .with_rpc_metadata_fallback(args.rpc_metadata_fallback);
    let (send, receive) = mpsc::sync_channel::<Job>(QUEUE);
    let (events, completed) = mpsc::channel();
    let receive = Arc::new(Mutex::new(receive));
    let head_kind = if args.best { "best" } else { "finalized" };
    let active_floor = Arc::new(AtomicU64::new(0));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let receive = receive.clone();
            let active_floor = active_floor.clone();
            let rpc = rpc.fork();
            let session = session.clone();
            let otlp = args.otlp.clone();
            let events = events.clone();
            thread::spawn(move || {
                while !rpc.cancelled() {
                    let job = match receive.lock() {
                        Ok(rx) => match rx.recv_timeout(Duration::from_millis(100)) {
                            Ok(job) => job,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(_) => break,
                        },
                        Err(_) => break,
                    };
                    if rpc.cancelled() {
                        break;
                    }
                    let _ = events.send(Event::Started(job.hash.clone(), job.reverted.clone()));
                    let start = Instant::now();
                    let outcome = job_outcome(|| {
                        if job.reverted.load(Ordering::SeqCst) {
                            return Err(Skipped("block already reverted").into());
                        }
                        if job.number < active_floor.load(Ordering::SeqCst) {
                            return Err(Skipped("queued block expired behind tip").into());
                        }
                        let loaded = source::replay_loaded(
                            frame_trace_replay::online_with_session(&rpc, &job.hash, &session)?,
                            None,
                        )?;
                        let mut block = frame_trace_decode::annotate_at_block(
                            &loaded.traced,
                            loaded.metadata.as_ref(),
                            loaded.block_number,
                        );
                        block.execution = loaded.execution;
                        if let Some(endpoint) = &otlp {
                            frame_trace_otel::export_otlp(&block, endpoint, Timing::default())?;
                        }
                        eprintln!(
                            "traced {}{head_kind} block #{} {}: {} ops in {:.3}s",
                            if job.reverted.load(Ordering::SeqCst) {
                                "reverted "
                            } else {
                                ""
                            },
                            job.number,
                            job.hash,
                            block.totals.ops,
                            start.elapsed().as_secs_f64()
                        );
                        Ok(())
                    });
                    let _ = events.send(Event::Finished(job.hash, job.reverted, outcome));
                }
            })
        })
        .collect();
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut order = VecDeque::new();
    let mut floor = None;
    while !cancel.is_cancelled() {
        for event in completed.try_iter() {
            match event {
                Event::Started(hash, generation) => {
                    if let Some(entry) = current_entry(&mut entries, &hash, &generation)
                        && !entry.reverted.load(Ordering::SeqCst)
                    {
                        entry.state = State::InFlight;
                    }
                }
                Event::Finished(hash, generation, outcome) => {
                    if let Some(entry) = current_entry(&mut entries, &hash, &generation) {
                        finish(entry, outcome);
                        if entry.state == State::Terminal {
                            eprintln!(
                                "terminal block {hash} after {} attempt(s); no further retries",
                                entry.attempts
                            );
                        }
                    }
                }
            }
        }
        let result = (|| -> Result<()> {
            let header = if args.best {
                rpc.call("chain_getHeader", serde_json::json!([]))?
            } else {
                let hash = rpc.call("chain_getFinalizedHead", serde_json::json!([]))?;
                rpc.call("chain_getHeader", serde_json::json!([hash]))?
            };
            let tip = number(&header)?;
            floor = Some(
                floor
                    .unwrap_or(tip)
                    .max(tip.saturating_sub(QUEUE as u64 - 1)),
            );
            active_floor.store(floor.unwrap_or(tip), Ordering::SeqCst);
            let pending = ancestry(header, floor, &entries, |hash| {
                Ok(rpc.call("chain_getHeader", serde_json::json!([hash]))?)
            })?;
            if args.best {
                report_reorg(&pending, tip, &mut entries);
            }
            // Newest first: a saturated queue cannot force a long catch-up of stale blocks.
            for (hash, number) in pending {
                let reverted = Arc::new(AtomicBool::new(false));
                match send.try_send(Job {
                    hash: hash.clone(),
                    number,
                    reverted: reverted.clone(),
                }) {
                    Ok(()) => {
                        order.retain(|old| old != &hash);
                        order.push_back(hash.clone());
                        entries.insert(
                            hash,
                            Entry {
                                number,
                                state: State::Queued,
                                attempts: 0,
                                retry_at: Instant::now(),
                                reverted,
                            },
                        );
                    }
                    Err(mpsc::TrySendError::Full(_)) => {
                        eprintln!("follow queue full; sampling near the tip at replay speed");
                        break;
                    }
                    Err(_) => anyhow::bail!("replay workers stopped"),
                }
            }
            for (hash, entry) in &mut entries {
                if entry.state == State::Failed && entry.number < floor.unwrap_or(tip) {
                    entry.state = State::Skipped;
                    eprintln!("skipped block {hash}: retry expired behind tip");
                }
                if entry.state == State::Failed
                    && entry.retry_at <= Instant::now()
                    && send
                        .try_send(Job {
                            hash: hash.clone(),
                            number: entry.number,
                            reverted: entry.reverted.clone(),
                        })
                        .is_ok()
                {
                    entry.state = State::Queued;
                    eprintln!(
                        "retrying block {hash} (attempt {}/{MAX_ATTEMPTS})",
                        entry.attempts + 1
                    );
                }
            }
            while entries.len() > 256 {
                let Some(pos) = order.iter().position(|h| {
                    entries
                        .get(h)
                        .is_some_and(|e| !matches!(e.state, State::Queued | State::InFlight))
                }) else {
                    break;
                };
                if let Some(hash) = order.remove(pos) {
                    entries.remove(&hash);
                }
            }
            Ok(())
        })();
        if let Err(error) = result
            && !cancel.is_cancelled()
        {
            eprintln!("follow poll failed: {error:#}; retrying");
        }
        let counts = [
            State::Queued,
            State::InFlight,
            State::Done,
            State::Failed,
            State::Terminal,
            State::Reverted,
            State::Skipped,
            State::Cancelled,
        ]
        .map(|state| entries.values().filter(|e| e.state == state).count());
        eprintln!(
            "follow: queued={} in-flight={} done={} failed={} terminal={} reverted={} skipped={} cancelled={}",
            counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6], counts[7]
        );
        for _ in 0..20 {
            if cancel.is_cancelled() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    for (hash, entry) in &mut entries {
        if matches!(entry.state, State::Queued | State::InFlight | State::Failed) {
            entry.state = State::Cancelled;
            eprintln!("cancelled block {hash}: follow stopped");
        }
    }
    eprintln!(
        "follow shutdown: skipped={} cancelled={}",
        entries
            .values()
            .filter(|e| e.state == State::Skipped)
            .count(),
        entries
            .values()
            .filter(|e| e.state == State::Cancelled)
            .count()
    );
    drop(send);
    for worker in workers {
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
    eprintln!("follow stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use serde_json::json;
    fn header(number: u64, parent: &str, tag: u8) -> serde_json::Value {
        json!({"number":format!("0x{number:x}"), "parentHash":parent,
            "stateRoot":format!("0x{}", format!("{tag:02x}").repeat(32)),
            "extrinsicsRoot":format!("0x{}", "00".repeat(32)), "digest":{"logs":[]}})
    }
    fn entry(number: u64, state: State) -> Entry {
        Entry {
            number,
            state,
            attempts: 0,
            retry_at: Instant::now(),
            reverted: Arc::new(AtomicBool::new(false)),
        }
    }
    #[test]
    fn reorg_walk_stops_at_start_height_and_reports_traced_reversions() {
        let zero = format!("0x{}", "00".repeat(32));
        let old = header(100, &zero, 1);
        let old_hash = frame_trace_replay::header_hash(&old).unwrap();
        let mut entries = BTreeMap::from([(old_hash.clone(), entry(100, State::Done))]);
        let mut fork = Vec::new();
        let mut parent = zero;
        for number in 100..=102 {
            let h = header(number, &parent, 2);
            parent = frame_trace_replay::header_hash(&h).unwrap();
            fork.push(h);
        }
        let mut calls = 0;
        let pending = ancestry(fork[2].clone(), Some(100), &entries, |hash| {
            calls += 1;
            Ok(fork
                .iter()
                .find(|h| frame_trace_replay::header_hash(h).unwrap() == hash)
                .unwrap()
                .clone())
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            pending.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
            [102, 101, 100]
        );
        report_reorg(&pending, 102, &mut entries);
        assert_eq!(entries[&old_hash].state, State::Reverted);
        finish(entries.get_mut(&old_hash).unwrap(), Outcome::Passed);
        assert_eq!(entries[&old_hash].state, State::Reverted);
    }
    #[test]
    fn best_head_retreat_reports_reverted_descendants_even_when_new_tip_is_known() {
        let mut entries = BTreeMap::from([("old".into(), entry(100, State::Done))]);
        report_reorg(&[], 99, &mut entries);
        assert_eq!(entries["old"].state, State::Reverted);
    }
    #[test]
    fn catch_up_is_limited_to_eight_recent_blocks_and_startup_only_the_tip() {
        let mut chain = Vec::new();
        let mut parent = format!("0x{}", "00".repeat(32));
        for n in 1..=100 {
            let h = header(n, &parent, 1);
            parent = frame_trace_replay::header_hash(&h).unwrap();
            chain.push(h);
        }
        let entries = BTreeMap::new();
        assert_eq!(
            ancestry(chain[99].clone(), None, &entries, |_| panic!(
                "startup backfilled"
            ))
            .unwrap()
            .len(),
            1
        );
        let mut calls = 0;
        let pending = ancestry(chain[99].clone(), Some(1), &entries, |hash| {
            calls += 1;
            Ok(chain
                .iter()
                .find(|h| frame_trace_replay::header_hash(h).unwrap() == hash)
                .unwrap()
                .clone())
        })
        .unwrap();
        assert_eq!(calls, 7);
        assert_eq!(pending.len(), QUEUE);
        assert_eq!(pending.last().unwrap().1, 93);
    }
    #[test]
    fn whole_job_panics_are_terminal_and_workers_can_accept_the_next_job() {
        for stage in ["replay_loaded", "annotate", "export"] {
            assert_eq!(job_outcome(|| panic!("{stage}")), Outcome::Terminal);
            assert_eq!(job_outcome(|| Ok(())), Outcome::Passed);
        }
    }
    #[test]
    fn retry_cap_and_non_transient_failures_are_terminal() {
        let mut e = entry(1, State::InFlight);
        for attempt in 1..=MAX_ATTEMPTS {
            finish(&mut e, Outcome::Retryable);
            assert_eq!(
                e.state,
                if attempt == MAX_ATTEMPTS {
                    State::Terminal
                } else {
                    State::Failed
                }
            );
        }
        for error in [
            frame_trace_replay::Error::Execution("root mismatch".into()),
            frame_trace_replay::Error::Unavailable("pruned".into()),
            frame_trace_replay::Error::Invalid("bad block".into()),
        ] {
            assert_eq!(job_outcome(|| Err(error.into())), Outcome::Terminal);
        }
        assert_eq!(
            job_outcome(|| Err(frame_trace_replay::Error::Cancelled.into())),
            Outcome::Cancelled
        );
        assert_eq!(
            job_outcome(|| Err(Skipped("expired").into())),
            Outcome::Skipped
        );
        let mut skipped = entry(1, State::InFlight);
        skipped.reverted.store(true, Ordering::SeqCst);
        finish(&mut skipped, Outcome::Skipped);
        assert_eq!(skipped.state, State::Skipped);
        finish(&mut skipped, Outcome::Cancelled);
        assert_eq!(skipped.state, State::Cancelled);
        assert_eq!(
            job_outcome(|| Err(frame_trace_replay::Error::TransientRpc("timeout".into()).into())),
            Outcome::Retryable
        );
    }
}

#[cfg(test)]
mod canonical_again_tests {
    use super::*;
    #[test]
    fn reverted_tip_is_eligible_again_and_new_generation_survives_old_completion() {
        let header = serde_json::json!({"number":"0x1", "parentHash":format!("0x{}", "00".repeat(32)),
            "stateRoot":format!("0x{}", "11".repeat(32)), "extrinsicsRoot":format!("0x{}", "00".repeat(32)), "digest":{"logs":[]}});
        let hash = frame_trace_replay::header_hash(&header).expect("header");
        let old = Arc::new(AtomicBool::new(true));
        let mut entries = BTreeMap::from([(
            hash.clone(),
            Entry {
                number: 1,
                state: State::Reverted,
                attempts: 1,
                retry_at: Instant::now(),
                reverted: old.clone(),
            },
        )]);
        let pending = ancestry(header.clone(), Some(1), &entries, |_| panic!("past floor"))
            .expect("ancestry");
        assert_eq!(pending, [(hash.clone(), 1)]);
        let new = Arc::new(AtomicBool::new(false));
        entries.insert(
            hash.clone(),
            Entry {
                number: 1,
                state: State::Queued,
                attempts: 0,
                retry_at: Instant::now(),
                reverted: new.clone(),
            },
        );
        assert!(
            current_entry(&mut entries, &hash, &old).is_none(),
            "ignore events from the previous job"
        );
        let entry = current_entry(&mut entries, &hash, &new).expect("current job");
        finish(entry, Outcome::Passed);
        assert_eq!(entry.state, State::Done);
        assert!(
            ancestry(header, Some(1), &entries, |_| panic!("known tip"))
                .expect("ancestry")
                .is_empty()
        );
    }
}
