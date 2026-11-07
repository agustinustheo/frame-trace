//! Reusable, bounded acquisition and compiled-runtime caches. Each replay authenticates its own root.
use crate::{Error, Executor, Result, nodes::Nodes};
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
pub(crate) struct MetadataEntry {
    pub metadata: String,
    pub reads: Vec<MetadataRead>,
    /// Iteration and whole-state/child-root operations bind reuse to this root.
    pub iteration_root: Option<H256>,
    pub proof_nodes: Vec<Vec<u8>>,
}
/// Shared: a hit copies only the metadata text into the block's artifact.
type MetadataCache = BTreeMap<H256, Arc<MetadataEntry>>;

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
    /// Counts runtimes loaded, so maintenance knows to measure the Wasm cache again.
    wasm_generation: Arc<std::sync::atomic::AtomicU64>,
    /// False when the Wasm cache directory could not be created: runtimes compile each run.
    wasm_cache: bool,
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
        // The disk cache is disposable: a directory that cannot be used, or that another user
        // could write to, is a warning, and polkadot-frame-trace runs without it. Wasmtime loads
        // native code from `wasm/`, so a directory others can write would let them run code here.
        let (base, wasm_cache, disk) =
            match private_cache(cache_dir) {
                Ok(base) => {
                    let wasm_cache = match private_dir(&base.join("wasm")) {
                        Ok(()) => true,
                        Err(e) => {
                            note!("warning: disk cache: {e}; runtimes are compiled again each run");
                            false
                        }
                    };
                    let disk = cache_dir.map(|_| base.join("proof-nodes")).and_then(|dir| {
                        match private_dir(&dir).and_then(|()| {
                            crate::disk::NodeStore::open(
                                &dir,
                                cache_max_bytes,
                                Some(base.join("nodes")),
                            )
                            .map_err(|e| format!("{}: {e}", dir.display()))
                        }) {
                            Ok(store) => Some(Arc::new(store)),
                            Err(e) => {
                                note!("warning: disk cache: {e}; running without it");
                                None
                            }
                        }
                    });
                    (base, wasm_cache, disk)
                }
                Err(e) => {
                    note!("warning: disk cache: {e}; running without it");
                    (PathBuf::new(), false, None)
                }
            };
        #[cfg(test)]
        let wasm_dir = wasm_cache.then(|| base.join("wasm"));
        let session = Self {
            nodes: Default::default(),
            hints: Default::default(),
            cold_start: Default::default(),
            disk,
            cache_base: base,
            cache_max_bytes,
            wasm: Default::default(),
            wasm_generation: Default::default(),
            wasm_cache,
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
            executor: Arc::new({
                let builder = Executor::builder()
                    .with_max_runtime_instances(2)
                    .with_runtime_cache_size(4)
                    .with_allow_missing_host_functions(false);
                match wasm_dir {
                    Some(dir) => builder.with_cache_path(dir).build(),
                    None => builder.build(),
                }
            }),
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
        // Runtimes in use stay. Once they are idle again, the next new runtime drops as many as
        // it takes to get back to four.
        while executors.len() >= 4
            && let Some(old) = executors
                .iter()
                .find(|(_, e)| Arc::strong_count(e) == 1)
                .map(|(h, _)| *h)
        {
            executors.remove(&old);
        }
        let builder = Executor::builder()
            .with_max_runtime_instances(2)
            .with_runtime_cache_size(4)
            .with_allow_missing_host_functions(false);
        let builder = if self.wasm_cache {
            builder.with_cache_path(self.cache_base.join("wasm").join(format!("{hash:x}")))
        } else {
            builder
        };
        let executor = Arc::new(builder.build());
        executors.insert(hash, executor.clone());
        self.wasm_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        executor
    }
    /// Keeps the node and Wasm caches within `--cache-max-bytes`. Node totals are kept as
    /// they are written, so this reads no directory. The Wasm cache is measured again only
    /// after a new runtime was loaded, or every ten minutes because Wasmtime prunes it too.
    ///
    /// Runtimes not in use are deleted before any proof node. The node budget is the limit
    /// less the runtimes in use, but never below `min(64 MiB, limit / 4)`, so a runtime larger
    /// than the limit does not empty the node cache at every block.
    pub(crate) fn prune_disk_cache(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        let Some(_maintenance) = self.maintenance.try_lock() else {
            return Ok(());
        };
        let root = self.cache_base.join("wasm");
        let generation = self.wasm_generation.load(Ordering::SeqCst);
        // Without a Wasm cache folder there is nothing to measure or delete.
        let due = self.wasm_cache && {
            let wasm = self.wasm.lock();
            wasm.generation != Some(generation)
                || wasm
                    .measured
                    .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(600))
        };
        let mut result = Ok(());
        if due {
            match wasm_modules(&root) {
                Ok(modules) => {
                    let mut wasm = self.wasm.lock();
                    wasm.modules = modules;
                    wasm.generation = Some(generation);
                    wasm.measured = Some(std::time::Instant::now());
                }
                Err(e) => result = Err(Error::Invalid(format!("{}: {e}", root.display()))),
            }
        }
        let mut modules = std::mem::take(&mut self.wasm.lock().modules);
        let nodes = self.disk.as_ref().map_or(0, |d| d.total());
        // Deletes below `wasm/` only while it is a real directory, never through a symlink.
        let mut failed = (self.wasm_cache
            && std::fs::symlink_metadata(&root).is_ok_and(|m| !m.file_type().is_dir()))
        .then(|| format!("{}: is not a directory", root.display()));
        let pinned_bytes = {
            // Held while deleting, so no runtime can be loaded from a folder being deleted.
            let executors = self.executors.lock();
            let pinned: BTreeSet<String> = executors.keys().map(|h| format!("{h:x}")).collect();
            // Without a known runtime every module may be the next one needed: measure only.
            if !pinned.is_empty() {
                modules.sort_by_key(|m| m.modified);
                let mut bytes: u64 = modules.iter().map(|m| m.bytes).sum();
                let mut kept = Vec::new();
                for module in modules {
                    if bytes + nodes <= self.cache_max_bytes
                        || pinned.contains(&module.name)
                        || failed.is_some()
                    {
                        kept.push(module);
                        continue;
                    }
                    let path = root.join(&module.name);
                    let removed = if module.directory {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                    match removed {
                        Ok(()) => bytes -= module.bytes,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bytes -= module.bytes,
                        Err(e) => {
                            failed = Some(format!("{}: {e}", path.display()));
                            kept.push(module);
                        }
                    }
                }
                modules = kept;
            }
            (!pinned.is_empty()).then(|| {
                modules
                    .iter()
                    .filter(|m| pinned.contains(&m.name))
                    .map(|m| m.bytes)
                    .sum::<u64>()
            })
        };
        let wasm_bytes: u64 = modules.iter().map(|m| m.bytes).sum();
        // A runtime that could not be deleted still uses disk, in use or not.
        let runtimes = match pinned_bytes {
            Some(pinned) if failed.is_none() => pinned,
            _ => wasm_bytes,
        };
        let floor = (64 * 1024 * 1024).min(self.cache_max_bytes / 4);
        let mut wasm = self.wasm.lock();
        wasm.modules = modules;
        if let Some(failed) = failed {
            // Measure and try again at the next maintenance; report it once.
            wasm.generation = None;
            if !std::mem::replace(&mut wasm.warned_remove, true) {
                result = Err(Error::Invalid(failed));
            }
        }
        // The space kept for state is part of the disk cache only with a node folder.
        let kept = if self.disk.is_some() { floor } else { 0 };
        let warn = pinned_bytes.is_some_and(|b| b + kept > self.cache_max_bytes)
            && !std::mem::replace(&mut wasm.warned_small, true);
        // Logged after the lock is released: a blocked log must not stall other replays.
        drop(wasm);
        if warn {
            note!(
                "warning: disk cache: --cache-max-bytes {} is less than the runtimes in use ({} bytes) plus {kept} bytes kept for downloaded state; the disk cache stays above the limit",
                self.cache_max_bytes,
                pinned_bytes.unwrap_or_default()
            );
        }
        if let Some(disk) = &self.disk {
            disk.set_budget(self.cache_max_bytes.saturating_sub(runtimes).max(floor));
        }
        result
    }
    /// Proof nodes kept in memory for all replays of this session, within a fixed budget.
    pub fn memory_cache(&self) -> crate::MemoryCacheStats {
        self.nodes.lock().stats()
    }
    pub(crate) fn shared() -> Result<&'static Self> {
        static SESSION: OnceLock<std::result::Result<ReplaySession, String>> = OnceLock::new();
        SESSION
            .get_or_init(|| Self::new(None).map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::Invalid(e.clone()))
    }
}

/// The disk cache root: `cache_dir`, the user's cache directory, or a per-user folder in the
/// temporary directory. Created private, then resolved through symlinks, so a root that links
/// elsewhere (systemd's `CacheDirectory` with `DynamicUser`) is checked where it really is.
fn private_cache(cache_dir: Option<&Path>) -> std::result::Result<PathBuf, String> {
    let base = cache_dir.map(Path::to_path_buf).unwrap_or_else(|| {
        match std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        {
            Some(dir) => dir.join("polkadot-frame-trace"),
            None => std::env::temp_dir().join(format!(
                "polkadot-frame-trace-{}",
                rustix::process::getuid().as_raw()
            )),
        }
    });
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&base)
        .and_then(|()| base.canonicalize())
        .map_err(|e| format!("{}: {e}", base.display()))
        .and_then(|base| {
            private_dir(&base)
                .and_then(|()| trusted_ancestors(&base))
                .map(|()| base)
        })
}

/// The StrictModes rule of OpenSSH and sudo, for every directory above the cache root: owned
/// by root or this user, and not writable by group or others unless sticky, as `/tmp` is.
/// Otherwise another user could rename the cache away and put their own in its place.
fn trusted_ancestors(root: &Path) -> std::result::Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let uid = rustix::process::getuid().as_raw();
    for dir in root.ancestors().skip(1) {
        let fail = |why: std::fmt::Arguments| {
            format!(
                "{}: {why}, so another user could replace the cache below it",
                dir.display()
            )
        };
        let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if meta.uid() != 0 && meta.uid() != uid {
            return Err(fail(format_args!(
                "owned by user {}, not by root or this user ({uid})",
                meta.uid()
            )));
        }
        if meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0 {
            return Err(fail(format_args!(
                "group or others can write to it (mode {:o}) and it is not sticky",
                meta.mode() & 0o7777
            )));
        }
    }
    Ok(())
}

/// Creates `dir` with mode 0700 if missing. Refuses a symlink, a directory another user owns,
/// and one that group or others can write.
fn private_dir(dir: &Path) -> std::result::Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let fail = |why: &dyn std::fmt::Display| format!("{}: {why}", dir.display());
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(fail(&e)),
        _ => {}
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| fail(&e))?;
    if meta.file_type().is_symlink() {
        return Err(fail(&"is a symlink"));
    }
    if !meta.file_type().is_dir() {
        return Err(fail(&"is not a directory"));
    }
    let uid = rustix::process::getuid().as_raw();
    if meta.uid() != uid {
        return Err(fail(&format_args!(
            "owned by user {}, not by this user ({uid})",
            meta.uid()
        )));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(fail(&format_args!(
            "group or others can write to it (mode {:o})",
            meta.mode() & 0o777
        )));
    }
    Ok(())
}

#[derive(Default)]
struct WasmUsage {
    /// What `wasm/` held when last measured, less what maintenance deleted since.
    modules: Vec<WasmModule>,
    /// The runtime count when last measured.
    generation: Option<u64>,
    measured: Option<std::time::Instant>,
    warned_small: bool,
    warned_remove: bool,
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
            *bytes += crate::disk::file_charge(&meta);
            let entries = match std::fs::read_dir(path) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };
            for entry in entries {
                walk(&entry?.path(), bytes, modified)?;
            }
        } else {
            *bytes += crate::disk::file_charge(&meta);
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
        let dir =
            std::env::temp_dir().join(format!("polkadot-frame-trace-wasm-{}", std::process::id()));
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
    #[test]
    fn idle_runtimes_above_four_go_when_the_next_one_loads() {
        let session = ReplaySession::new(None).expect("session");
        // Six runtimes in use at once: all stay.
        let busy: Vec<_> = (0..6u8).map(|i| session.executor_for(&[i])).collect();
        assert_eq!(session.executors.lock().len(), 6);
        drop(busy);
        // Once idle, one more runtime brings the map back to four, not five.
        let _next = session.executor_for(b"next");
        assert_eq!(session.executors.lock().len(), 4);
    }
    fn persist(session: &ReplaySession, nodes: impl Iterator<Item = u32>) {
        let db = crate::proof::ProofDb::with_session(None, H256::zero(), session.clone());
        db.insert_nodes(nodes.map(|i| [i.to_le_bytes().as_slice(), &[7; 200]].concat()));
        db.persist().expect("persist");
    }
    fn stored(session: &ReplaySession, i: u32) -> bool {
        let bytes = [i.to_le_bytes().as_slice(), &[7; 200]].concat();
        let hash = H256::from(sp_io::hashing::blake2_256(&bytes));
        session.disk.as_ref().expect("disk").get(&hash) == Some(bytes)
    }
    #[test]
    fn unused_runtimes_go_before_proof_nodes() {
        let dir = std::env::temp_dir().join(format!(
            "polkadot-frame-trace-pressure-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let cap = 1024 * 1024;
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), cap).expect("session");
        let _active = session.executor_for(b"runtime");
        let unused = dir.join("wasm").join("unused");
        std::fs::create_dir_all(&unused).expect("mkdir");
        std::fs::write(unused.join("module"), vec![0; 400 * 1024]).expect("write");
        session.prune_disk_cache().expect("prune");
        assert!(unused.exists(), "deleted while under the limit");
        // About 800 KB of nodes: with the runtime, more than the limit.
        for block in 0..32 {
            persist(&session, block * 100..block * 100 + 100);
        }
        session.prune_disk_cache().expect("prune");
        assert!(!unused.exists(), "an unused runtime outlived the limit");
        assert!(
            stored(&session, 0),
            "the oldest node went before an unused runtime"
        );
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
    #[test]
    fn a_runtime_larger_than_the_limit_leaves_the_node_floor() {
        let dir =
            std::env::temp_dir().join(format!("polkadot-frame-trace-floor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cap = 1024 * 1024;
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), cap).expect("session");
        let wasm = b"big runtime";
        let _active = session.executor_for(wasm);
        let hash = H256::from(sp_io::hashing::blake2_256(wasm));
        let module = dir.join("wasm").join(format!("{hash:x}"));
        std::fs::create_dir_all(&module).expect("mkdir");
        std::fs::write(module.join("module"), vec![0; 2 * 1024 * 1024]).expect("write");
        for block in 0..8 {
            persist(&session, block * 100..block * 100 + 100);
            session.prune_disk_cache().expect("prune");
            // The floor is a quarter of the limit: the last block's nodes stay.
            assert!(
                stored(&session, block * 100),
                "block {block} was evicted at once"
            );
        }
        assert!(session.disk.as_ref().expect("disk").total() <= cap / 4);
        assert!(session.wasm.lock().warned_small);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
    #[test]
    fn a_runtime_that_cannot_be_deleted_stays_in_the_node_budget() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "polkadot-frame-trace-stuck-wasm-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let cap = 1024 * 1024;
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), cap).expect("session");
        let _active = session.executor_for(b"runtime");
        let unused = dir.join("wasm").join("unused");
        std::fs::create_dir_all(&unused).expect("mkdir");
        std::fs::write(unused.join("module"), vec![0; 600 * 1024]).expect("write");
        // Its file cannot be deleted from a read-only folder.
        std::fs::set_permissions(&unused, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let mut errors = 0;
        for block in 0..32 {
            persist(&session, block * 100..block * 100 + 100);
            errors += usize::from(session.prune_disk_cache().is_err());
        }
        let nodes = session.disk.as_ref().expect("disk").total();
        std::fs::set_permissions(&unused, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        std::fs::remove_dir_all(&dir).expect("cleanup");
        assert!(errors == 1, "{errors} errors: the failure is reported once");
        // 600 KiB of runtime that could not go leaves under 400 KiB for state.
        assert!(nodes <= cap - 600 * 1024, "{nodes} bytes of state");
    }
    #[test]
    fn a_cache_others_could_write_or_that_links_elsewhere_is_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir =
            std::env::temp_dir().join(format!("polkadot-frame-trace-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let used = |root: &Path| {
            let session = ReplaySession::with_cache_max_bytes(Some(root), 0).expect("session");
            (session.wasm_cache, session.disk.is_some())
        };
        // A new root is private; a symlink to it is checked where it leads, as systemd's
        // `CacheDirectory` with `DynamicUser` makes.
        let fresh = dir.join("fresh");
        assert_eq!(used(&fresh), (true, true));
        let mode = std::fs::metadata(&fresh)
            .expect("root")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        symlink(&fresh, dir.join("alias")).expect("symlink");
        assert_eq!(used(&dir.join("alias")), (true, true));
        // A `wasm` folder that links elsewhere is not used, and nothing there is deleted.
        let outside = dir.join("outside").join("module");
        std::fs::create_dir_all(&outside).expect("mkdir");
        std::fs::write(outside.join("file"), [0; 64]).expect("write");
        let linked = dir.join("linked");
        std::fs::create_dir(&linked).expect("mkdir");
        symlink(dir.join("outside"), linked.join("wasm")).expect("symlink");
        let session = ReplaySession::with_cache_max_bytes(Some(&linked), 0).expect("session");
        assert!(!session.wasm_cache);
        let _active = session.executor_for(b"runtime");
        session.prune_disk_cache().expect("prune");
        assert!(outside.join("file").exists());
        // A root the group can write is refused whole.
        let shared = dir.join("shared");
        std::fs::create_dir(&shared).expect("mkdir");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o775)).expect("chmod");
        assert_eq!(used(&shared), (false, false));
        // So is one below a parent the group can write: they could swap the whole cache. A
        // sticky parent, as `/tmp` is, lets no one rename what another user owns.
        let below = shared.join("cache");
        assert_eq!(used(&below), (false, false));
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1775)).expect("chmod");
        assert_eq!(used(&below), (true, true));
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
    #[test]
    fn a_disabled_wasm_cache_is_not_maintained() {
        let dir = std::env::temp_dir().join(format!(
            "polkadot-frame-trace-no-wasm-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("wasm"), b"not a folder").expect("write");
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), 1 << 20).expect("session");
        let _active = session.executor_for(b"runtime");
        for _ in 0..3 {
            session
                .prune_disk_cache()
                .expect("no error for a folder that is not used");
        }
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
    #[test]
    fn runtimes_in_use_plus_the_node_floor_over_the_limit_warn() {
        let dir = std::env::temp_dir().join(format!(
            "polkadot-frame-trace-floor-warn-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let cap = 1024 * 1024;
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), cap).expect("session");
        let wasm = b"runtime in use";
        let _active = session.executor_for(wasm);
        let hash = H256::from(sp_io::hashing::blake2_256(wasm));
        let module = dir.join("wasm").join(format!("{hash:x}"));
        std::fs::create_dir_all(&module).expect("mkdir");
        // Under the limit alone, over it with the 256 KiB kept for state.
        std::fs::write(module.join("module"), vec![0; 900 * 1024]).expect("write");
        session.prune_disk_cache().expect("prune");
        assert!(session.wasm.lock().warned_small);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
    #[test]
    fn an_unusable_cache_directory_is_a_warning() {
        let dir = std::env::temp_dir().join(format!(
            "polkadot-frame-trace-unusable-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        // A file where the node folder should be.
        std::fs::write(dir.join("proof-nodes"), b"not a folder").expect("write");
        let session = ReplaySession::with_cache_max_bytes(Some(&dir), 1 << 30).expect("session");
        assert!(session.disk.is_none());
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}
