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
    pub(crate) disk: Option<Arc<crate::disk::NodeStore>>,
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
    wasm: Arc<Mutex<WasmUsage>>,
    /// One worker at a time measures the Wasm cache; the others skip maintenance.
    maintenance: Arc<Mutex<()>>,
    /// The first failed node cache write is logged; later ones are not.
    pub(crate) warned_write: Arc<std::sync::atomic::AtomicBool>,
}
impl ReplaySession {
    /// `cache_dir` enables persistent proof nodes and Wasmtime artifacts. Without it, only
    /// compiled modules persist in the user's cache directory; proof nodes stay in memory.
    pub fn new(cache_dir: Option<&Path>) -> Result<Self> {
        Self::with_cache_max_bytes(cache_dir, 1024 * 1024 * 1024)
    }
    /// Bound the combined node/Wasm disk cache (default 1 GiB). Returns without reading the
    /// node cache: its index loads in the background, and lookups miss until it is ready.
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
        let disk = cache_dir
            .map(|p| {
                let dir = p.join("proof-nodes");
                crate::disk::NodeStore::open(&dir, cache_max_bytes, Some(p.join("nodes")))
                    .map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))
            })
            .transpose()?;
        let session = Self {
            nodes: Default::default(),
            hints: Default::default(),
            cold_start: Default::default(),
            disk,
            cache_base: base,
            cache_max_bytes,
            wasm: Default::default(),
            maintenance: Default::default(),
            warned_write: Default::default(),
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
        // Wasm eviction waits until authenticated :code identifies the active runtime.
        // Pruning here could delete the very module about to be loaded.
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
        self.wasm.lock().dirty = true;
        executor
    }
    /// Keeps the node and Wasm caches within `--cache-max-bytes`. Node totals are kept as
    /// they are written, so this reads no directory. The Wasm cache is measured again only
    /// after a new runtime was loaded, or every ten minutes because Wasmtime prunes it too.
    pub(crate) fn prune_disk_cache(&self) -> Result<()> {
        let Some(_maintenance) = self.maintenance.try_lock() else {
            return Ok(());
        };
        let due = {
            let wasm = self.wasm.lock();
            wasm.dirty
                || wasm
                    .measured
                    .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(600))
        };
        let mut result = Ok(());
        if due {
            result = self.prune_wasm();
        }
        let wasm_bytes = self.wasm.lock().bytes;
        if let Some(disk) = &self.disk {
            disk.set_budget(self.cache_max_bytes.saturating_sub(wasm_bytes));
        }
        result
    }
    fn prune_wasm(&self) -> Result<()> {
        let pinned: BTreeSet<String> = self
            .executors
            .lock()
            .keys()
            .map(|hash| format!("{hash:x}"))
            .collect();
        let root = self.cache_base.join("wasm");
        let mut modules =
            wasm_modules(&root).map_err(|e| Error::Invalid(format!("{}: {e}", root.display())))?;
        let mut bytes: u64 = modules.iter().map(|m| m.bytes).sum();
        let pinned_bytes: u64 = modules
            .iter()
            .filter(|m| pinned.contains(&m.name))
            .map(|m| m.bytes)
            .sum();
        let nodes = self.disk.as_ref().map_or(0, |d| d.total());
        let mut result = Ok(());
        // Without a known runtime every module may be the next one needed: measure only.
        if !pinned.is_empty() {
            modules.sort_by_key(|m| m.modified);
            for module in modules.iter().filter(|m| !pinned.contains(&m.name)) {
                // Runtimes not in use go before any proof node.
                if bytes + nodes <= self.cache_max_bytes {
                    break;
                }
                let path = root.join(&module.name);
                let removed = if module.directory {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                match removed {
                    Ok(()) => bytes = bytes.saturating_sub(module.bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        bytes = bytes.saturating_sub(module.bytes)
                    }
                    Err(e) => {
                        result = Err(Error::Invalid(format!("{}: {e}", path.display())));
                        break;
                    }
                }
            }
        }
        let mut wasm = self.wasm.lock();
        wasm.bytes = bytes;
        wasm.dirty = false;
        wasm.measured = Some(std::time::Instant::now());
        // Logged after the lock is released: a blocked log must not stall other replays.
        let warn =
            pinned_bytes > self.cache_max_bytes && !std::mem::replace(&mut wasm.warned_small, true);
        drop(wasm);
        if warn {
            note!(
                "warning: --cache-max-bytes {} is smaller than active runtime modules ({} bytes); keeping runtime code in use",
                self.cache_max_bytes,
                pinned_bytes
            );
        }
        result
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
struct WasmUsage {
    /// Bytes the Wasm cache used on disk when last measured.
    bytes: u64,
    measured: Option<std::time::Instant>,
    /// A runtime was loaded since the last measurement.
    dirty: bool,
    warned_small: bool,
}

/// One entry under `wasm/`: Wasmtime's cache for one runtime, or a stray file.
struct WasmModule {
    name: String,
    directory: bool,
    bytes: u64,
    modified: std::time::SystemTime,
}

/// Wasmtime keeps a few files per runtime, and eviction bounds the runtimes, so this walk is
/// small. It runs only when a runtime was loaded, or every ten minutes.
fn wasm_modules(root: &Path) -> std::io::Result<Vec<WasmModule>> {
    fn walk(
        path: &Path,
        bytes: &mut u64,
        modified: &mut std::time::SystemTime,
    ) -> std::io::Result<()> {
        let meta = match path.symlink_metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        *modified = (*modified).max(meta.modified()?);
        if meta.is_dir() {
            *bytes += crate::disk::disk_size(0);
            let entries = match std::fs::read_dir(path) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            for entry in entries {
                walk(&entry?.path(), bytes, modified)?;
            }
        } else {
            *bytes += crate::disk::disk_size(meta.len());
        }
        Ok(())
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut modules = Vec::new();
    for entry in entries {
        let entry = entry?;
        let (mut bytes, mut modified) = (0, std::time::SystemTime::UNIX_EPOCH);
        walk(&entry.path(), &mut bytes, &mut modified)?;
        modules.push(WasmModule {
            name: entry.file_name().to_string_lossy().into_owned(),
            directory: entry.file_type()?.is_dir(),
            bytes,
            modified,
        });
    }
    Ok(modules)
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
    fn wasm_cache_is_measured_when_runtimes_change_and_active_runtime_is_pinned() {
        let dir = std::env::temp_dir().join(format!("frame-trace-wasm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), 1).expect("session");
        let wasm = b"runtime";
        let active = session.executor_for(wasm);
        let hash = H256::from(sp_io::hashing::blake2_256(wasm));
        let module = dir.join("wasm").join(format!("{hash:x}")).join("module");
        let unused = dir.join("wasm").join("unused").join("module");
        for path in [&module, &unused] {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, [0; 32]).expect("write");
        }
        session.prune_disk_cache().expect("prune");
        assert!(module.exists());
        assert!(!unused.exists());
        assert!(session.wasm.lock().warned_small);
        // Until another runtime loads, maintenance reads no directory.
        let stray = dir.join("wasm").join("stray");
        std::fs::write(&stray, [0; 8]).expect("write");
        session.prune_disk_cache().expect("prune");
        assert!(stray.exists());
        drop(active);
        drop(session);
        let restarted = ReplaySession::with_cache_max_bytes(Some(&dir), 1).expect("restart");
        restarted.prune_disk_cache().expect("measure");
        assert!(
            module.exists() && stray.exists(),
            "eviction must wait until the runtime is identified"
        );
        let active = restarted.executor_for(wasm);
        restarted.prune_disk_cache().expect("restart prune");
        assert!(module.exists());
        assert!(!stray.exists());
        drop(active);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
    /// Cache maintenance at scale. Run with
    /// `FT_BENCH_DIR=<empty dir> cargo test --release -p frame-trace-replay --lib bench_disk_cache -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_disk_cache_1m_nodes() {
        use std::time::{Duration, Instant};
        fn rss_kib() -> u64 {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .expect("ps");
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .expect("rss")
        }
        fn node(i: u64) -> Vec<u8> {
            let seed = sp_io::hashing::blake2_256(&i.to_le_bytes());
            seed.iter().cycle().take(200).copied().collect()
        }
        let n: u64 = std::env::var("FT_BENCH_NODES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_000_000);
        let dir = PathBuf::from(std::env::var("FT_BENCH_DIR").expect("FT_BENCH_DIR"));
        if !dir.join("proof-nodes").exists() {
            let session =
                ReplaySession::with_cache_max_bytes(Some(&dir), 1 << 30).expect("session");
            let start = Instant::now();
            for block in 0..n / 1000 {
                let db = crate::proof::ProofDb::with_session(None, H256::zero(), session.clone());
                db.insert_nodes((block * 1000..block * 1000 + 1000).map(node));
                db.persist().expect("persist");
            }
            eprintln!("populate {n} nodes: {:?}", start.elapsed());
        }
        // The index alone, without the rest of a session.
        let rss = rss_kib();
        let store =
            crate::disk::NodeStore::open(&dir.join("proof-nodes"), 1 << 30, None).expect("store");
        store.wait_loaded();
        let index_rss = rss_kib().saturating_sub(rss) / 1024;
        drop(store);
        let start = Instant::now();
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), 1 << 30).expect("session");
        let startup = start.elapsed();
        let disk = session.disk.clone().expect("disk");
        disk.wait_loaded();
        let loaded = start.elapsed();
        eprintln!(
            "startup {startup:?}; index loaded in the background after {loaded:?}; {} nodes; index RSS {} MiB (table {} MiB); {} segment files",
            disk.node_count(),
            index_rss,
            disk.index_bytes() / (1024 * 1024),
            disk.segment_count()
        );
        let (mut persist, mut maintenance) = (Vec::new(), Vec::new());
        for r in 0..20u64 {
            let db = crate::proof::ProofDb::with_session(None, H256::zero(), session.clone());
            db.insert_nodes((0..50).map(|j| node(n + 1_000_000 + r * 50 + j)));
            let start = Instant::now();
            db.persist().expect("persist");
            persist.push(start.elapsed());
            let start = Instant::now();
            session.prune_disk_cache().expect("prune");
            maintenance.push(start.elapsed());
        }
        let median = |v: &mut Vec<Duration>| {
            v.sort();
            (v[v.len() / 2], v[v.len() - 1])
        };
        eprintln!(
            "persist per replay (median, max): {:?}; maintenance per replay: {:?}",
            median(&mut persist),
            median(&mut maintenance)
        );
        // The rare expensive step: dropping the oldest segment walks the index once.
        let (before, nodes) = (disk.total(), disk.node_count());
        let start = Instant::now();
        disk.set_budget(before - 1);
        eprintln!(
            "evicting one segment: {:?}, {} of {nodes} index entries removed",
            start.elapsed(),
            nodes - disk.node_count()
        );
    }
}
