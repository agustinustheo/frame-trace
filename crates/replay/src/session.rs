//! Reusable, bounded acquisition and compiled-runtime caches. Each replay authenticates its own root.
use crate::{Error, Executor, Result};
use parking_lot::Mutex;
use sp_core::H256;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

const MAX_SHARED_BYTES: usize = 256 * 1024 * 1024;
#[derive(Default)]
pub(crate) struct Nodes {
    pub nodes: BTreeMap<H256, Vec<u8>>,
    bytes: usize,
}
impl Nodes {
    pub fn insert(&mut self, hash: H256, bytes: Vec<u8>) {
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
    pub(crate) hints: Arc<Mutex<BTreeSet<Vec<u8>>>>,
    pub(crate) executor: Arc<Executor>,
    pub(crate) node_dir: Option<PathBuf>,
    pub(crate) speculative_discovery: bool,
    cache_base: PathBuf,
    cache_max_bytes: u64,
    disk_gate: Arc<Mutex<()>>,
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
            node_dir,
            cache_base: base,
            cache_max_bytes,
            disk_gate: Default::default(),
            speculative_discovery: true,
            executor: Arc::new(
                Executor::builder()
                    .with_max_runtime_instances(2)
                    .with_runtime_cache_size(4)
                    .with_allow_missing_host_functions(false)
                    .with_cache_path(wasm_dir)
                    .build(),
            ),
        };
        session.prune_disk_cache()?;
        Ok(session)
    }
    /// Disable throw-away speculative passes when debugging strict execution-driven paging.
    pub fn with_speculative_discovery(mut self, enabled: bool) -> Self {
        self.speculative_discovery = enabled;
        self
    }
    pub(crate) fn prune_disk_cache(&self) -> Result<()> {
        let _gate = self.disk_gate.lock();
        fn files(
            dir: &Path,
            out: &mut Vec<(std::time::SystemTime, PathBuf, u64)>,
        ) -> std::io::Result<()> {
            if !dir.exists() {
                return Ok(());
            }
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let meta = match entry.path().symlink_metadata() {
                    Ok(meta) => meta,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                if meta.is_dir() {
                    files(&entry.path(), out)?;
                } else if meta.is_file() {
                    let modified = meta.modified()?;
                    // Do not race active atomic writes; interrupted writes age into eviction.
                    let active_temporary = entry.path().extension().is_some_and(|ext| ext == "tmp")
                        && modified.elapsed().unwrap_or_default()
                            < std::time::Duration::from_secs(300);
                    if !active_temporary {
                        out.push((modified, entry.path(), meta.len()));
                    }
                }
            }
            Ok(())
        }
        let mut entries = Vec::new();
        for dir in [self.node_dir.clone(), Some(self.cache_base.join("wasm"))]
            .into_iter()
            .flatten()
        {
            files(&dir, &mut entries)
                .map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?;
        }
        let mut total = entries.iter().map(|(_, _, size)| size).sum::<u64>();
        entries.sort_by_key(|(time, _, _)| *time);
        for (_, path, size) in entries {
            if total <= self.cache_max_bytes {
                break;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => total = total.saturating_sub(size),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    total = total.saturating_sub(size)
                }
                Err(e) => return Err(Error::Invalid(format!("{}: {e}", path.display()))),
            }
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
