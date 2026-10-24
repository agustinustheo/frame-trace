//! Demand-paged content-addressed nodes. SDK trie traversal authenticates all returned nodes.
use crate::session::ReplaySession;
use crate::{
    Error, Result,
    rpc::{Rpc, hex0, unhex},
};
use hash_db::Prefix;
use parking_lot::Mutex;
use serde_json::json;
use sp_core::{Blake2Hasher, H256, storage::ChildInfo};
use sp_state_machine::TrieBackendStorage;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

thread_local! {
    static KEY_HINT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    static CHILD: std::cell::RefCell<Option<ChildInfo>> = const { std::cell::RefCell::new(None) };
}
pub fn with_key_hint<T>(key: &[u8], f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Vec<u8>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            KEY_HINT.with(|h| h.replace(self.0.take()));
        }
    }
    let _restore = Restore(KEY_HINT.with(|h| h.replace(Some(key.to_vec()))));
    f()
}
pub fn with_child<T>(child: &ChildInfo, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<ChildInfo>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CHILD.with(|h| h.replace(self.0.take()));
        }
    }
    let _restore = Restore(CHILD.with(|h| h.replace(Some(child.clone()))));
    f()
}

type PendingKey = (Option<Vec<u8>>, Vec<u8>);

#[derive(Clone)]
pub struct ProofDb {
    rpc: Option<Rpc>,
    parent: H256,
    session: ReplaySession,
    nodes: Arc<Mutex<BTreeMap<H256, Vec<u8>>>>,
    used: Arc<Mutex<BTreeSet<H256>>>,
    bootstrap: Arc<Mutex<BTreeSet<H256>>>,
    families: Arc<BTreeSet<(Vec<u8>, Concat)>>,
    // Serialize misses and recheck the cache after acquiring this gate. Concurrent requests for
    // the same node share the first result. Complete-key hints batch the entire lookup path.
    fetch_gate: Arc<Mutex<()>>,
    pub online: Arc<AtomicBool>,
    pub speculative: Arc<AtomicBool>,
    pending: Arc<Mutex<BTreeMap<PendingKey, Option<ChildInfo>>>>,
    observed: Arc<Mutex<BTreeSet<Vec<u8>>>>,
    root: Arc<Mutex<Option<H256>>>,
    rpc_failure: Arc<Mutex<Option<Error>>>,
    pub failure: Arc<Mutex<Option<String>>>,
    pub round_trips: Arc<AtomicUsize>,
}
impl ProofDb {
    #[cfg(test)]
    pub fn new(rpc: Option<Rpc>, parent: H256) -> Self {
        Self::with_session(
            rpc,
            parent,
            ReplaySession::new(None).expect("cache directory"),
        )
    }
    pub fn with_session(rpc: Option<Rpc>, parent: H256, session: ReplaySession) -> Self {
        let online = rpc.is_some();
        let families = Arc::new(families(&session.predicted_keys()));
        Self {
            rpc,
            parent,
            session,
            nodes: Default::default(),
            used: Default::default(),
            bootstrap: Default::default(),
            families,
            fetch_gate: Default::default(),
            online: Arc::new(AtomicBool::new(online)),
            speculative: Default::default(),
            pending: Default::default(),
            observed: Default::default(),
            root: Default::default(),
            rpc_failure: Default::default(),
            failure: Default::default(),
            round_trips: Default::default(),
        }
    }
    pub fn rpc(&self) -> Option<&Rpc> {
        self.rpc.as_ref()
    }
    pub fn mark_bootstrap(&self) {
        *self.bootstrap.lock() = self.used.lock().clone();
    }
    pub fn begin_measured(&self) {
        *self.used.lock() = self.bootstrap.lock().clone();
    }
    pub fn set_root(&self, root: H256) {
        *self.root.lock() = Some(root);
    }
    pub fn take_error(&self, fallback: String) -> Error {
        self.rpc_failure
            .lock()
            .take()
            .unwrap_or(Error::Invalid(fallback))
    }
    /// Walk only authenticated cache nodes from this block's parent root. This probe cannot
    /// fetch, record failures, or use speculative placeholders. Prefetched nodes are exported
    /// only if execution subsequently reads them.
    fn covered(&self, child: Option<&ChildInfo>, key: &[u8]) -> bool {
        use sp_state_machine::Backend;
        struct Cached<'a>(&'a ProofDb);
        impl TrieBackendStorage<Blake2Hasher> for Cached<'_> {
            fn get(&self, hash: &H256, _: Prefix) -> std::result::Result<Option<Vec<u8>>, String> {
                self.0.cached(hash)
            }
        }
        let Some(root) = *self.root.lock() else {
            return false;
        };
        let backend = sp_state_machine::TrieBackendBuilder::new(Cached(self), root).build();
        match child {
            Some(child) => backend.child_storage(child, key).is_ok(),
            None => backend.storage(key).is_ok(),
        }
    }
    pub fn fetch(&self, child: Option<&ChildInfo>, paths: &[Vec<u8>]) -> Result<()> {
        self.fetch_inner(child, paths, true)
    }
    fn fetch_inner(
        &self,
        child: Option<&ChildInfo>,
        paths: &[Vec<u8>],
        count_round: bool,
    ) -> Result<()> {
        if !self.online.load(Ordering::SeqCst) {
            return Err(Error::Invalid("offline proof cache miss".into()));
        }
        let rpc = self
            .rpc
            .as_ref()
            .ok_or_else(|| Error::Invalid("offline proof cache miss".into()))?;
        let keys: Vec<_> = paths
            .iter()
            .filter(|key| !self.covered(child, key))
            .map(|k| hex0(k))
            .collect();
        if keys.is_empty() {
            return Ok(());
        }
        if count_round {
            self.round_trips.fetch_add(1, Ordering::Relaxed);
        }
        let proof = match child {
            Some(child) => rpc.call(
                "state_getChildReadProof",
                json!([
                    hex0(child.prefixed_storage_key().as_slice()),
                    keys,
                    self.parent
                ]),
            )?,
            None => rpc.call("state_getReadProof", json!([keys, self.parent]))?,
        };
        if proof["at"] != json!(self.parent) {
            return Err(Error::Invalid(
                "read proof returned a different parent hash".into(),
            ));
        }
        let nodes = proof["proof"]
            .as_array()
            .ok_or_else(|| Error::Invalid("proof nodes are not an array".into()))?;
        self.insert_nodes(
            nodes
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or_else(|| Error::Invalid("non-string proof node".into()))
                        .and_then(unhex)
                })
                .collect::<Result<Vec<_>>>()?,
        );
        Ok(())
    }
    pub fn insert_nodes(&self, fetched: impl IntoIterator<Item = Vec<u8>>) {
        let mut nodes = self.nodes.lock();
        let mut shared = self.session.nodes.lock();
        for bytes in fetched {
            let hash = H256::from(sp_io::hashing::blake2_256(&bytes));
            shared.insert(hash, bytes.clone());
            nodes.entry(hash).or_insert(bytes);
        }
    }
    fn cached(&self, hash: &H256) -> std::result::Result<Option<Vec<u8>>, String> {
        if let Some(node) = self.nodes.lock().get(hash).cloned() {
            return Ok(Some(node));
        }
        // Offline artifacts must stand alone; a warm online session must not mask truncation.
        if self.rpc.is_none() {
            return Ok(None);
        }
        let cached = self.session.nodes.lock().nodes.get(hash).cloned();
        if let Some(node) = cached {
            self.nodes.lock().insert(*hash, node.clone());
            return Ok(Some(node));
        }
        if let Some(dir) = &self.session.node_dir {
            let path = dir.join(format!("{hash:x}"));
            match std::fs::File::open(&path) {
                Ok(file) => {
                    use std::io::Read;
                    let mut node = Vec::new();
                    file.take(16 * 1024 * 1024 + 1)
                        .read_to_end(&mut node)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    if node.len() > 16 * 1024 * 1024
                        || H256::from(sp_io::hashing::blake2_256(&node)) != *hash
                    {
                        // Content-addressed disk caches are disposable. Heal corrupt files.
                        match std::fs::remove_file(&path) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(format!("{}: {e}", path.display())),
                        }
                        return Ok(None);
                    }
                    self.insert_nodes([node.clone()]);
                    return Ok(Some(node));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Ok(None)
    }
    pub fn remember_key(&self, key: &[u8]) {
        if self.rpc.is_some() && key != b":code" && CHILD.with(|c| c.borrow().is_none()) {
            let mut hints = self.observed.lock();
            if hints.len() < 4096 {
                hints.insert(key.to_vec());
            }
        }
    }
    /// Predictable full keys/storage prefixes are acquired in batches, across a bounded pool.
    pub fn prefetch(&self, keys: &[Vec<u8>]) -> Result<()> {
        let concurrency = self.rpc.as_ref().map_or(1, Rpc::parallelism);
        let batches: Vec<_> = keys.chunks(32).collect();
        for group in batches.chunks(concurrency) {
            let before = self.rpc.as_ref().map_or(0, Rpc::count);
            std::thread::scope(|scope| {
                let handles: Vec<_> = group
                    .iter()
                    .map(|keys| scope.spawn(|| self.fetch_inner(None, keys, false)))
                    .collect();
                for handle in handles {
                    handle.join().map_err(|_| {
                        Error::Invalid("proof acquisition worker panicked".into())
                    })??;
                }
                Ok::<_, Error>(())
            })?;
            if self.rpc.as_ref().map_or(0, Rpc::count) > before {
                self.round_trips.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    }
    pub fn acquire_pending(&self) -> Result<bool> {
        let pending = std::mem::take(&mut *self.pending.lock());
        if pending.is_empty() {
            return Ok(false);
        }
        let mut top = Vec::new();
        for ((_, path), child) in pending {
            if let Some(child) = child {
                self.fetch(Some(&child), &[path])?;
            } else {
                self.remember_key(&path);
                top.push(path);
            }
        }
        self.prefetch(&top)?;
        Ok(true)
    }
    pub fn speculative_value<T>(
        &self,
        value: std::result::Result<Option<T>, String>,
    ) -> std::result::Result<Option<T>, String> {
        if self.speculative.load(Ordering::SeqCst) {
            Ok(value.unwrap_or(None))
        } else {
            value
        }
    }
    pub fn persist(&self) -> Result<()> {
        if self.rpc.is_some() {
            self.session.remember_paths(self.observed.lock().clone());
        }
        if let Some(dir) = &self.session.node_dir {
            for (hash, node) in self.nodes.lock().iter() {
                let path = dir.join(format!("{hash:x}"));
                if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == node.len() as u64)
                    && let Ok(bytes) = std::fs::read(&path)
                    && H256::from(sp_io::hashing::blake2_256(&bytes)) == *hash
                {
                    continue;
                }
                static NEXT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                let temporary = dir.join(format!(
                    ".{hash:x}.{}.{}.tmp",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::write(&temporary, node)
                    .map_err(|e| Error::Invalid(format!("{}: {e}", temporary.display())))?;
                std::fs::rename(&temporary, &path)
                    .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
                self.session.index_write(path, node.len() as u64);
            }
        }
        Ok(())
    }
    pub fn export(&self) -> Vec<String> {
        let used = self.used.lock();
        self.nodes
            .lock()
            .iter()
            .filter(|(hash, _)| used.contains(hash))
            .map(|(_, v)| hex0(v))
            .collect()
    }
}
impl TrieBackendStorage<Blake2Hasher> for ProofDb {
    fn get(&self, hash: &H256, prefix: Prefix) -> std::result::Result<Option<Vec<u8>>, String> {
        if let Some(node) = self.cached(hash).inspect_err(|e| {
            *self.failure.lock() = Some(e.clone());
        })? {
            self.used.lock().insert(*hash);
            return Ok(Some(node));
        }
        let _gate = self.fetch_gate.lock();
        if let Some(node) = self.cached(hash).inspect_err(|e| {
            *self.failure.lock() = Some(e.clone());
        })? {
            self.used.lock().insert(*hash);
            return Ok(Some(node));
        }
        let result = (|| {
            let child = CHILD.with(|c| c.borrow().clone());
            let bytes = match &child {
                Some(c) => prefix
                    .0
                    .strip_prefix(c.keyspace())
                    .ok_or_else(|| "child node lacks SDK keyspace prefix".to_owned())?,
                None => prefix.0,
            };
            let mut path = bytes.to_vec();
            if let Some(nibble) = prefix.1 {
                path.push(nibble);
            }
            let mut full_key_hint = false;
            KEY_HINT.with(|h| {
                if let Some(key) = &*h.borrow()
                    // On warm blocks fetch only the missing branch of :code. Requesting the
                    // full key would download the unchanged multi-megabyte Wasm again.
                    && (key != b":code" || bytes == key.as_slice() || self.session.nodes.lock().nodes.is_empty())
                    && key.starts_with(bytes)
                    && prefix
                        .1
                        .is_none_or(|n| key.get(bytes.len()).is_some_and(|b| b & 0xf0 == n))
                {
                    path = key.clone();
                    full_key_hint = true;
                }
            });
            // Root calculation and iteration ask for branch prefixes, rather than full keys.
            // Query each immediate child in the same proof: root collapse commonly needs a
            // neighboring node next. These are hints only; every get still checks its hash.
            let mut paths = if !full_key_hint && self.session.speculative_discovery {
                sibling_paths(&path, prefix.1.is_some())
            } else {
                vec![path.clone()]
            };
            if child.is_none() && full_key_hint {
                paths.extend(related_keys(&paths, &self.families));
            }
            if self.speculative.load(Ordering::SeqCst) {
                // Missing values are placeholders in throw-away executions only. Their output
                // and root are never accepted; the final two runs use strict authentication.
                let mut pending = self.pending.lock();
                for path in &paths {
                    pending.insert(
                        (child.as_ref().map(|c| c.keyspace().to_vec()), path.clone()),
                        child.clone(),
                    );
                }
                return Err(format!(
                    "speculative proof node {hash:?} at {}",
                    hex0(&path)
                ));
            }
            if !self.online.load(Ordering::SeqCst) {
                return Err(format!(
                    "proof node {hash:?} missing/invalid at path {}",
                    hex0(&path)
                ));
            }
            if child.is_none() {
                for path in &paths {
                    self.remember_key(path);
                }
            }
            self.fetch(child.as_ref(), &paths).map_err(|e| {
                let message = format!("proof fetch for {hash:?} at {}: {e}", hex0(&path));
                *self.rpc_failure.lock() = Some(e);
                message
            })?;
            self.cached(hash)?.map(Some).ok_or_else(|| {
                format!(
                    "proof did not contain {hash:?} at {}; missing/invalid proof node",
                    hex0(&path)
                )
            })
        })();
        if let Err(e) = &result
            && !self.speculative.load(Ordering::SeqCst)
        {
            *self.failure.lock() = Some(e.clone());
        }
        if matches!(&result, Ok(Some(_))) {
            self.used.lock().insert(*hash);
        }
        result
    }
}

/// Prefix bytes are padded with zero in the low nibble when the SDK supplies an odd prefix.
pub(crate) fn sibling_paths(path: &[u8], odd: bool) -> Vec<Vec<u8>> {
    let mut paths = vec![path.to_vec()];
    for nibble in 0..16 {
        let mut sibling = path.to_vec();
        if odd {
            if let Some(last) = sibling.last_mut() {
                *last = (nibble << 4) | (*last & 0x0f);
            }
        } else {
            sibling.push(nibble << 4);
        }
        if sibling != path {
            paths.push(sibling);
        }
    }
    paths
}

/// Learned single-map concat layouts let a newly encountered 32-byte identifier predict
/// related reads (e.g. a linked-list node followed by account/ledger lookups). These are
/// bandwidth-only guesses: every used value still traverses the current authenticated root.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Concat {
    Blake,
    Twox,
}
fn split(key: &[u8]) -> Option<(Concat, &[u8])> {
    if key.len() == 80 && key[32..48] == sp_crypto_hashing::blake2_128(&key[48..]) {
        Some((Concat::Blake, &key[48..]))
    } else if key.len() == 72 && key[32..40] == sp_crypto_hashing::twox_64(&key[40..]) {
        Some((Concat::Twox, &key[40..]))
    } else {
        None
    }
}
fn families(known: &BTreeSet<Vec<u8>>) -> BTreeSet<(Vec<u8>, Concat)> {
    known
        .iter()
        .filter_map(|key| split(key).map(|(hasher, _)| (key[..32].to_vec(), hasher)))
        .take(256)
        .collect()
}
fn related_keys(keys: &[Vec<u8>], families: &BTreeSet<(Vec<u8>, Concat)>) -> Vec<Vec<u8>> {
    let mut hints = BTreeSet::new();
    for key in keys {
        let Some((_, value)) = split(key) else {
            continue;
        };
        for (prefix, hasher) in families {
            let mut predicted = prefix.clone();
            match hasher {
                Concat::Blake => predicted.extend(sp_crypto_hashing::blake2_128(value)),
                Concat::Twox => predicted.extend(sp_crypto_hashing::twox_64(value)),
            }
            predicted.extend(value);
            if !keys.contains(&predicted) {
                hints.insert(predicted);
            }
            if hints.len() >= 31 {
                return hints.into_iter().collect();
            }
        }
    }
    hints.into_iter().collect()
}

#[cfg(test)]
mod prediction_tests {
    use super::*;
    fn key(prefix: u8, id: u8, twox: bool) -> Vec<u8> {
        let mut key = vec![prefix; 32];
        let id = [id; 32];
        if twox {
            key.extend(sp_crypto_hashing::twox_64(&id));
        } else {
            key.extend(sp_crypto_hashing::blake2_128(&id));
        }
        key.extend(id);
        key
    }
    #[test]
    fn related_predictions_rehash_new_ids_across_observed_maps_and_bound_bandwidth() {
        let known = [key(1, 7, false), key(2, 8, true)].into_iter().collect();
        let source = key(1, 9, false);
        assert_eq!(
            related_keys(std::slice::from_ref(&source), &families(&known)),
            vec![key(2, 9, true)]
        );
        let mut corrupt = key(3, 7, false);
        corrupt[32] ^= 1;
        assert!(
            related_keys(
                std::slice::from_ref(&source),
                &families(&[corrupt].into_iter().collect())
            )
            .is_empty()
        );
        assert!(related_keys(&[vec![0; 32]], &families(&known)).is_empty());
        let many = (0..64).map(|p| key(p, 7, false)).collect();
        assert_eq!(related_keys(&[source], &families(&many)).len(), 31);
    }
}
