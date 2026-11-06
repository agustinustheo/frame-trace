//! Proof nodes on disk: append-only segment files, evicted whole and oldest first.
//!
//! A segment is `MAGIC` followed by records `[hash: 32][len: u32 LE][node bytes]`. Records are
//! never rewritten. Nothing is fsynced, by design: the cache is disposable. A crash can leave
//! a torn record at the end of a segment, and a power loss can also leave zeroed or stale
//! blocks in the middle of one. Every read checks the record's full hash against the node's
//! content, so damage of either kind is a miss, never a wrong node.
//!
//! The index maps the first 8 bytes of a node hash to its segment and offset: 16 bytes per
//! node in the table, about 30 bytes per node with the table's spare capacity. A prefix
//! collision only costs a miss. Startup lists the segment files and returns; a background
//! thread scans them sequentially to rebuild the index.
//!
//! One process per directory is the supported use. A second process never appends to a
//! segment the first is writing (a file lock), but neither counts the other's files, so their
//! sum can exceed the limit and their evictions cause misses, never wrong bytes.
use parking_lot::Mutex;
use sp_core::H256;
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    hash::{BuildHasherDefault, Hasher},
    io::{BufReader, Read},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

const MAGIC: &[u8; 8] = b"FTNODES1";
const HEADER: u64 = 36;
/// No trie node is larger: values above 32 bytes are separate nodes, and runtime code is a
/// few MiB. A record longer than this is damage.
pub(crate) const MAX_NODE: usize = 16 * 1024 * 1024;
/// File systems allocate whole blocks, and each file also costs an inode and a directory
/// entry. Counting one extra block per file keeps the total at or above real disk usage.
const BLOCK: u64 = 4096;
pub(crate) const MIN_SEGMENT: u64 = 64 * 1024;
pub(crate) const MAX_SEGMENT: u64 = 64 * 1024 * 1024;
/// A segment is a sixteenth of the limit, so a full cache is about 16 segments.
pub(crate) const SEGMENTS_PER_LIMIT: u64 = 16;
/// A record larger than a sixteenth of a segment is not stored. So a sealed segment is at
/// least 15/16 full, and the file count stays near the limit divided by the segment size.
const RECORDS_PER_SEGMENT: u64 = 16;
/// Index entries merged per lock acquisition while loading, so writers never wait long.
const LOAD_CHUNK: usize = 16 * 1024;
/// A miss during the index load waits for it until this long after startup. Loading 1 GiB
/// takes well under a second, and a signal is never held up for longer than this.
const LOAD_WAIT: std::time::Duration = std::time::Duration::from_millis(1000);

/// Bytes a file of `len` bytes may use on disk, rounded up, plus one block of overhead.
pub(crate) fn disk_size(len: u64) -> u64 {
    len.div_ceil(BLOCK) * BLOCK + BLOCK
}

/// Node hashes are uniformly random already, so the key is its own hash.
#[derive(Default)]
struct KeyHasher(u64);
impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(*byte);
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}
type Index = HashMap<u64, u64, BuildHasherDefault<KeyHasher>>;

fn key(hash: &H256) -> u64 {
    let mut prefix = [0; 8];
    prefix.copy_from_slice(&hash.as_bytes()[..8]);
    u64::from_le_bytes(prefix)
}
fn pack(segment: u32, offset: u32) -> u64 {
    (u64::from(segment) << 32) | u64::from(offset)
}
fn segment_of(location: u64) -> u32 {
    (location >> 32) as u32
}

struct Segment {
    id: u32,
    file: Arc<File>,
    len: u64,
}

/// An evicted segment whose file is not deleted yet. Its bytes stay in the total until the
/// file is gone.
struct Doomed {
    id: u32,
    bytes: u64,
    warned: bool,
}

struct State {
    index: Index,
    /// Oldest first. The newest is the one this process appends to, once it has opened one.
    segments: VecDeque<Segment>,
    writable: bool,
    /// The newest segment from an earlier run and the end of its last whole record, once
    /// the background load has read it. The first write continues that segment.
    resume: Option<(u32, u64)>,
    next_id: u32,
    doomed: Vec<Doomed>,
    /// `disk_size` of every segment and every doomed file.
    total: u64,
    budget: u64,
}
impl State {
    fn segment(&self, id: u32) -> Option<&Segment> {
        self.segments
            .binary_search_by_key(&id, |s| s.id)
            .ok()
            .map(|i| &self.segments[i])
    }
    /// Nodes in the older half of the cache are written again when a replay uses them, so
    /// eviction drops the nodes least recently used, not merely the least recently written.
    fn refresh_below(&self) -> u32 {
        if self.segments.len() < 4 {
            return 0;
        }
        self.segments[self.segments.len() / 2].id
    }
    /// Drops oldest segments until the live segments fit the budget. Their files are deleted
    /// by `NodeStore::delete_doomed`, outside the lock. Files still waiting for deletion stay
    /// in `total` but do not make more segments go: a file that cannot be deleted must not
    /// empty the cache.
    fn evict(&mut self) {
        let mut evicted = false;
        let mut doomed: u64 = self.doomed.iter().map(|d| d.bytes).sum();
        while self.total - doomed > self.budget {
            let Some(segment) = self.segments.pop_front() else {
                break;
            };
            if self.segments.is_empty() {
                self.writable = false;
            }
            let bytes = disk_size(segment.len);
            self.doomed.push(Doomed {
                id: segment.id,
                bytes,
                warned: false,
            });
            doomed += bytes;
            evicted = true;
        }
        if !evicted {
            return;
        }
        // Eviction always takes the oldest, so every entry below the oldest survivor is dead.
        match self.segments.front() {
            Some(first) => {
                let floor = first.id;
                self.index
                    .retain(|_, location| segment_of(*location) >= floor);
            }
            None => self.index = Index::default(),
        }
    }
    /// Sets a segment's length after a write, and its share of the total.
    fn resize(&mut self, at: usize, len: u64) {
        let old = disk_size(self.segments[at].len);
        self.segments[at].len = len;
        self.total = self.total - old + disk_size(len);
    }
}

fn segment_name(id: u32) -> String {
    format!("{id:010}.seg")
}
fn parse_name(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let id = name.strip_suffix(".seg")?;
    (id.len() == 10).then_some(())?;
    id.parse().ok()
}

struct Inner {
    dir: PathBuf,
    segment_size: u64,
    state: Mutex<State>,
    loaded: AtomicBool,
    /// Set when the last handle is dropped. The loader stops, and the write lock is released.
    closed: AtomicBool,
    opened: std::time::Instant,
    /// Nodes too large for a segment, or beyond a quarter of the budget in one batch.
    skipped: AtomicU64,
}

/// The handle sessions share. Dropping the last one releases this process's write lock at
/// once, even while the background load still runs.
pub(crate) struct NodeStore {
    inner: Arc<Inner>,
}
impl Drop for NodeStore {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let mut state = self.inner.state.lock();
        if state.writable
            && let Some(segment) = state.segments.back()
        {
            let _ = segment.file.unlock();
        }
        state.writable = false;
        state.resume = None;
    }
}

impl NodeStore {
    /// Lists existing segments and starts the background index load. Never reads node data.
    /// A segment that cannot be opened is skipped with a warning; only an unusable directory
    /// is an error.
    pub fn open(dir: &Path, budget: u64, legacy: Option<PathBuf>) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut segments = Vec::new();
        let mut next_id = 0;
        for entry in std::fs::read_dir(dir)? {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(e) => {
                    note!("warning: disk cache: {}: {e}", dir.display());
                    continue;
                }
            };
            let Some(id) = parse_name(&path) else {
                continue;
            };
            next_id = next_id.max(id.saturating_add(1));
            match File::open(&path).and_then(|file| Ok((file.metadata()?.len(), file))) {
                Ok((len, file)) => segments.push(Segment {
                    id,
                    file: Arc::new(file),
                    len,
                }),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => note!("warning: disk cache: skipping {}: {e}", path.display()),
            }
        }
        segments.sort_by_key(|s| s.id);
        let total = segments.iter().map(|s| disk_size(s.len)).sum();
        let store = Self {
            inner: Arc::new(Inner {
                dir: dir.to_owned(),
                segment_size: (budget / SEGMENTS_PER_LIMIT).clamp(MIN_SEGMENT, MAX_SEGMENT),
                state: Mutex::new(State {
                    index: Index::default(),
                    segments: segments.into(),
                    writable: false,
                    resume: None,
                    next_id,
                    doomed: Vec::new(),
                    total,
                    budget,
                }),
                loaded: AtomicBool::new(false),
                closed: AtomicBool::new(false),
                opened: std::time::Instant::now(),
                skipped: AtomicU64::new(0),
            }),
        };
        store.inner.state.lock().evict();
        store.delete_doomed();
        let inner = store.inner.clone();
        std::thread::Builder::new()
            .name("node-cache-load".into())
            .spawn(move || {
                hold_for_test(&inner);
                load(&inner);
                inner.loaded.store(true, Ordering::SeqCst);
                if let Some(legacy) = legacy {
                    remove_legacy(&inner, &legacy);
                }
            })?;
        Ok(store)
    }
    /// Whether the background load has indexed every segment that existed at startup.
    pub fn loaded(&self) -> bool {
        self.inner.loaded.load(Ordering::SeqCst)
    }
    /// Bytes the segment files use on disk, including evicted files not deleted yet.
    pub fn total(&self) -> u64 {
        self.inner.state.lock().total
    }
    pub fn set_budget(&self, budget: u64) {
        {
            let mut state = self.inner.state.lock();
            state.budget = budget;
            state.evict();
        }
        self.delete_doomed();
    }
    /// Deletes evicted files. A file that cannot be deleted stays counted and is tried again
    /// at the next write or maintenance; each is warned about once.
    fn delete_doomed(&self) {
        let ids: Vec<u32> = {
            let state = self.inner.state.lock();
            state.doomed.iter().map(|d| d.id).collect()
        };
        if ids.is_empty() {
            return;
        }
        let mut deleted = Vec::new();
        let mut failed = Vec::new();
        for id in ids {
            let path = self.inner.dir.join(segment_name(id));
            match std::fs::remove_file(&path) {
                Ok(()) => deleted.push(id),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => deleted.push(id),
                Err(e) => failed.push((id, path, e)),
            }
        }
        let mut warnings = Vec::new();
        {
            let mut state = self.inner.state.lock();
            let mut freed = 0;
            state.doomed.retain(|d| {
                let gone = deleted.contains(&d.id);
                if gone {
                    freed += d.bytes;
                }
                !gone
            });
            state.total -= freed;
            for (id, path, e) in failed {
                if let Some(doomed) = state.doomed.iter_mut().find(|d| d.id == id)
                    && !std::mem::replace(&mut doomed.warned, true)
                {
                    warnings.push(format!("warning: disk cache: {}: {e}", path.display()));
                }
            }
        }
        for warning in warnings {
            note!("{warning}");
        }
    }
    /// Reads and checks one node. Damaged or evicted records are misses.
    pub fn get(&self, hash: &H256) -> Option<Vec<u8>> {
        let (location, file) = loop {
            let loaded = self.loaded();
            let found = {
                let state = self.inner.state.lock();
                state.index.get(&key(hash)).and_then(|location| {
                    let file = state.segment(segment_of(*location))?.file.clone();
                    Some((*location, file))
                })
            };
            match found {
                Some(found) => break found,
                None if loaded || self.inner.opened.elapsed() >= LOAD_WAIT => return None,
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        };
        let node = read_record(&file, u64::from(location as u32), hash);
        if node.is_none() {
            let mut state = self.inner.state.lock();
            if state.index.get(&key(hash)) == Some(&location) {
                state.index.remove(&key(hash));
            }
        }
        node
    }
    /// Appends the nodes not stored yet, plus those in the older half that a replay used
    /// again. A node that missed while the index was still loading may be stored twice; the
    /// older copy goes with its segment.
    ///
    /// The batch is built outside the lock. The lock is held for the index lookups, and for
    /// the writes themselves, which must stay under it so offsets stay in order. A new file is
    /// created under it once per segment, and a torn tail is cut under it once per run.
    pub fn append<'a>(
        &self,
        nodes: impl IntoIterator<Item = (&'a H256, &'a [u8])>,
    ) -> std::io::Result<()> {
        let max_record = self.inner.segment_size / RECORDS_PER_SEGMENT;
        let (wanted, budget) = {
            let state = self.inner.state.lock();
            if state.budget == 0 {
                return Ok(());
            }
            let refresh_below = state.refresh_below();
            let wanted: Vec<_> = nodes
                .into_iter()
                .filter(|(hash, _)| {
                    state
                        .index
                        .get(&key(hash))
                        .is_none_or(|l| segment_of(*l) < refresh_below)
                })
                .collect();
            (wanted, state.budget)
        };
        // One batch never takes more than a quarter of the budget, so it cannot evict itself.
        let limit = budget / 4;
        let mut buffer = Vec::new();
        let mut records = Vec::new();
        let mut skipped = 0;
        for (hash, node) in wanted {
            let size = HEADER + node.len() as u64;
            if node.is_empty() || size > max_record || buffer.len() as u64 + size > limit {
                skipped += 1;
                continue;
            }
            records.push((key(hash), buffer.len(), size as usize));
            buffer.extend_from_slice(hash.as_bytes());
            buffer.extend_from_slice(&(node.len() as u32).to_le_bytes());
            buffer.extend_from_slice(node);
        }
        self.inner.skipped.fetch_add(skipped, Ordering::Relaxed);
        if records.is_empty() {
            return Ok(());
        }
        let result = self.write_records(&buffer, &records);
        self.inner.state.lock().evict();
        self.delete_doomed();
        result
    }
    /// Writes records at their boundaries, filling the current segment before opening the
    /// next. A failed write is cut back to the last whole record.
    fn write_records(&self, buffer: &[u8], records: &[(u64, usize, usize)]) -> std::io::Result<()> {
        let segment_size = self.inner.segment_size;
        let mut state = self.inner.state.lock();
        let mut next = 0;
        while next < records.len() {
            let first = records[next].2 as u64;
            if !state.writable
                && let Some((id, end)) = state.resume.take()
            {
                self.reopen(&mut state, id, end);
            }
            let room = |s: &Segment| s.len + first <= segment_size;
            if !state.writable || state.segments.back().is_none_or(|s| !room(s)) {
                self.create(&mut state)?;
            }
            let at = state.segments.len() - 1;
            let start = state.segments[at].len;
            let mut end = next;
            let mut len = 0u64;
            while end < records.len() && start + len + records[end].2 as u64 <= segment_size {
                len += records[end].2 as u64;
                end += 1;
            }
            let from = records[next].1;
            let bytes = &buffer[from..from + len as usize];
            let segment = &state.segments[at];
            let (id, file) = (segment.id, segment.file.clone());
            if let Err(e) = file.write_all_at(bytes, start) {
                // Cut the partial write off, so the next record starts at a record boundary.
                // If that fails too, count the file as it is and write to a new one.
                if file.set_len(start).is_err() {
                    let real = file.metadata().map_or(start + len, |m| m.len());
                    state.resize(at, real.max(start));
                    state.writable = false;
                }
                return Err(e);
            }
            state.resize(at, start + len);
            for &(key, offset, _) in &records[next..end] {
                let location = start + (offset - from) as u64;
                state.index.insert(key, pack(id, location as u32));
            }
            next = end;
        }
        Ok(())
    }
    /// Starts a new segment. A file whose header cannot be written is deleted, or counted
    /// and evicted later if it cannot be.
    fn create(&self, state: &mut State) -> std::io::Result<()> {
        // Another process using the same directory may have taken the next number.
        let (id, file) = loop {
            let id = state.next_id;
            state.next_id = id.saturating_add(1);
            match File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(self.inner.dir.join(segment_name(id)))
            {
                Ok(file) => break (id, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && id < u32::MAX => {}
                Err(e) => return Err(e),
            }
        };
        if state.writable
            && let Some(old) = state.segments.back()
        {
            let _ = old.file.unlock();
        }
        state.writable = false;
        // Held while the file is open, so no other process appends to it.
        let _ = file.try_lock();
        let header = file.write_all_at(MAGIC, 0);
        let len = match &header {
            Ok(()) => MAGIC.len() as u64,
            Err(_) => {
                let path = self.inner.dir.join(segment_name(id));
                if std::fs::remove_file(&path).is_ok() {
                    return header;
                }
                file.metadata().map_or(MAGIC.len() as u64, |m| m.len())
            }
        };
        state.total += disk_size(len);
        state.segments.push_back(Segment {
            id,
            file: Arc::new(file),
            len,
        });
        state.writable = header.is_ok();
        header
    }
    /// Continues the newest segment of an earlier run, cutting off a torn last record, unless
    /// another process has it open for writing.
    fn reopen(&self, state: &mut State, id: u32, end: u64) {
        let Some(at) = state
            .segments
            .back()
            .filter(|s| s.id == id)
            .map(|_| state.segments.len() - 1)
        else {
            return;
        };
        let Ok(file) = File::options()
            .read(true)
            .write(true)
            .open(self.inner.dir.join(segment_name(id)))
        else {
            return;
        };
        if file.try_lock().is_err() || (state.segments[at].len > end && file.set_len(end).is_err())
        {
            return;
        }
        state.segments[at].file = Arc::new(file);
        state.resize(at, end);
        state.writable = true;
    }
    #[cfg(test)]
    pub fn segment_count(&self) -> usize {
        self.inner.state.lock().segments.len()
    }
    /// Heap bytes of the index table: buckets of a 16-byte entry and one control byte.
    #[cfg(test)]
    pub fn index_bytes(&self) -> usize {
        let capacity = self.inner.state.lock().index.capacity();
        if capacity == 0 {
            return 0;
        }
        (capacity * 8 / 7).next_power_of_two() * 17
    }
    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        self.inner.state.lock().index.len()
    }
    #[cfg(test)]
    pub fn skipped(&self) -> u64 {
        self.inner.skipped.load(Ordering::Relaxed)
    }
    /// Panics after ten seconds, so a dead loader fails the test instead of hanging it.
    #[cfg(test)]
    pub fn wait_loaded(&self) {
        let start = std::time::Instant::now();
        while !self.loaded() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "disk cache index never finished loading"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

/// Tests of a signal during startup hold the background load while this file exists.
/// Debug builds only.
fn hold_for_test(inner: &Inner) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(path) = std::env::var_os("FRAME_TRACE_TEST_HOLD_CACHE_LOAD") else {
        return;
    };
    while Path::new(&path).exists() && !inner.closed.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The old cache wrote one file per node, named by its hash, through `.<hash>.<pid>.<n>.tmp`.
fn legacy_name(name: &str) -> bool {
    let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if hex(name) {
        return true;
    }
    let Some(rest) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".tmp")) else {
        return false;
    };
    let parts: Vec<_> = rest.split('.').collect();
    parts.len() == 3
        && hex(parts[0])
        && parts[1..]
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Deletes the files the old cache wrote, then the folder if nothing else is left in it.
/// Never follows a symlink, and never deletes anything it did not write.
fn remove_legacy(inner: &Inner, legacy: &Path) {
    if !std::fs::symlink_metadata(legacy).is_ok_and(|m| m.is_dir()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(legacy) else {
        return;
    };
    note!(
        "disk cache: removing {}, left by an older frame-trace; the disk cache is now in {}",
        legacy.display(),
        inner.dir.display()
    );
    let mut kept = false;
    for entry in entries {
        if inner.closed.load(Ordering::SeqCst) {
            return;
        }
        let Ok(entry) = entry else {
            kept = true;
            continue;
        };
        let ours = entry.file_type().is_ok_and(|t| t.is_file())
            && entry.file_name().to_str().is_some_and(legacy_name);
        if !ours || std::fs::remove_file(entry.path()).is_err() {
            kept = true;
        }
    }
    if kept || std::fs::remove_dir(legacy).is_err() {
        note!(
            "disk cache: kept {}: it holds files frame-trace did not write",
            legacy.display()
        );
    }
}

fn read_record(file: &File, offset: u64, hash: &H256) -> Option<Vec<u8>> {
    let mut header = [0; HEADER as usize];
    file.read_exact_at(&mut header, offset).ok()?;
    if header[..32] != hash.as_bytes()[..] {
        return None;
    }
    let len = u32::from_le_bytes(header[32..].try_into().ok()?) as usize;
    if len == 0 || len > MAX_NODE {
        return None;
    }
    let mut node = vec![0; len];
    file.read_exact_at(&mut node, offset + HEADER).ok()?;
    (H256::from(sp_io::hashing::blake2_256(&node)) == *hash).then_some(node)
}

/// Indexes the newest segment first, so the first write can continue it, then the rest from
/// oldest to newest. Where a node is in several segments, the newest copy wins whatever the
/// order. Stops when the store is closed. Entries for a segment evicted meanwhile are dropped.
fn load(inner: &Inner) {
    let segments: Vec<(u32, Arc<File>)> = {
        let mut state = inner.state.lock();
        // Sized once for records of about 200 bytes, the common trie node size. Growing
        // the table step by step would leave every smaller copy behind in the allocator.
        let records = state.segments.iter().map(|s| s.len).sum::<u64>() / 200;
        state.index.reserve(usize::try_from(records).unwrap_or(0));
        state
            .segments
            .iter()
            .map(|s| (s.id, s.file.clone()))
            .collect()
    };
    let newest = segments.len().saturating_sub(1);
    for (i, (id, file)) in segments
        .iter()
        .enumerate()
        .rev()
        .take(1)
        .chain(segments.iter().enumerate().take(newest))
    {
        let mut entries = Vec::new();
        let Some(end) = scan(file, *id, &mut |key, location| {
            entries.push((key, location));
            if entries.len() >= LOAD_CHUNK {
                return merge(inner, *id, &mut entries);
            }
            true
        }) else {
            return;
        };
        if !merge(inner, *id, &mut entries) {
            return;
        }
        if i == newest && end > 0 {
            inner.state.lock().resume = Some((*id, end));
        }
    }
}

/// Returns false once the store is closed.
fn merge(inner: &Inner, id: u32, entries: &mut Vec<(u64, u64)>) -> bool {
    if inner.closed.load(Ordering::SeqCst) {
        return false;
    }
    let mut state = inner.state.lock();
    if state.segment(id).is_some() {
        for (key, location) in entries.drain(..) {
            state
                .index
                .entry(key)
                .and_modify(|old| {
                    if segment_of(*old) <= id {
                        *old = location;
                    }
                })
                .or_insert(location);
        }
    }
    entries.clear();
    true
}

/// Reads record headers in order and stops at the first one that does not fit: a torn
/// tail. Returns the end of the last whole record (0 for a file that is not a segment), or
/// `None` when `found` asks to stop.
fn scan(file: &File, id: u32, found: &mut dyn FnMut(u64, u64) -> bool) -> Option<u64> {
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return Some(0);
    };
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut magic = [0; 8];
    if reader.read_exact(&mut magic).is_err() || &magic != MAGIC {
        return Some(0);
    }
    let mut offset = MAGIC.len() as u64;
    let mut header = [0; HEADER as usize];
    while offset + HEADER <= len && offset <= u64::from(u32::MAX) {
        if reader.read_exact(&mut header).is_err() {
            break;
        }
        let size = u64::from(u32::from_le_bytes(
            header[32..].try_into().unwrap_or_default(),
        ));
        if size == 0 || size > MAX_NODE as u64 || offset + HEADER + size > len {
            break;
        }
        if reader.seek_relative(size as i64).is_err() {
            break;
        }
        let hash = H256::from_slice(&header[..32]);
        if !found(key(&hash), pack(id, offset as u32)) {
            return None;
        }
        offset += HEADER + size;
    }
    Some(offset)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn node(i: u64, len: usize) -> (H256, Vec<u8>) {
        let seed = sp_io::hashing::blake2_256(&i.to_le_bytes());
        let bytes: Vec<u8> = seed.iter().cycle().take(len).copied().collect();
        (H256::from(sp_io::hashing::blake2_256(&bytes)), bytes)
    }
    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("frame-trace-disk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
    fn put(store: &NodeStore, nodes: &[(H256, Vec<u8>)]) {
        store
            .append(nodes.iter().map(|(h, n)| (h, n.as_slice())))
            .unwrap();
    }
    /// Blocks the file system really allocated, plus the per-file block the total counts.
    fn du(dir: &Path) -> u64 {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().blocks() * 512 + BLOCK)
            .sum()
    }
    fn files(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }
    fn wait_closed(weak: &std::sync::Weak<Inner>) {
        let start = std::time::Instant::now();
        while weak.upgrade().is_some() {
            assert!(start.elapsed() < std::time::Duration::from_secs(1));
            std::thread::yield_now();
        }
    }

    #[test]
    fn disk_usage_and_file_count_stay_within_the_cap() {
        let dir = dir("cap");
        let cap = 4 * 1024 * 1024;
        let store = NodeStore::open(&dir, cap, None).unwrap();
        let mut max_files = 0;
        for batch in 0..400u64 {
            let nodes: Vec<_> = (0..64).map(|i| node(batch * 64 + i, 200)).collect();
            put(&store, &nodes);
            assert!(store.total() <= cap, "{} > {cap}", store.total());
            assert!(
                du(&dir) <= store.total(),
                "{} > {}",
                du(&dir),
                store.total()
            );
            max_files = max_files.max(files(&dir));
        }
        // About 6 MB was written, more than the cap, so eviction ran.
        assert!(store.node_count() < 400 * 64);
        // 16 segments of a sixteenth of the cap, each at least 15/16 full, and one open.
        assert!(max_files <= 18, "{max_files} files");
        // The index holds at most one entry per record that fits in the cap.
        assert!(store.node_count() as u64 <= cap / (HEADER + 200));
        // Eviction stops once the cache fits: most of the cap is still in use.
        assert!(store.total() > cap / 2, "{} of {cap}", store.total());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn large_batches_are_split_at_record_boundaries_and_oversized_nodes_skipped() {
        let dir = dir("split");
        // 1 MiB segments, records up to 64 KiB, batches up to 4 MiB.
        let store = NodeStore::open(&dir, 16 * 1024 * 1024, None).unwrap();
        let nodes: Vec<_> = (0..1500).map(|i| node(i, 2000)).collect();
        put(&store, &nodes);
        assert_eq!(store.skipped(), 0);
        assert!(store.segment_count() >= 3);
        for entry in std::fs::read_dir(&dir).unwrap() {
            assert!(entry.unwrap().metadata().unwrap().len() <= 1024 * 1024);
        }
        for (hash, bytes) in &nodes {
            assert_eq!(store.get(hash).as_ref(), Some(bytes));
        }
        // A node larger than a sixteenth of a segment is not stored.
        let big = node(9999, 64 * 1024);
        put(&store, std::slice::from_ref(&big));
        assert_eq!(store.skipped(), 1);
        assert_eq!(store.get(&big.0), None);
        // A batch never takes more than a quarter of the budget.
        let many: Vec<_> = (2000..4500).map(|i| node(i, 2000)).collect();
        put(&store, &many);
        assert!(store.skipped() > 1);
        assert_eq!(store.get(&nodes[0].0).as_ref(), Some(&nodes[0].1));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn eviction_drops_oldest_segments_but_keeps_nodes_used_again() {
        let dir = dir("order");
        let store = NodeStore::open(&dir, 1024 * 1024, None).unwrap();
        let hot = node(u64::MAX, 300);
        let first = node(0, 300);
        put(&store, &[first.clone(), hot.clone()]);
        for batch in 1..2000u64 {
            let mut nodes: Vec<_> = (0..8).map(|i| node(batch * 8 + i, 300)).collect();
            // A replay uses the hot node in every batch.
            nodes.push(hot.clone());
            put(&store, &nodes);
        }
        assert_eq!(store.get(&first.0), None, "oldest node survived");
        assert_eq!(store.get(&hot.0), Some(hot.1.clone()));
        let newest = node(1999 * 8, 300);
        assert_eq!(store.get(&newest.0), Some(newest.1));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn evicted_files_that_cannot_be_deleted_stay_counted_until_they_are() {
        let dir = dir("undeletable");
        let store = NodeStore::open(&dir, 1024 * 1024, None).unwrap();
        put(&store, &(0..100).map(|i| node(i, 200)).collect::<Vec<_>>());
        let before = store.total();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        store.set_budget(0);
        assert_eq!(files(&dir), 1);
        assert_eq!(store.total(), before, "an undeleted file was forgotten");
        assert_eq!(store.node_count(), 0);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        store.set_budget(0);
        assert_eq!(files(&dir), 0);
        assert_eq!(store.total(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restart_reindexes_and_skips_a_torn_tail() {
        let dir = dir("torn");
        let nodes: Vec<_> = (0..100).map(|i| node(i, 120)).collect();
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        put(&store, &nodes);
        let weak = Arc::downgrade(&store.inner);
        drop(store);
        wait_closed(&weak);
        let segment = dir.join(segment_name(0));
        let full = std::fs::metadata(&segment).unwrap().len();
        // Cut the last record in half, as a crash during the write would.
        File::options()
            .write(true)
            .open(&segment)
            .unwrap()
            .set_len(full - 60)
            .unwrap();
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        store.wait_loaded();
        assert_eq!(store.node_count(), 99);
        assert_eq!(store.get(&nodes[98].0), Some(nodes[98].1.clone()));
        assert_eq!(store.get(&nodes[99].0), None);
        // The next write replaces the torn record, in the same file.
        put(&store, &[nodes[99].clone()]);
        assert_eq!(store.segment_count(), 1);
        assert_eq!(std::fs::metadata(&segment).unwrap().len(), full);
        drop(store);
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        store.wait_loaded();
        assert_eq!(store.node_count(), 100);
        assert_eq!(store.get(&nodes[99].0), Some(nodes[99].1.clone()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_corrupt_length_mid_file_costs_misses_never_wrong_bytes() {
        let dir = dir("length");
        let nodes: Vec<_> = (0..100).map(|i| node(i, 120)).collect();
        {
            let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
            put(&store, &nodes);
        }
        let segment = File::options()
            .write(true)
            .open(dir.join(segment_name(0)))
            .unwrap();
        let record = HEADER + 120;
        let length_of = |i: u64| MAGIC.len() as u64 + i * record + 32;
        // Larger than any node: the scan stops there.
        segment
            .write_all_at(&(MAX_NODE as u32 + 1).to_le_bytes(), length_of(50))
            .unwrap();
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        store.wait_loaded();
        assert_eq!(store.node_count(), 50);
        drop(store);
        // A plausible wrong length: the scan reads garbage headers after it.
        segment
            .write_all_at(&(120u32 + 7).to_le_bytes(), length_of(20))
            .unwrap();
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        store.wait_loaded();
        for (i, (hash, bytes)) in nodes.iter().enumerate() {
            match store.get(hash) {
                Some(found) => assert_eq!(&found, bytes, "node {i}"),
                None => assert!(i >= 20, "node {i} lost"),
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restarts_continue_the_newest_segment_unless_another_process_writes_it() {
        let dir = dir("resume");
        for run in 0..5u64 {
            let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
            store.wait_loaded();
            put(&store, &[node(run, 100)]);
        }
        assert_eq!(files(&dir), 1);
        let first = NodeStore::open(&dir, 1 << 30, None).unwrap();
        first.wait_loaded();
        put(&first, &[node(10, 100)]);
        let second = NodeStore::open(&dir, 1 << 30, None).unwrap();
        second.wait_loaded();
        put(&second, &[node(11, 100)]);
        assert_eq!(files(&dir), 2);
        drop((first, second));
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        store.wait_loaded();
        assert_eq!(store.node_count(), 7);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn damaged_records_are_misses() {
        let dir = dir("damaged");
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        let (hash, bytes) = node(1, 100);
        put(&store, &[(hash, bytes.clone())]);
        assert_eq!(store.get(&hash), Some(bytes.clone()));
        let segment = File::options()
            .write(true)
            .open(dir.join(segment_name(0)))
            .unwrap();
        segment
            .write_all_at(b"corrupt", MAGIC.len() as u64 + HEADER + 10)
            .unwrap();
        assert_eq!(store.get(&hash), None);
        assert_eq!(store.node_count(), 0);
        // The next replay that fetches it writes a good copy.
        put(&store, &[(hash, bytes.clone())]);
        assert_eq!(store.get(&hash), Some(bytes));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_segment_is_skipped_with_a_warning() {
        let dir = dir("unreadable");
        {
            let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
            put(&store, &[node(1, 100)]);
        }
        let bad = dir.join(segment_name(0));
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        store.wait_loaded();
        assert_eq!(store.segment_count(), 0);
        put(&store, &[node(2, 100)]);
        assert_eq!(store.get(&node(2, 100).0), Some(node(2, 100).1));
        assert!(
            dir.join(segment_name(1)).exists(),
            "reused a skipped file's number"
        );
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_writers_and_readers_agree() {
        let dir = dir("concurrent");
        let store = NodeStore::open(&dir, 8 * 1024 * 1024, None).unwrap();
        std::thread::scope(|scope| {
            for worker in 0..8u64 {
                let store = &store;
                scope.spawn(move || {
                    for batch in 0..100u64 {
                        let nodes: Vec<_> = (0..20)
                            .map(|i| node((worker * 100 + batch) * 20 + i, 150))
                            .collect();
                        put(store, &nodes);
                        // Shared nodes are written by every worker.
                        put(store, &[node(u64::MAX - batch, 150)]);
                        for (hash, bytes) in &nodes {
                            assert_eq!(store.get(hash).as_ref(), Some(bytes));
                        }
                    }
                });
            }
        });
        assert_eq!(store.node_count(), 8 * 100 * 20 + 100);
        drop(store);
        let store = NodeStore::open(&dir, 8 * 1024 * 1024, None).unwrap();
        store.wait_loaded();
        assert_eq!(store.node_count(), 8 * 100 * 20 + 100);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_layout_removal_deletes_only_old_cache_files_and_never_follows_symlinks() {
        let wait_gone = |path: &Path| {
            let start = std::time::Instant::now();
            while path.exists() && start.elapsed() < std::time::Duration::from_secs(10) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        let open = |root: &Path| {
            let store =
                NodeStore::open(&root.join("proof-nodes"), 1 << 30, Some(root.join("nodes")))
                    .unwrap();
            store.wait_loaded();
            store
        };
        // Only old cache files: the folder goes.
        let root = dir("legacy");
        let legacy = root.join("nodes");
        std::fs::create_dir_all(&legacy).unwrap();
        for i in 0..100 {
            std::fs::write(legacy.join(format!("{i:064x}")), [0; 32]).unwrap();
        }
        std::fs::write(legacy.join(format!(".{:064x}.123.4.tmp", 7)), [0; 32]).unwrap();
        let store = open(&root);
        wait_gone(&legacy);
        assert!(!legacy.exists());
        drop(store);
        // Someone else's files, a folder and symlinks: all kept, and their targets too.
        let outside = root.join("outside");
        std::fs::create_dir_all(outside.join("inner")).unwrap();
        std::fs::write(outside.join("target"), b"keep").unwrap();
        std::fs::write(outside.join(format!("{:064x}", 1)), b"keep").unwrap();
        std::fs::create_dir_all(legacy.join("sub")).unwrap();
        std::fs::write(legacy.join("notes.txt"), b"keep").unwrap();
        std::fs::write(legacy.join(format!("{:064x}", 2)), [0; 32]).unwrap();
        std::os::unix::fs::symlink(outside.join("target"), legacy.join(format!("{:064x}", 3)))
            .unwrap();
        let store = open(&root);
        wait_gone(&legacy.join(format!("{:064x}", 2)));
        drop(store);
        assert!(!legacy.join(format!("{:064x}", 2)).exists());
        assert!(legacy.join("notes.txt").exists() && legacy.join("sub").exists());
        assert!(
            legacy
                .join(format!("{:064x}", 3))
                .symlink_metadata()
                .is_ok()
        );
        assert_eq!(std::fs::read(outside.join("target")).unwrap(), b"keep");
        // `nodes` itself a symlink to a folder of hash-named files: untouched.
        std::fs::remove_dir_all(&legacy).unwrap();
        std::os::unix::fs::symlink(&outside, &legacy).unwrap();
        let store = open(&root);
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(store);
        assert!(outside.join(format!("{:064x}", 1)).exists());
        assert!(legacy.symlink_metadata().unwrap().file_type().is_symlink());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn open_does_not_read_nodes_and_load_stops_with_the_store() {
        let dir = dir("startup");
        {
            let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
            for batch in 0..200u64 {
                let nodes: Vec<_> = (0..500).map(|i| node(batch * 500 + i, 200)).collect();
                put(&store, &nodes);
            }
        }
        let start = std::time::Instant::now();
        let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
        let opened = start.elapsed();
        assert!(opened < std::time::Duration::from_millis(500), "{opened:?}");
        let weak = Arc::downgrade(&store.inner);
        drop(store);
        // The loader checks between chunks, so it lets go at once.
        wait_closed(&weak);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
