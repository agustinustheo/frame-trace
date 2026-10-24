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
}
impl ReplaySession {
    /// `cache_dir` enables persistent proof nodes and Wasmtime artifacts. Without it, only
    /// compiled modules persist in the user's cache directory; proof nodes stay in memory.
    pub fn new(cache_dir: Option<&Path>) -> Result<Self> {
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
        Ok(Self {
            nodes: Default::default(),
            hints: Default::default(),
            node_dir,
            speculative_discovery: true,
            executor: Arc::new(
                Executor::builder()
                    .with_max_runtime_instances(2)
                    .with_runtime_cache_size(4)
                    .with_allow_missing_host_functions(false)
                    .with_cache_path(wasm_dir)
                    .build(),
            ),
        })
    }
    /// Disable throw-away speculative passes when debugging strict execution-driven paging.
    pub fn with_speculative_discovery(mut self, enabled: bool) -> Self {
        self.speculative_discovery = enabled;
        self
    }
    pub(crate) fn shared() -> Result<&'static Self> {
        static SESSION: OnceLock<std::result::Result<ReplaySession, String>> = OnceLock::new();
        SESSION
            .get_or_init(|| Self::new(None).map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::Invalid(e.clone()))
    }
}
