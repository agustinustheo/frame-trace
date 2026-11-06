//! Proof nodes on disk: append-only segment files, evicted whole and oldest first.
//!
//! A segment is `MAGIC` followed by records `[hash: 32][len: u32 LE][node bytes]`. Records are
//! never rewritten, so a crash can only leave a torn record at the end of the newest segment.
//! Every read checks the node's hash, so a damaged record is a miss, never a wrong node.
//!
//! The index maps the first 8 bytes of a node hash to its segment and offset (16 bytes per
//! node). A prefix collision only costs a miss, because the record header and the node hash
//! are both checked on read. Startup lists the segment files and returns; a background thread
//! scans them sequentially to rebuild the index, and lookups miss until it reaches them.
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
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

const MAGIC: &[u8; 8] = b"FTNODES1";
const HEADER: u64 = 36;
/// Larger nodes stay in memory only. Runtime code is the largest node and is far smaller.
pub(crate) const MAX_NODE: usize = 16 * 1024 * 1024;
/// File systems allocate whole blocks, and each file also costs an inode and a directory
/// entry. Counting one extra block per file keeps the total at or above real disk usage.
const BLOCK: u64 = 4096;
const MIN_SEGMENT: u64 = 64 * 1024;
const MAX_SEGMENT: u64 = 64 * 1024 * 1024;
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

struct State {
    index: Index,
    /// Oldest first. The newest is the one this process appends to, once it has opened one.
    segments: VecDeque<Segment>,
    writable: bool,
    /// The newest segment from an earlier run and the end of its last whole record, once
    /// the background load has read it. The first write continues that segment.
    resume: Option<(u32, u64)>,
    next_id: u32,
    /// Sum of `disk_size` over all segments.
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
    /// Drops oldest segments until the total fits the budget. Returns the files to delete.
    fn evict(&mut self) -> Vec<PathBuf> {
        let mut evicted = Vec::new();
        while self.total > self.budget {
            let Some(segment) = self.segments.pop_front() else {
                break;
            };
            if self.segments.is_empty() {
                self.writable = false;
            }
            self.total = self.total.saturating_sub(disk_size(segment.len));
            evicted.push(segment.id);
        }
        if evicted.is_empty() {
            return Vec::new();
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
        evicted
            .into_iter()
            .map(segment_name)
            .map(PathBuf::from)
            .collect()
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

pub(crate) struct NodeStore {
    dir: PathBuf,
    segment_size: u64,
    state: Mutex<State>,
    loaded: AtomicBool,
    opened: std::time::Instant,
}
impl NodeStore {
    /// Lists existing segments and starts the background index load. Never reads node data.
    pub fn open(dir: &Path, budget: u64, legacy: Option<PathBuf>) -> std::io::Result<Arc<Self>> {
        std::fs::create_dir_all(dir)?;
        let mut segments = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(id) = parse_name(&path) else {
                continue;
            };
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let len = file.metadata()?.len();
            segments.push(Segment {
                id,
                file: Arc::new(file),
                len,
            });
        }
        segments.sort_by_key(|s| s.id);
        let next_id = segments.last().map_or(0, |s| s.id.saturating_add(1));
        let total = segments.iter().map(|s| disk_size(s.len)).sum();
        let store = Arc::new(Self {
            dir: dir.to_owned(),
            segment_size: (budget / 16).clamp(MIN_SEGMENT, MAX_SEGMENT),
            state: Mutex::new(State {
                index: Index::default(),
                segments: segments.into(),
                writable: false,
                resume: None,
                next_id,
                total,
                budget,
            }),
            loaded: AtomicBool::new(false),
            opened: std::time::Instant::now(),
        });
        store.remove(store.state.lock().evict());
        let weak = Arc::downgrade(&store);
        let dir = dir.to_owned();
        std::thread::Builder::new()
            .name("node-cache-load".into())
            .spawn(move || {
                load(&weak);
                if let Some(store) = weak.upgrade() {
                    store.loaded.store(true, Ordering::SeqCst);
                }
                if let Some(legacy) = legacy.filter(|p| p.is_dir()) {
                    note!(
                        "cache: removing {}, left by an older frame-trace; fetched state is now kept in {}",
                        legacy.display(),
                        dir.display()
                    );
                    if let Err(e) = std::fs::remove_dir_all(&legacy) {
                        note!("warning: cache: {}: {e}", legacy.display());
                    }
                }
            })?;
        Ok(store)
    }
    /// Whether the background load has indexed every segment that existed at startup.
    pub fn loaded(&self) -> bool {
        self.loaded.load(Ordering::SeqCst)
    }
    /// Bytes the segment files may use on disk.
    pub fn total(&self) -> u64 {
        self.state.lock().total
    }
    pub fn set_budget(&self, budget: u64) {
        let evicted = {
            let mut state = self.state.lock();
            state.budget = budget;
            state.evict()
        };
        self.remove(evicted);
    }
    fn remove(&self, names: Vec<PathBuf>) {
        for name in names {
            match std::fs::remove_file(self.dir.join(&name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => note!("warning: cache: {}: {e}", self.dir.join(name).display()),
            }
        }
    }
    /// Reads and checks one node. Damaged or evicted records are misses.
    pub fn get(&self, hash: &H256) -> Option<Vec<u8>> {
        let (location, file) = loop {
            let loaded = self.loaded();
            let found = {
                let state = self.state.lock();
                state.index.get(&key(hash)).and_then(|location| {
                    let file = state.segment(segment_of(*location))?.file.clone();
                    Some((*location, file))
                })
            };
            match found {
                Some(found) => break found,
                None if loaded || self.opened.elapsed() >= LOAD_WAIT => return None,
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        };
        let node = read_record(&file, u64::from(location as u32), hash);
        if node.is_none() {
            let mut state = self.state.lock();
            if state.index.get(&key(hash)) == Some(&location) {
                state.index.remove(&key(hash));
            }
        }
        node
    }
    /// Appends the nodes not stored yet, plus those in the older half that a replay used
    /// again. One write call per batch; the lock covers only that call and the index update.
    pub fn append<'a>(
        &self,
        nodes: impl IntoIterator<Item = (&'a H256, &'a [u8])>,
    ) -> std::io::Result<()> {
        let mut state = self.state.lock();
        let refresh_below = state.refresh_below();
        let mut buffer = Vec::new();
        let mut keys = Vec::new();
        for (hash, node) in nodes {
            if node.is_empty() || node.len() > MAX_NODE {
                continue;
            }
            let key = key(hash);
            if state
                .index
                .get(&key)
                .is_some_and(|l| segment_of(*l) >= refresh_below)
            {
                continue;
            }
            keys.push((key, buffer.len() as u64));
            buffer.extend_from_slice(hash.as_bytes());
            buffer.extend_from_slice(&(node.len() as u32).to_le_bytes());
            buffer.extend_from_slice(node);
        }
        if buffer.is_empty() {
            return Ok(());
        }
        let full = |s: &Segment| {
            s.len > MAGIC.len() as u64
                && (s.len + buffer.len() as u64 > self.segment_size
                    || s.len + buffer.len() as u64 > u64::from(u32::MAX))
        };
        let resumed = !state.writable
            && state
                .resume
                .take()
                .is_some_and(|(id, end)| self.reopen(&mut state, id, end, buffer.len() as u64));
        if !resumed && (!state.writable || state.segments.back().is_none_or(full)) {
            // Another process using the same directory may have taken the next number.
            let (id, file) = loop {
                let id = state.next_id;
                state.next_id = id.saturating_add(1);
                match File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(self.dir.join(segment_name(id)))
                {
                    Ok(file) => break (id, file),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && id < u32::MAX => {}
                    Err(e) => return Err(e),
                }
            };
            // Held while the file is open, so no other process appends to it.
            let _ = file.try_lock();
            file.write_all_at(MAGIC, 0)?;
            state.total += disk_size(MAGIC.len() as u64);
            state.segments.push_back(Segment {
                id,
                file: Arc::new(file),
                len: MAGIC.len() as u64,
            });
            state.writable = true;
        }
        let Some(segment) = state.segments.back_mut() else {
            return Ok(());
        };
        let (id, start) = (segment.id, segment.len);
        if let Err(e) = segment.file.write_all_at(&buffer, start) {
            // Part of the batch may be on disk. Never append after it: a later scan would
            // stop at the torn record and miss everything behind it.
            state.writable = false;
            return Err(e);
        }
        let before = disk_size(start);
        segment.len = start + buffer.len() as u64;
        let after = disk_size(segment.len);
        state.total = state.total - before + after;
        for (key, offset) in keys {
            state.index.insert(key, pack(id, (start + offset) as u32));
        }
        let evicted = state.evict();
        drop(state);
        self.remove(evicted);
        Ok(())
    }
    /// Continues the newest segment of an earlier run, cutting off a torn last record, unless
    /// it is full or another process has it open for writing.
    fn reopen(&self, state: &mut State, id: u32, end: u64, extra: u64) -> bool {
        let segment_size = self.segment_size;
        let Some(segment) = state.segments.back_mut().filter(|s| s.id == id) else {
            return false;
        };
        if end + extra > segment_size || end + extra > u64::from(u32::MAX) {
            return false;
        }
        let Ok(file) = File::options()
            .read(true)
            .write(true)
            .open(self.dir.join(segment_name(id)))
        else {
            return false;
        };
        if file.try_lock().is_err() || (segment.len > end && file.set_len(end).is_err()) {
            return false;
        }
        let old = segment.len;
        segment.len = end;
        segment.file = Arc::new(file);
        state.total = state.total - disk_size(old) + disk_size(end);
        state.writable = true;
        true
    }
    #[cfg(test)]
    pub fn segment_count(&self) -> usize {
        self.state.lock().segments.len()
    }
    /// Heap bytes of the index table: 16-byte entries plus one control byte each.
    #[cfg(test)]
    pub fn index_bytes(&self) -> usize {
        self.state.lock().index.capacity() * 17
    }
    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        self.state.lock().index.len()
    }
    #[cfg(test)]
    pub fn wait_loaded(&self) {
        while !self.loaded() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
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

/// Indexes segments oldest first, so the newest copy of a node wins. Stops when the store
/// is dropped. Entries for a segment evicted meanwhile are discarded.
fn load(store: &Weak<NodeStore>) {
    let segments: Vec<(u32, Arc<File>)> = match store.upgrade() {
        Some(store) => {
            let mut state = store.state.lock();
            // Sized once for records of about 200 bytes, the common trie node size. Growing
            // the table step by step would leave every smaller copy behind in the allocator.
            let records = state.segments.iter().map(|s| s.len).sum::<u64>() / 200;
            state.index.reserve(usize::try_from(records).unwrap_or(0));
            state
                .segments
                .iter()
                .map(|s| (s.id, s.file.clone()))
                .collect()
        }
        None => return,
    };
    // The newest first: the first write after startup continues it.
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
                return merge(store, *id, &mut entries);
            }
            true
        }) else {
            return;
        };
        if !merge(store, *id, &mut entries) {
            return;
        }
        if i == newest
            && end > 0
            && let Some(store) = store.upgrade()
        {
            store.state.lock().resume = Some((*id, end));
        }
    }
}

/// Returns false once the store is gone.
fn merge(store: &Weak<NodeStore>, id: u32, entries: &mut Vec<(u64, u64)>) -> bool {
    let Some(store) = store.upgrade() else {
        return false;
    };
    let mut state = store.state.lock();
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
    fn du(dir: &Path) -> u64 {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| disk_size(e.unwrap().metadata().unwrap().len()))
            .sum()
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
            assert!(du(&dir) <= store.total());
            max_files = max_files.max(std::fs::read_dir(&dir).unwrap().count());
        }
        // 400 batches of ~15 KB is ~6 MB written: more than the cap, so eviction ran.
        assert!(store.node_count() < 400 * 64);
        // A 4 MiB cap uses 256 KiB segments: at most 16 full ones and the one being written.
        assert!(max_files <= 17, "{max_files} files");
        // The index holds at most one entry per record that fits in the cap.
        assert!(store.node_count() as u64 <= cap / (HEADER + 200));
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
    fn restart_reindexes_and_skips_a_torn_tail() {
        let dir = dir("torn");
        let nodes: Vec<_> = (0..100).map(|i| node(i, 120)).collect();
        {
            let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
            put(&store, &nodes);
        }
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
    fn restarts_continue_the_newest_segment_unless_another_process_writes_it() {
        let dir = dir("resume");
        for run in 0..5u64 {
            let store = NodeStore::open(&dir, 1 << 30, None).unwrap();
            store.wait_loaded();
            put(&store, &[node(run, 100)]);
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let first = NodeStore::open(&dir, 1 << 30, None).unwrap();
        first.wait_loaded();
        put(&first, &[node(10, 100)]);
        let second = NodeStore::open(&dir, 1 << 30, None).unwrap();
        second.wait_loaded();
        put(&second, &[node(11, 100)]);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
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
    fn old_layout_is_removed_in_the_background() {
        let dir = dir("legacy");
        let legacy = dir.join("nodes");
        std::fs::create_dir_all(&legacy).unwrap();
        for i in 0..100 {
            std::fs::write(legacy.join(format!("{i:064x}")), [0; 32]).unwrap();
        }
        let store =
            NodeStore::open(&dir.join("proof-nodes"), 1 << 30, Some(legacy.clone())).unwrap();
        let start = std::time::Instant::now();
        while legacy.exists() && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!legacy.exists());
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
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
        let weak = Arc::downgrade(&store);
        drop(store);
        // The loader holds the store only while merging a chunk, so it lets go at once.
        let start = std::time::Instant::now();
        while weak.upgrade().is_some() {
            assert!(start.elapsed() < std::time::Duration::from_secs(1));
            std::thread::yield_now();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
