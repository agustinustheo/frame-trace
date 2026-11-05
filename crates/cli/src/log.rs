//! The `follow` log. Writing to stderr blocks when nobody reads it, for example a full pipe.
//! `follow` therefore queues its lines, and the replay library's, for one writer thread: only
//! that thread waits. Signals, locks and the stop never wait for a write.
//!
//! Before [`start`], as in the other commands, lines go straight to stderr.

use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant},
};

/// Lines waiting for the writer. Progress lines beyond `capacity` displace the oldest progress
/// line, which is counted as dropped. Important lines, such as errors and the summary, are
/// always kept.
pub struct Queue {
    state: Mutex<State>,
    ready: Condvar,
    capacity: usize,
}
#[derive(Default)]
struct State {
    lines: VecDeque<(String, bool)>,
    progress: usize,
    dropped: u64,
    writing: bool,
}
impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::default(),
            ready: Condvar::new(),
            capacity,
        }
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Queues a line without waiting for the writer.
    pub fn push(&self, text: String, important: bool) {
        let mut state = self.lock();
        if !important && state.progress >= self.capacity {
            if let Some(oldest) = state.lines.iter().position(|(_, important)| !important) {
                state.lines.remove(oldest);
                state.progress -= 1;
            }
            state.dropped += 1;
        }
        state.progress += usize::from(!important);
        state.lines.push_back((text, important));
        self.ready.notify_one();
    }
    /// The next line, and how many progress lines were dropped before it. Waits up to `wait`.
    fn pop(&self, wait: Duration) -> Option<(u64, String)> {
        let mut state = self.lock();
        if state.lines.is_empty() {
            state = self
                .ready
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        let (text, important) = state.lines.pop_front()?;
        state.progress -= usize::from(!important);
        state.writing = true;
        Some((std::mem::take(&mut state.dropped), text))
    }
    fn written(&self) {
        self.lock().writing = false;
    }
    fn idle(&self) -> bool {
        let state = self.lock();
        state.lines.is_empty() && !state.writing
    }
}

static QUEUE: OnceLock<Queue> = OnceLock::new();

/// Starts the writer thread. Later lines are queued.
pub fn start() {
    if QUEUE.set(Queue::new(4096)).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("log".into()).spawn(|| {
        let Some(queue) = QUEUE.get() else { return };
        loop {
            let Some((dropped, text)) = queue.pop(Duration::from_secs(1)) else {
                continue;
            };
            // The lock is not held here: this write may block for as long as stderr does.
            if dropped > 0 {
                eprintln!("follow: {dropped} log line(s) dropped while stderr was blocked");
            }
            eprintln!("{text}");
            queue.written();
        }
    });
}

/// Logs a line. `important` lines are never dropped.
pub fn line(text: String, important: bool) {
    match QUEUE.get() {
        Some(queue) => queue.push(text, important),
        None => eprintln!("{text}"),
    }
}

/// Waits up to `timeout` for queued lines to be written.
pub fn flush(timeout: Duration) {
    let Some(queue) = QUEUE.get() else { return };
    let deadline = Instant::now() + timeout;
    while !queue.idle() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A progress line: dropped, oldest first, when too many wait.
macro_rules! say {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*), false) };
}
/// An error, a stop line or the summary: always kept.
macro_rules! shout {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*), true) };
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn a_full_queue_drops_old_progress_but_keeps_errors_and_the_summary() {
        let queue = Queue::new(10);
        queue.push("error".into(), true);
        for n in 0..50 {
            queue.push(format!("progress {n}"), false);
        }
        queue.push("summary".into(), true);
        let mut lines = Vec::new();
        let mut dropped = 0;
        while let Some((before, text)) = queue.pop(Duration::ZERO) {
            dropped += before;
            lines.push(text);
            queue.written();
        }
        assert_eq!(dropped, 40);
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0], "error");
        assert_eq!(
            lines[1], "progress 40",
            "the newest progress lines are kept"
        );
        assert_eq!(lines.last().unwrap(), "summary");
        assert!(queue.idle());
    }
}
