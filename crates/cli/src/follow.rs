//! Bounded, finality-aware replay sampling. Unfinished jobs are marked cancelled at shutdown.
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
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLL_DELAY: Duration = Duration::from_secs(60);
// A quiet chain still prints a status line this often, and repeated failures log this often.
const HEARTBEAT: Duration = Duration::from_secs(60);
// Traces waiting for the collector. Each holds a whole decoded block, so memory stays bounded.
const EXPORT_QUEUE: usize = 32;
const FIRST_EXPORT_DELAY: Duration = Duration::from_secs(1);
const MAX_EXPORT_DELAY: Duration = Duration::from_secs(30);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

/// Consecutive failures of one kind: logged on the first, then at most once per [`HEARTBEAT`].
#[derive(Default)]
struct Streak {
    failures: u64,
    since: Option<Instant>,
    logged: Option<Instant>,
}
impl Streak {
    /// Records a failure and returns whether to log it.
    fn fail(&mut self) -> bool {
        let now = Instant::now();
        self.failures += 1;
        self.since.get_or_insert(now);
        let log = self
            .logged
            .is_none_or(|at| now.duration_since(at) >= HEARTBEAT);
        if log {
            self.logged = Some(now);
        }
        log
    }
    /// Ends the streak, returning its failures and length.
    fn recover(&mut self) -> Option<(u64, Duration)> {
        let since = self.since.take()?;
        self.logged = None;
        Some((std::mem::take(&mut self.failures), since.elapsed()))
    }
    /// `first` after one failure, doubling per further failure, capped at `max`.
    fn delay(&self, first: Duration, max: Duration) -> Duration {
        let doublings = self.failures.saturating_sub(1).min(16) as u32;
        first.saturating_mul(1 << doublings).min(max)
    }
}

fn duration_text(duration: Duration) -> String {
    let secs = duration.as_secs();
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m{s:02}s"),
        (h, m, s) => format!("{h}h{m:02}m{s:02}s"),
    }
}

/// Sleeps in short steps so a stop signal is seen within 100 ms.
fn pause(duration: Duration, stop: impl Fn() -> bool) {
    let deadline = Instant::now() + duration;
    while !stop() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        thread::sleep(left.min(Duration::from_millis(100)));
    }
}

struct Trace {
    number: u64,
    hash: String,
    block: frame_trace_decode::AnnotatedBlock,
}
#[derive(Default)]
struct ExportState {
    waiting: VecDeque<Trace>,
    exported: u64,
    // Pushed out of the full queue while the collector was failing.
    dropped: u64,
    // Refused by the receiver or impossible to send; never retried.
    rejected: u64,
    full: Streak,
}
/// Replayed traces waiting for the OTLP collector. Export failures retry the export only,
/// never the replay, so a collector outage does not cost RPC requests.
#[derive(Default)]
struct Exports {
    state: Mutex<ExportState>,
    ready: Condvar,
    // Set once workers have stopped: send what is left once, without waiting between tries.
    draining: AtomicBool,
}
impl Exports {
    fn lock(&self) -> std::sync::MutexGuard<'_, ExportState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn push(&self, trace: Trace) {
        let mut state = self.lock();
        state.waiting.push_back(trace);
        drop_oldest(&mut state);
        self.ready.notify_one();
    }
    fn next(&self) -> Option<Trace> {
        let mut state = self.lock();
        loop {
            if let Some(trace) = state.waiting.pop_front() {
                return Some(trace);
            }
            if self.draining.load(Ordering::SeqCst) {
                return None;
            }
            state = self
                .ready
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
    fn run(&self, endpoint: &str) {
        let mut streak = Streak::default();
        while let Some(trace) = self.next() {
            let draining = self.draining.load(Ordering::SeqCst);
            let error = match frame_trace_otel::export_otlp(
                &trace.block,
                endpoint,
                Timing::default(),
            ) {
                Ok(_) => {
                    let mut state = self.lock();
                    state.exported += 1;
                    if let Some((failures, lasted)) = streak.recover() {
                        eprintln!(
                            "OTLP export recovered after {failures} failed attempt(s) over {}; {} trace(s) waiting",
                            duration_text(lasted),
                            state.waiting.len()
                        );
                    }
                    continue;
                }
                Err(error) => error,
            };
            // Partial rejection is the receiver's answer about these spans; sending them again
            // would duplicate the accepted ones. A bad endpoint cannot get better either.
            if matches!(
                error,
                frame_trace_otel::Error::Rejected { .. } | frame_trace_otel::Error::Build(_)
            ) {
                self.lock().rejected += 1;
                eprintln!(
                    "OTLP export of block #{} {} failed: {error}; will not retry",
                    trace.number, trace.hash
                );
                continue;
            }
            let log = streak.fail();
            let delay = streak.delay(FIRST_EXPORT_DELAY, MAX_EXPORT_DELAY);
            let waiting = {
                let mut state = self.lock();
                state.waiting.push_front(trace);
                drop_oldest(&mut state);
                state.waiting.len()
            };
            if draining {
                // The receiver is still down: trying the rest would only use up the stop wait.
                eprintln!(
                    "OTLP export failed during shutdown: {error}; {waiting} trace(s) not sent"
                );
                return;
            }
            if log {
                eprintln!(
                    "OTLP export failed ({} attempt(s) in a row): {error}; retrying in {}, {waiting} trace(s) waiting",
                    streak.failures,
                    duration_text(delay)
                );
            }
            pause(delay, || self.draining.load(Ordering::SeqCst));
        }
    }
}
fn drop_oldest(state: &mut ExportState) {
    while state.waiting.len() > EXPORT_QUEUE {
        let Some(old) = state.waiting.pop_front() else {
            break;
        };
        state.dropped += 1;
        if state.full.fail() {
            eprintln!(
                "OTLP export queue full ({EXPORT_QUEUE} traces); dropped the oldest, block #{} {} ({} dropped so far)",
                old.number, old.hash, state.dropped
            );
        }
    }
}

/// Totals since start. The block table only keeps recent blocks, so it cannot count these.
#[derive(Default)]
struct Totals {
    // Counted by workers before a trace is queued for export, so it never trails `exported`.
    traced: Arc<AtomicU64>,
    stopped: u64,
    skipped: u64,
    reverted: u64,
    cancelled: u64,
    poll_failures: u64,
}
impl Totals {
    fn count(&mut self, state: State) {
        match state {
            State::Terminal => self.stopped += 1,
            State::Skipped => self.skipped += 1,
            State::Cancelled => self.cancelled += 1,
            _ => {}
        }
    }
}

// Four queued blocks per worker absorb cold runtime startup without unbounded catch-up.
fn queue_capacity(workers: u8) -> usize {
    usize::from(workers).max(2) * 4
}
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
                .is_some_and(|e| e.is_transient());
            eprintln!(
                "replay job failed: {error:#}; {}",
                if retry {
                    "will retry"
                } else {
                    "will not retry"
                }
            );
            if retry {
                Outcome::Retryable
            } else {
                Outcome::Terminal
            }
        }
        Err(_) => {
            eprintln!("replay job panicked; will not retry");
            Outcome::Terminal
        }
    }
}
fn finish(entry: &mut Entry, outcome: &Outcome) {
    entry.attempts += 1;
    entry.state = if *outcome == Outcome::Skipped {
        State::Skipped
    } else if *outcome == Outcome::Cancelled {
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
    capacity: usize,
    mut parent: impl FnMut(&str) -> Result<serde_json::Value>,
) -> Result<Vec<(String, u64)>> {
    let tip = number(&header)?;
    let lower = floor
        .unwrap_or(tip)
        .max(tip.saturating_sub(capacity as u64 - 1));
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
/// Marks blocks replaced by a reorg and returns how many were newly reverted.
fn report_reorg(pending: &[(String, u64)], tip: u64, entries: &mut BTreeMap<String, Entry>) -> u64 {
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
    let mut reverted = 0;
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
                reverted += 1;
            }
        }
    }
    reverted
}

pub fn run(args: FollowArgs) -> Result<()> {
    let started = Instant::now();
    // Wasmtime uses Rayon's global pool to compile functions. Using every CPU here retains
    // hundreds of MiB of compiler allocations on macOS, despite network-bound replay.
    // This is a CLI process policy; the replay library does not change its caller's pool.
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|index| format!("wasm-compile-{index}"))
        .build_global()
        .context("initializing runtime compilation pool")?;
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    // Ctrl-C, SIGTERM and SIGHUP all stop gracefully; a second one exits at once.
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
    let capacity = queue_capacity(args.workers);
    let (send, receive) = mpsc::sync_channel::<Job>(capacity);
    let (events, completed) = mpsc::channel();
    let receive = Arc::new(Mutex::new(receive));
    let head_kind = if args.best { "best" } else { "finalized" };
    let active_floor = Arc::new(AtomicU64::new(0));
    let mut totals = Totals::default();
    let exports = args.otlp.as_ref().map(|_| Arc::new(Exports::default()));
    let exporter = args
        .otlp
        .clone()
        .zip(exports.clone())
        .map(|(endpoint, exports)| thread::spawn(move || exports.run(&endpoint)));
    let workers: Vec<_> = (0..args.workers)
        .map(|_| {
            let receive = receive.clone();
            let active_floor = active_floor.clone();
            let rpc = rpc.fork();
            let cancel = cancel.clone();
            let session = session.clone();
            let exports = exports.clone();
            let events = events.clone();
            let traced = totals.traced.clone();
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
                    let outcome = job_outcome(|| {
                        // Waits behind a cold replay; returns Cancelled before or after admission.
                        // Expiry is checked after admission, since the wait can outlast the window.
                        let cold_start = session.cold_start(&cancel)?;
                        if job.reverted.load(Ordering::SeqCst) {
                            return Err(Skipped("block already reverted").into());
                        }
                        if job.number < active_floor.load(Ordering::SeqCst) {
                            return Err(Skipped("queued block expired behind tip").into());
                        }
                        let start = Instant::now();
                        let replay =
                            frame_trace_replay::online_with_session(&rpc, &job.hash, &session)?;
                        drop(cold_start);
                        let loaded = source::replay_loaded(replay, None)?;
                        let mut block = frame_trace_decode::annotate_at_block(
                            &loaded.traced,
                            loaded.metadata.as_ref(),
                            loaded.block_number,
                        );
                        block.execution = loaded.execution;
                        eprintln!(
                            "traced {}{head_kind} block #{} {}: {} storage operations in {:.3}s",
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
                        traced.fetch_add(1, Ordering::SeqCst);
                        if let Some(exports) = &exports {
                            exports.push(Trace {
                                number: job.number,
                                hash: job.hash.clone(),
                                block,
                            });
                        }
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
    let mut poll = Streak::default();
    let mut full = Streak::default();
    let mut status = (String::new(), None::<Instant>);
    let process = |event: Event,
                   entries: &mut BTreeMap<String, Entry>,
                   totals: &mut Totals,
                   shutdown: bool| {
        match event {
            Event::Started(hash, generation) => {
                if let Some(entry) = current_entry(entries, &hash, &generation)
                    && !entry.reverted.load(Ordering::SeqCst)
                {
                    entry.state = State::InFlight;
                }
            }
            Event::Finished(hash, generation, outcome) => {
                if let Some(entry) = current_entry(entries, &hash, &generation) {
                    finish(entry, &outcome);
                    totals.count(entry.state);
                    if entry.state == State::Terminal && !shutdown {
                        eprintln!(
                            "stopped retrying block {hash} after {} attempt(s); no further retries",
                            entry.attempts
                        );
                    }
                }
            }
        }
    };
    while !cancel.is_cancelled() {
        for event in completed.try_iter() {
            process(event, &mut entries, &mut totals, false);
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
                    .max(tip.saturating_sub(capacity as u64 - 1)),
            );
            active_floor.store(floor.unwrap_or(tip), Ordering::SeqCst);
            let pending = ancestry(header, floor, &entries, capacity, |hash| {
                Ok(rpc.call("chain_getHeader", serde_json::json!([hash]))?)
            })?;
            if args.best {
                totals.reverted += report_reorg(&pending, tip, &mut entries);
            }
            // Newest first: a saturated queue cannot force a long catch-up of stale blocks.
            let mut saturated = false;
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
                        saturated = true;
                        if full.fail() {
                            eprintln!("follow queue full; sampling near the tip at replay speed");
                        }
                        break;
                    }
                    Err(_) => anyhow::bail!("replay workers stopped"),
                }
            }
            if !saturated {
                full.recover();
            }
            for (hash, entry) in &mut entries {
                if entry.state == State::Failed && entry.number < floor.unwrap_or(tip) {
                    entry.state = State::Skipped;
                    totals.skipped += 1;
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
        match result {
            Ok(()) => {
                if let Some((failures, lasted)) = poll.recover() {
                    eprintln!(
                        "follow poll recovered after {failures} failed attempt(s) over {}",
                        duration_text(lasted)
                    );
                }
            }
            Err(error) if !cancel.is_cancelled() => {
                totals.poll_failures += 1;
                if poll.fail() {
                    let delay = duration_text(poll.delay(POLL_INTERVAL, MAX_POLL_DELAY));
                    if poll.failures == 1 {
                        eprintln!("follow poll failed: {error:#}; retrying in {delay}");
                    } else {
                        eprintln!(
                            "follow poll still failing after {} attempts over {}: {error:#}; retrying in {delay}",
                            poll.failures,
                            duration_text(poll.since.map_or(Duration::ZERO, |at| at.elapsed()))
                        );
                    }
                }
            }
            Err(_) => {}
        }
        let line = status_line(&entries, &totals, exports.as_deref());
        // Print on a change worth reading, otherwise once a minute so a quiet run shows it is alive.
        if line.0 != status.0 || status.1.is_none_or(|at| at.elapsed() >= HEARTBEAT) {
            eprintln!("{} up={}", line.1, duration_text(started.elapsed()));
            status = (line.0, Some(Instant::now()));
        }
        pause(poll.delay(POLL_INTERVAL, MAX_POLL_DELAY), || {
            cancel.is_cancelled()
        });
    }
    drop(send);
    for event in completed.try_iter() {
        process(event, &mut entries, &mut totals, false);
    }
    let busy = entries
        .values()
        .filter(|e| e.state == State::InFlight)
        .count();
    eprintln!(
        "follow: stopping; waiting up to {} for {busy} block(s) being replayed (signal again to exit now)",
        duration_text(SHUTDOWN_WAIT)
    );
    let unfinished = join_workers(workers, SHUTDOWN_WAIT);
    if unfinished != 0 {
        eprintln!(
            "follow shutdown: {unfinished} of {} worker(s) still replaying after {}; their blocks count as cancelled",
            args.workers,
            duration_text(SHUTDOWN_WAIT)
        );
    }
    for event in completed.try_iter() {
        process(event, &mut entries, &mut totals, true);
    }
    for (hash, entry) in &mut entries {
        if matches!(entry.state, State::Queued | State::InFlight | State::Failed) {
            entry.state = State::Cancelled;
            totals.cancelled += 1;
            eprintln!("cancelled block {hash}: follow stopped");
        }
    }
    let mut export_summary = String::new();
    if let (Some(exports), Some(exporter)) = (&exports, exporter) {
        let waiting = exports.lock().waiting.len();
        if waiting > 0 {
            eprintln!(
                "follow shutdown: exporting {waiting} waiting trace(s), up to {}",
                duration_text(SHUTDOWN_WAIT)
            );
        }
        exports.draining.store(true, Ordering::SeqCst);
        exports.ready.notify_all();
        if join_workers(vec![exporter], SHUTDOWN_WAIT) != 0 {
            eprintln!("follow shutdown: OTLP export still running; remaining traces not sent");
        }
        let state = exports.lock();
        // Traced but neither exported, refused nor dropped: still queued or mid-export at exit.
        let unsent = totals
            .traced
            .load(Ordering::SeqCst)
            .saturating_sub(state.exported + state.rejected + state.dropped);
        export_summary = format!(
            " exported={} export-rejected={} export-dropped={} export-unsent={unsent}",
            state.exported, state.rejected, state.dropped
        );
    }
    eprintln!(
        "follow stopped after {}: traced={}{export_summary} stopped={} skipped={} reverted={} cancelled={} poll-failures={}",
        duration_text(started.elapsed()),
        totals.traced.load(Ordering::SeqCst),
        totals.stopped,
        totals.skipped,
        totals.reverted,
        totals.cancelled,
        totals.poll_failures
    );
    Ok(())
}

/// The status line, and the part of it whose change is worth printing at once.
fn status_line(
    entries: &BTreeMap<String, Entry>,
    totals: &Totals,
    exports: Option<&Exports>,
) -> (String, String) {
    let count = |state| entries.values().filter(|e| e.state == state).count();
    let problems = format!(
        "retrying={} stopped={} skipped={} reverted={}",
        count(State::Failed),
        totals.stopped,
        totals.skipped,
        totals.reverted
    );
    let mut line = format!(
        "follow: queued={} in-flight={} traced={}",
        count(State::Queued),
        count(State::InFlight),
        totals.traced.load(Ordering::SeqCst)
    );
    let mut key = problems.clone();
    if let Some(exports) = exports {
        let state = exports.lock();
        line += &format!(
            " exported={} export-waiting={}",
            state.exported,
            state.waiting.len()
        );
        line += &format!(
            " export-rejected={} export-dropped={}",
            state.rejected, state.dropped
        );
        // Drops have their own line, at most once a minute, so they do not trigger this one.
        key += &format!(" export-rejected={}", state.rejected);
    }
    (key, format!("{line} {problems}"))
}

// One shared deadline bounds shutdown even when a CPU replay or export ignores cancellation.
fn join_workers(mut workers: Vec<thread::JoinHandle<()>>, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let mut pending = Vec::new();
        for worker in workers {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                pending.push(worker);
            }
        }
        if pending.is_empty() || Instant::now() >= deadline {
            return pending.len();
        }
        workers = pending;
        thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
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
        let pending = ancestry(fork[2].clone(), Some(100), &entries, 8, |hash| {
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
        finish(entries.get_mut(&old_hash).unwrap(), &Outcome::Passed);
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
            ancestry(chain[99].clone(), None, &entries, 8, |_| panic!(
                "startup backfilled"
            ))
            .unwrap()
            .len(),
            1
        );
        let mut calls = 0;
        let pending = ancestry(chain[99].clone(), Some(1), &entries, 8, |hash| {
            calls += 1;
            Ok(chain
                .iter()
                .find(|h| frame_trace_replay::header_hash(h).unwrap() == hash)
                .unwrap()
                .clone())
        })
        .unwrap();
        assert_eq!(calls, 7);
        assert_eq!(pending.len(), 8);
        assert_eq!(pending.last().unwrap().1, 93);
    }
    #[test]
    fn worker_queue_preserves_consecutive_heights_during_startup() {
        for workers in [1, 2, 4, 8, 16] {
            let capacity = queue_capacity(workers);
            assert_eq!(capacity, usize::from(workers).max(2) * 4);
            let mut chain = BTreeMap::new();
            let mut parent = format!("0x{}", "00".repeat(32));
            let start = 100;
            let known = BTreeMap::from([(parent.clone(), entry(start, State::InFlight))]);
            let mut tip = serde_json::Value::Null;
            for height in start + 1..=start + capacity as u64 - 1 {
                tip = header(height, &parent, 1);
                parent = frame_trace_replay::header_hash(&tip).unwrap();
                chain.insert(parent.clone(), tip.clone());
            }
            let pending = ancestry(tip, Some(start), &known, capacity, |hash| {
                Ok(chain[hash].clone())
            })
            .unwrap();
            assert_eq!(pending.len(), capacity - 1);
            let mut heights: Vec<_> = pending.into_iter().map(|(_, height)| height).collect();
            heights.sort_unstable();
            assert_eq!(
                heights,
                (start + 1..start + capacity as u64).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn failure_streaks_back_off_log_once_a_minute_and_reset_on_recovery() {
        let mut streak = Streak::default();
        assert_eq!(streak.delay(POLL_INTERVAL, MAX_POLL_DELAY), POLL_INTERVAL);
        let mut logged = 0;
        let mut delays = Vec::new();
        for _ in 0..8 {
            logged += usize::from(streak.fail());
            delays.push(streak.delay(POLL_INTERVAL, MAX_POLL_DELAY).as_secs());
        }
        assert_eq!(delays, [2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(logged, 1, "later failures within a minute are not logged");
        streak.logged = Some(Instant::now() - HEARTBEAT);
        assert!(
            streak.fail(),
            "a long streak is logged again after a minute"
        );
        assert_eq!(streak.recover().map(|(failures, _)| failures), Some(9));
        assert_eq!(streak.recover(), None);
        assert_eq!(streak.delay(POLL_INTERVAL, MAX_POLL_DELAY), POLL_INTERVAL);
        assert!(streak.fail(), "a new streak logs its first failure");
        assert_eq!(duration_text(Duration::from_secs(59)), "59s");
        assert_eq!(duration_text(Duration::from_secs(192)), "3m12s");
        assert_eq!(duration_text(Duration::from_secs(3723)), "1h02m03s");
    }
    #[test]
    fn export_queue_keeps_the_newest_traces_and_counts_drops() {
        let block = frame_trace_decode::annotate_at_block(
            &frame_trace_model::TracedBlock {
                runtime_spans: Vec::new(),
                timing: None,
                block_hash: String::new(),
                parent_hash: String::new(),
                ops: Vec::new(),
                phases: Vec::new(),
                skipped_events: 0,
            },
            None,
            None,
        );
        let exports = Exports::default();
        for number in 0..EXPORT_QUEUE as u64 + 5 {
            exports.push(Trace {
                number,
                hash: format!("0x{number}"),
                block: block.clone(),
            });
        }
        let state = exports.lock();
        assert_eq!(state.waiting.len(), EXPORT_QUEUE);
        assert_eq!(state.dropped, 5);
        assert_eq!(state.waiting.front().map(|t| t.number), Some(5));
        drop(state);
        exports.draining.store(true, Ordering::SeqCst);
        let mut sent = 0;
        while exports.next().is_some() {
            sent += 1;
        }
        assert_eq!(
            sent, EXPORT_QUEUE,
            "draining hands over what is left, then stops"
        );
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
            finish(&mut e, &Outcome::Retryable);
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
        finish(&mut skipped, &Outcome::Skipped);
        assert_eq!(skipped.state, State::Skipped);
        finish(&mut skipped, &Outcome::Cancelled);
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
        let pending = ancestry(header.clone(), Some(1), &entries, 8, |_| {
            panic!("past floor")
        })
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
        finish(entry, &Outcome::Passed);
        assert_eq!(entry.state, State::Done);
        assert!(
            ancestry(header, Some(1), &entries, 8, |_| panic!("known tip"))
                .expect("ancestry")
                .is_empty()
        );
    }
}
