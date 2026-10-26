//! Reusable, bounded acquisition and compiled-runtime caches. Each replay authenticates its own root.
use crate::{Error, Executor, Result};
use parking_lot::Mutex;
use sp_core::H256;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// Metadata dependencies are authenticated again at each new state root. Pure metadata
/// exports have an empty dependency list and reuse solely by code hash.
type MetadataRead = (Vec<u8>, Option<Vec<u8>>, Option<H256>);
#[derive(Clone)]
pub(crate) struct MetadataEntry {
    pub metadata: String,
    pub reads: Vec<MetadataRead>,
    /// Iteration and whole-state/child-root operations bind reuse to this root.
    pub iteration_root: Option<H256>,
    pub proof_nodes: Vec<Vec<u8>>,
}
type MetadataCache = BTreeMap<H256, MetadataEntry>;

const MAX_SHARED_BYTES: usize = 256 * 1024 * 1024;
#[derive(Default)]
pub(crate) struct Nodes {
    pub nodes: BTreeMap<H256, Arc<[u8]>>,
    bytes: usize,
}
impl Nodes {
    pub fn insert(&mut self, hash: H256, bytes: Arc<[u8]>) {
        if self.nodes.contains_key(&hash) {
            return;
        }
        if self.bytes + bytes.len() > MAX_SHARED_BYTES {
            self.nodes.clear();
            self.bytes = 0;
        }
        self.bytes += bytes.len();
        self.nodes.insert(hash, bytes);
    }
}
/// Keep one session across blocks. SDK compilation is keyed by authenticated code hash and heap settings.
#[derive(Clone)]
pub struct ReplaySession {
    pub(crate) nodes: Arc<Mutex<Nodes>>,
    pub(crate) hints: Arc<Mutex<VecDeque<BTreeSet<Vec<u8>>>>>,
    // One cold replay teaches the shared proof cache and key predictions before followers
    // start. Otherwise every startup worker repeats the same sequential discovery rounds.
    cold_start: Arc<Mutex<()>>,
    #[cfg(test)]
    pub(crate) executor: Arc<Executor>,
    executors: Arc<Mutex<BTreeMap<H256, Arc<Executor>>>>,
    pub(crate) node_dir: Option<PathBuf>,
    pub(crate) speculative_discovery: bool,
    pub(crate) metadata: Arc<Mutex<MetadataCache>>,
    pub(crate) rpc_metadata_fallback: bool,
    pub(crate) runtime_override: Option<Arc<Vec<u8>>>,
    /// The last announced override, so repeated loads of one context print once.
    pub(crate) announced_override: Arc<Mutex<Option<crate::RuntimeOverride>>>,
    #[cfg(test)]
    pub(crate) metadata_runs: Arc<std::sync::atomic::AtomicUsize>,
    cache_base: PathBuf,
    cache_max_bytes: u64,
    disk_index: Arc<Mutex<DiskIndex>>,
}
impl ReplaySession {
    /// `cache_dir` enables persistent proof nodes and Wasmtime artifacts. Without it, only
    /// compiled modules persist in the user's cache directory; proof nodes stay in memory.
    pub fn new(cache_dir: Option<&Path>) -> Result<Self> {
        Self::with_cache_max_bytes(cache_dir, 1024 * 1024 * 1024)
    }
    /// Bound the combined node/Wasm disk cache (default 1 GiB), evicting oldest mtimes.
    pub fn with_cache_max_bytes(cache_dir: Option<&Path>, cache_max_bytes: u64) -> Result<Self> {
        let base = cache_dir.map(Path::to_path_buf).unwrap_or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
                .unwrap_or_else(std::env::temp_dir)
                .join("frame-trace")
        });
        let wasm_dir = base.join("wasm");
        std::fs::create_dir_all(&wasm_dir)
            .map_err(|e| Error::Invalid(format!("{}: {e}", wasm_dir.display())))?;
        let node_dir = cache_dir.map(|p| p.join("nodes"));
        if let Some(dir) = &node_dir {
            std::fs::create_dir_all(dir)
                .map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?;
        }
        let session = Self {
            nodes: Default::default(),
            hints: Default::default(),
            cold_start: Default::default(),
            node_dir,
            cache_base: base,
            cache_max_bytes,
            disk_index: Default::default(),
            executors: Default::default(),
            speculative_discovery: true,
            metadata: Default::default(),
            rpc_metadata_fallback: false,
            runtime_override: None,
            announced_override: Default::default(),
            #[cfg(test)]
            metadata_runs: Default::default(),
            #[cfg(test)]
            executor: Arc::new(
                Executor::builder()
                    .with_max_runtime_instances(2)
                    .with_runtime_cache_size(4)
                    .with_allow_missing_host_functions(false)
                    .with_cache_path(wasm_dir)
                    .build(),
            ),
        };
        // Learn sizes once, but defer eviction until authenticated :code identifies the
        // active runtime. Pruning here could delete the very module about to be loaded.
        session.refresh_disk_index()?;
        Ok(session)
    }
    /// Admit one cold replay at a time until successful replay paths are available.
    /// `follow` holds the returned guard through replay; other callers need not use this gate.
    pub fn cold_start(
        &self,
        cancel: &crate::rpc::CancellationToken,
    ) -> Result<Option<parking_lot::MutexGuard<'_, ()>>> {
        self.cold_start_wait(cancel, || {})
    }
    fn cold_start_wait(
        &self,
        cancel: &crate::rpc::CancellationToken,
        waiting: impl FnOnce(),
    ) -> Result<Option<parking_lot::MutexGuard<'_, ()>>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if !self.hints.lock().is_empty() {
            return if cancel.is_cancelled() {
                Err(Error::Cancelled)
            } else {
                Ok(None)
            };
        }
        let mut waiting = Some(waiting);
        loop {
            if let Some(guard) = self
                .cold_start
                .try_lock_for(std::time::Duration::from_millis(25))
            {
                if cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                // A replay may have finished while this worker waited.
                return Ok(self.hints.lock().is_empty().then_some(guard));
            }
            if let Some(waiting) = waiting.take() {
                waiting();
            }
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
        }
    }
    /// Full storage keys and trie paths from the last three successful replays.
    pub(crate) fn predicted_keys(&self) -> BTreeSet<Vec<u8>> {
        self.hints
            .lock()
            .iter()
            .rev()
            .flatten()
            .take(4096)
            .cloned()
            .collect()
    }
    pub(crate) fn remember_paths(&self, paths: BTreeSet<Vec<u8>>) {
        let mut history = self.hints.lock();
        history.push_back(paths);
        while history.len() > 3 {
            history.pop_front();
        }
    }
    /// Explicitly permit unauthenticated RPC/stored metadata only if local execution fails.
    pub fn with_rpc_metadata_fallback(mut self, enabled: bool) -> Self {
        self.rpc_metadata_fallback = enabled;
        self
    }
    /// Execute this Wasm instead of the state's authenticated `:code`. This deliberately
    /// bypasses the code-hash binding to the parent state; every replay reports both hashes.
    /// State proofs and the block's storage-root assertion still apply.
    pub fn with_runtime_override(mut self, wasm: Option<Vec<u8>>) -> Self {
        self.runtime_override = wasm.map(Arc::new);
        self
    }
    /// Disable throw-away speculative passes when debugging strict execution-driven paging.
    pub fn with_speculative_discovery(mut self, enabled: bool) -> Self {
        self.speculative_discovery = enabled;
        self
    }
    pub(crate) fn executor_for(&self, wasm: &[u8]) -> Arc<Executor> {
        let hash = H256::from(sp_io::hashing::blake2_256(wasm));
        let mut executors = self.executors.lock();
        if let Some(executor) = executors.get(&hash) {
            return executor.clone();
        }
        if executors.len() >= 4
            && let Some(old) = executors
                .iter()
                .find(|(_, e)| Arc::strong_count(e) == 1)
                .map(|(h, _)| *h)
        {
            executors.remove(&old);
        }
        let executor = Arc::new(
            Executor::builder()
                .with_max_runtime_instances(2)
                .with_runtime_cache_size(4)
                .with_allow_missing_host_functions(false)
                .with_cache_path(self.cache_base.join("wasm").join(format!("{hash:x}")))
                .build(),
        );
        executors.insert(hash, executor.clone());
        executor
    }
    pub(crate) fn index_write(&self, path: PathBuf, size: u64) {
        self.disk_index
            .lock()
            .insert(path, std::time::SystemTime::now(), size);
    }
    fn refresh_disk_index(&self) -> Result<()> {
        let mut index = self.disk_index.lock();
        for dir in [self.node_dir.clone(), Some(self.cache_base.join("wasm"))]
            .into_iter()
            .flatten()
        {
            index
                .refresh(&dir)
                .map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?;
        }
        Ok(())
    }
    pub(crate) fn prune_disk_cache(&self) -> Result<()> {
        // Entries are immutable content-addressed files. Only new directory entries need stat;
        // proof persistence registers writes directly. Wasmtime writes are discovered by mtime.
        let pinned: Vec<_> = self
            .executors
            .lock()
            .keys()
            .map(|hash| self.cache_base.join("wasm").join(format!("{hash:x}")))
            .collect();
        self.refresh_disk_index()?;
        let mut index = self.disk_index.lock();
        let pinned_bytes: u64 = index
            .files
            .iter()
            .filter(|(path, _)| pinned.iter().any(|p| path.starts_with(p)))
            .map(|(_, (_, size))| size)
            .sum();
        if pinned_bytes > self.cache_max_bytes && !index.warned_small {
            eprintln!(
                "warning: --cache-max-bytes {} is smaller than active runtime modules ({} bytes); keeping runtime code in use",
                self.cache_max_bytes, pinned_bytes
            );
            index.warned_small = true;
        }
        while index.total > self.cache_max_bytes {
            let Some((time, path)) = index
                .oldest
                .iter()
                .find(|(_, path)| !pinned.iter().any(|p| path.starts_with(p)))
                .cloned()
            else {
                break;
            };
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::Invalid(format!("{}: {e}", path.display()))),
            }
            if let Some((_, size)) = index.files.remove(&path) {
                index.total = index.total.saturating_sub(size);
            }
            index.oldest.remove(&(time, path));
        }
        Ok(())
    }
    pub(crate) fn shared() -> Result<&'static Self> {
        static SESSION: OnceLock<std::result::Result<ReplaySession, String>> = OnceLock::new();
        SESSION
            .get_or_init(|| Self::new(None).map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::Invalid(e.clone()))
    }
}

#[derive(Default)]
struct DiskIndex {
    files: BTreeMap<PathBuf, (std::time::SystemTime, u64)>,
    oldest: BTreeSet<(std::time::SystemTime, PathBuf)>,
    directories: BTreeMap<PathBuf, std::time::SystemTime>,
    total: u64,
    warned_small: bool,
    #[cfg(test)]
    file_stats: usize,
}
impl DiskIndex {
    fn insert(&mut self, path: PathBuf, time: std::time::SystemTime, size: u64) {
        if let Some((old, size)) = self.files.insert(path.clone(), (time, size)) {
            self.total = self.total.saturating_sub(size);
            self.oldest.remove(&(old, path.clone()));
        }
        self.total += size;
        self.oldest.insert((time, path));
    }
    fn refresh(&mut self, dir: &Path) -> std::io::Result<()> {
        let modified = match std::fs::metadata(dir) {
            Ok(meta) => meta.modified()?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let changed = self.directories.get(dir) != Some(&modified);
        if changed {
            self.directories.insert(dir.to_owned(), modified);
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                // Another session may evict a directory between metadata and read_dir.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.directories.remove(dir);
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if self.files.contains_key(&path) || self.directories.contains_key(&path) {
                    continue;
                }
                let meta = match path.symlink_metadata() {
                    Ok(meta) => meta,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                #[cfg(test)]
                {
                    self.file_stats += usize::from(meta.is_file());
                }
                if meta.is_dir() {
                    self.refresh(&path)?;
                } else if meta.is_file() {
                    let time = meta.modified()?;
                    if path.extension().is_some_and(|ext| ext == "tmp")
                        && time.elapsed().unwrap_or_default() < std::time::Duration::from_secs(300)
                    {
                        self.directories.remove(dir);
                    } else {
                        self.insert(path, time, meta.len());
                    }
                }
            }
        }
        let children: Vec<_> = self
            .directories
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect();
        for child in children {
            self.refresh(&child)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;
    #[test]
    fn cold_followers_wait_until_first_replay_releases_guard() {
        use std::{sync::mpsc, time::Duration};
        let session = ReplaySession::new(None).expect("session");
        let cancel = crate::rpc::CancellationToken::new();
        let guard = session
            .cold_start(&cancel)
            .expect("admission")
            .expect("cold");
        std::thread::scope(|scope| {
            let (waiting, ready) = mpsc::channel();
            let (done, finished) = mpsc::channel();
            let (session, cancel) = (&session, &cancel);
            let next = scope.spawn(move || {
                assert!(
                    session
                        .cold_start_wait(cancel, || waiting.send(()).expect("waiting"))
                        .expect("admission")
                        .is_none()
                );
                assert!(session.predicted_keys().contains(b"observed".as_slice()));
                done.send(()).expect("done");
            });
            ready
                .recv_timeout(Duration::from_secs(2))
                .expect("contended lock");
            session.remember_paths([b"observed".to_vec()].into_iter().collect());
            assert!(matches!(
                finished.recv_timeout(Duration::from_millis(75)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            drop(guard);
            finished
                .recv_timeout(Duration::from_secs(2))
                .expect("released");
            next.join().expect("follower completed");
        });
        assert!(session.cold_start(&cancel).expect("warm").is_none());
    }
    #[test]
    fn online_error_releases_explicit_startup_admission() {
        let session = ReplaySession::new(None).expect("session");
        let cancel = crate::rpc::CancellationToken::new();
        let rpc = crate::rpc::Rpc::new("http://127.0.0.1:1").expect("rpc");
        let replay = || -> Result<()> {
            let _guard = session.cold_start(&cancel)?;
            crate::online_with_session(&rpc, "invalid-selector", &session)?;
            Ok(())
        };
        assert!(replay().is_err());
        assert_eq!(rpc.count(), 0);
        assert!(session.cold_start.try_lock().is_some());
        assert!(session.cold_start(&cancel).expect("retry").is_some());
    }
    #[test]
    fn panic_releases_startup_admission() {
        let session = ReplaySession::new(None).expect("session");
        let cancel = crate::rpc::CancellationToken::new();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = session.cold_start(&cancel).expect("admission");
                panic!("replay panicked");
            }))
            .is_err()
        );
        assert!(session.cold_start.try_lock().is_some());
    }
    #[test]
    fn cancelled_waiter_exits_while_first_replay_still_holds_gate() {
        use std::{sync::mpsc, time::Duration};
        let session = ReplaySession::new(None).expect("session");
        let cancel = crate::rpc::CancellationToken::new();
        let _guard = session.cold_start(&cancel).expect("admission");
        std::thread::scope(|scope| {
            let (waiting, ready) = mpsc::channel();
            let (done, finished) = mpsc::channel();
            let (session, cancel) = (&session, &cancel);
            scope.spawn(move || {
                assert!(matches!(
                    session.cold_start_wait(cancel, || waiting.send(()).expect("waiting")),
                    Err(Error::Cancelled)
                ));
                done.send(()).expect("done");
            });
            ready
                .recv_timeout(Duration::from_secs(2))
                .expect("contended lock");
            cancel.cancel();
            finished
                .recv_timeout(Duration::from_secs(2))
                .expect("cancelled without gate release");
        });
        assert!(matches!(session.cold_start(&cancel), Err(Error::Cancelled)));
        let rpc =
            crate::rpc::Rpc::with_options("http://127.0.0.1:1", 1, Duration::from_secs(1), cancel)
                .expect("rpc");
        assert!(matches!(
            crate::online_with_session(&rpc, "best", &session),
            Err(Error::Cancelled)
        ));
        assert_eq!(rpc.count(), 0);
    }
    #[test]
    fn unchanged_files_are_not_statted_again_and_active_runtime_is_pinned() {
        let dir = std::env::temp_dir().join(format!("frame-trace-index-{}", std::process::id()));
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), 1).expect("session");
        let wasm = b"runtime";
        let active = session.executor_for(wasm);
        let hash = H256::from(sp_io::hashing::blake2_256(wasm));
        let module = dir.join("wasm").join(format!("{hash:x}")).join("module");
        std::fs::create_dir_all(module.parent().expect("parent")).expect("mkdir");
        std::fs::write(&module, [0; 32]).expect("write");
        session.prune_disk_cache().expect("prune");
        let stats = session.disk_index.lock().file_stats;
        session.prune_disk_cache().expect("prune");
        assert_eq!(stats, session.disk_index.lock().file_stats);
        assert!(module.exists());
        assert!(session.disk_index.lock().warned_small);
        drop(active);
        drop(session);
        let restarted = ReplaySession::with_cache_max_bytes(Some(&dir), 1).expect("restart");
        assert!(
            module.exists(),
            "startup must defer eviction until the runtime is identified"
        );
        let active = restarted.executor_for(wasm);
        restarted.prune_disk_cache().expect("restart prune");
        assert!(module.exists());
        drop(active);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}
