//! The `follow` log. Writing to stderr blocks when nobody reads it, for example a full pipe.
//! `follow` therefore queues its lines, and the replay library's, for one writer thread: only
//! that thread waits. Signals, locks and the stop never wait for a write.
//!
//! One policy bounds the queue: at most [`MAX_LINES`] lines and [`MAX_BYTES`] bytes. When it is
//! full, the oldest line of the least important kind goes. A line never displaces a more
//! important one; it is dropped instead. Every dropped line is counted. The summary and a fatal
//! error are the most important kind, so they always find room.
//!
//! Before [`start`], as in the other commands, lines go straight to stderr.

use std::{
    collections::VecDeque,
    io::Write,
    sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant},
};

const MAX_LINES: usize = 4096;
const MAX_BYTES: usize = 1 << 20;
// A longer line is cut, so one line cannot fill the byte limit.
const MAX_LINE_BYTES: usize = 16 << 10;

/// How much a line matters when the queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Progress: traced blocks, status lines.
    Progress,
    /// Failures and warnings.
    Failure,
    /// The summary and a fatal error.
    Final,
}

struct Line {
    text: String,
    kind: Kind,
    // The same line queued again right after itself.
    repeats: u64,
}

/// Lines waiting for the writer.
pub struct Queue {
    state: Mutex<State>,
    ready: Condvar,
    max_lines: usize,
    max_bytes: usize,
}
#[derive(Default)]
struct State {
    lines: VecDeque<Line>,
    bytes: usize,
    // Dropped since the last line written, and in total.
    dropped: u64,
    total_dropped: u64,
    writing: bool,
    // The writer could not write; later lines are counted as dropped, not kept.
    closed: bool,
}
impl Queue {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            state: Mutex::default(),
            ready: Condvar::new(),
            max_lines,
            max_bytes,
        }
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Queues a line without waiting for the writer.
    pub fn push(&self, text: String, kind: Kind) {
        let text = cut(text);
        let mut state = self.lock();
        if state.closed {
            state.total_dropped += 1;
            return;
        }
        if let Some(last) = state.lines.back_mut()
            && last.text == text
            && last.kind == kind
        {
            last.repeats += 1;
            return;
        }
        if self.make_room(&mut state, text.len(), kind) {
            self.append(&mut state, text, kind);
        }
    }
    /// Queues the line `make` builds from the total of dropped lines, taken after making room
    /// for it, so the count includes what this line pushed out.
    pub fn push_counted(&self, kind: Kind, make: impl Fn(u64) -> String) {
        let mut state = self.lock();
        if state.closed {
            state.total_dropped += 1;
            return;
        }
        // The longest the line can be: the count has at most as many digits as u64::MAX.
        let longest = cut(make(u64::MAX)).len();
        if self.make_room(&mut state, longest, kind) {
            let text = cut(make(state.total_dropped));
            self.append(&mut state, text, kind);
        }
    }
    /// Drops the oldest lines of the least important kind, never more important than `kind`,
    /// until a line of `len` bytes fits. Returns false, counting it as dropped, if it cannot.
    fn make_room(&self, state: &mut State, len: usize, kind: Kind) -> bool {
        while state.lines.len() >= self.max_lines || state.bytes + len > self.max_bytes {
            let victim = state
                .lines
                .iter()
                .map(|line| line.kind)
                .min()
                .filter(|least| *least <= kind)
                .and_then(|least| state.lines.iter().position(|line| line.kind == least));
            let Some(victim) = victim.and_then(|at| state.lines.remove(at)) else {
                state.dropped += 1;
                state.total_dropped += 1;
                return false;
            };
            state.bytes -= victim.text.len();
            state.dropped += 1 + victim.repeats;
            state.total_dropped += 1 + victim.repeats;
        }
        true
    }
    fn append(&self, state: &mut State, text: String, kind: Kind) {
        state.bytes += text.len();
        state.lines.push_back(Line {
            text,
            kind,
            repeats: 0,
        });
        self.ready.notify_one();
    }
    /// The next line to write, with the lines dropped before it. Waits up to `wait`.
    fn pop(&self, wait: Duration) -> Option<(u64, Line)> {
        let mut state = self.lock();
        if state.lines.is_empty() {
            state = self
                .ready
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        let line = state.lines.pop_front()?;
        state.bytes -= line.text.len();
        state.writing = true;
        Some((std::mem::take(&mut state.dropped), line))
    }
    fn written(&self) {
        self.lock().writing = false;
    }
    /// The writer cannot write: what is queued is lost, and nothing more is kept.
    fn close(&self) {
        let mut state = self.lock();
        let lost: u64 = state.lines.iter().map(|line| 1 + line.repeats).sum();
        state.total_dropped += lost + 1;
        state.lines.clear();
        state.bytes = 0;
        state.writing = false;
        state.closed = true;
    }
    fn idle(&self) -> bool {
        let state = self.lock();
        state.lines.is_empty() && !state.writing
    }
    #[cfg(test)]
    fn total_dropped(&self) -> u64 {
        self.lock().total_dropped
    }
    /// Writes queued lines to `out` until a write fails. The lock is not held while writing.
    fn write_all(&self, out: &mut impl Write) {
        loop {
            let Some((dropped, line)) = self.pop(Duration::from_secs(1)) else {
                continue;
            };
            let mut text = String::new();
            if dropped > 0 {
                text +=
                    &format!("follow: {dropped} log line(s) dropped while stderr was blocked\n");
            }
            text += &line.text;
            if line.repeats > 0 {
                text += &format!(" (repeated {} more time(s))", line.repeats);
            }
            text.push('\n');
            if out.write_all(text.as_bytes()).is_err() {
                self.close();
                return;
            }
            self.written();
        }
    }
}

/// A line of at most [`MAX_LINE_BYTES`], marker included, in a fresh allocation: the queue
/// counts bytes by length, so a cut line must not keep its original capacity.
fn cut(text: String) -> String {
    const MARKER: &str = " [cut]";
    if text.len() <= MAX_LINE_BYTES {
        return text;
    }
    let mut end = MAX_LINE_BYTES - MARKER.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut short = String::with_capacity(end + MARKER.len());
    short.push_str(&text[..end]);
    short.push_str(MARKER);
    short
}

static QUEUE: OnceLock<Queue> = OnceLock::new();

/// Starts the writer thread. Later lines are queued.
pub fn start() {
    if QUEUE.set(Queue::new(MAX_LINES, MAX_BYTES)).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new().name("log".into()).spawn(|| {
        if let Some(queue) = QUEUE.get() {
            queue.write_all(&mut std::io::stderr());
        }
    });
    if spawned.is_err()
        && let Some(queue) = QUEUE.get()
    {
        queue.close();
    }
}

/// Logs a line.
pub fn line(text: String, kind: Kind) {
    match QUEUE.get() {
        Some(queue) => queue.push(text, kind),
        None => eprintln!("{text}"),
    }
}

/// Logs the line `make` builds from the number of lines dropped so far, counted after this
/// line has found its room.
pub fn line_counted(kind: Kind, make: impl Fn(u64) -> String) {
    match QUEUE.get() {
        Some(queue) => queue.push_counted(kind, make),
        None => eprintln!("{}", make(0)),
    }
}

/// Whether writing to stderr has failed. The log is then closed for good.
pub fn broken() -> bool {
    QUEUE.get().is_some_and(|queue| queue.lock().closed)
}

/// Waits up to `timeout` for queued lines to be written.
pub fn flush(timeout: Duration) {
    let Some(queue) = QUEUE.get() else { return };
    let deadline = Instant::now() + timeout;
    while !queue.idle() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Progress: the first to go when too many lines wait.
macro_rules! say {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*), $crate::log::Kind::Progress) };
}
/// A failure or warning: kept ahead of progress.
macro_rules! shout {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*), $crate::log::Kind::Failure) };
}
/// The summary or a fatal error: kept ahead of everything else.
macro_rules! last_word {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*), $crate::log::Kind::Final) };
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn drain(queue: &Queue) -> (u64, Vec<String>) {
        let mut dropped = 0;
        let mut lines = Vec::new();
        while let Some((before, line)) = queue.pop(Duration::ZERO) {
            dropped += before;
            lines.push(line.text);
            queue.written();
        }
        (dropped, lines)
    }

    #[test]
    fn a_full_queue_drops_the_least_important_lines_first_and_counts_them() {
        let queue = Queue::new(10, MAX_BYTES);
        queue.push("error 0".into(), Kind::Failure);
        for n in 0..50 {
            queue.push(format!("progress {n}"), Kind::Progress);
        }
        queue.push("summary".into(), Kind::Final);
        let (dropped, lines) = drain(&queue);
        assert_eq!(dropped, 42);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "error 0");
        assert_eq!(
            lines[1], "progress 42",
            "the newest progress lines are kept"
        );
        assert_eq!(lines.last().unwrap(), "summary");
    }

    #[test]
    fn errors_are_bounded_too_and_the_summary_still_fits() {
        let queue = Queue::new(10, MAX_BYTES);
        for n in 0..100_000 {
            queue.push(format!("error {n}"), Kind::Failure);
        }
        queue.push("summary".into(), Kind::Final);
        // Progress cannot push out a failure: it is dropped instead.
        queue.push("progress".into(), Kind::Progress);
        let (dropped, lines) = drain(&queue);
        assert_eq!(lines.len(), 10);
        assert_eq!(dropped, 100_000 - 9 + 1);
        assert_eq!(queue.total_dropped(), 100_000 - 9 + 1);
        assert_eq!(lines[0], "error 99991");
        assert_eq!(lines.last().unwrap(), "summary");
    }

    #[test]
    fn bytes_are_bounded_and_repeats_collapse() {
        let queue = Queue::new(MAX_LINES, 100);
        for _ in 0..1000 {
            queue.push("same failure".into(), Kind::Failure);
        }
        queue.push("x".repeat(MAX_LINE_BYTES * 2), Kind::Progress);
        let state = queue.lock();
        assert_eq!(state.lines.len(), 1);
        assert_eq!(state.lines[0].repeats, 999);
        assert!(state.bytes <= 100);
        assert_eq!(state.total_dropped, 1, "the long line did not fit");
    }

    #[test]
    fn the_summary_counts_the_lines_its_own_room_cost() {
        let queue = Queue::new(3, MAX_BYTES);
        for n in 0..3 {
            queue.push(format!("progress {n}"), Kind::Progress);
        }
        queue.push_counted(Kind::Final, |dropped| {
            format!("summary log-dropped={dropped}")
        });
        let (_, lines) = drain(&queue);
        assert_eq!(lines.last().unwrap(), "summary log-dropped=1");
    }

    #[test]
    fn a_failed_write_closes_the_log_and_counts_what_is_lost() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let queue = Queue::new(10, MAX_BYTES);
        queue.push("first".into(), Kind::Progress);
        queue.push("second".into(), Kind::Failure);
        queue.write_all(&mut Broken);
        assert!(queue.lock().closed);
        queue.push("third".into(), Kind::Final);
        assert!(queue.idle(), "nothing is kept or being written");
        assert_eq!(queue.total_dropped(), 3);
    }
}
