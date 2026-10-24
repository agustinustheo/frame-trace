//! Demand-paged content-addressed nodes. SDK trie traversal authenticates all returned nodes.
use crate::{
    Error, Result,
    rpc::{Rpc, hex0, unhex},
};
use hash_db::{EMPTY_PREFIX, HashDB, Prefix};
use parking_lot::Mutex;
use serde_json::json;
use sp_core::{Blake2Hasher, H256, storage::ChildInfo};
use sp_state_machine::TrieBackendStorage;
use sp_trie::MemoryDB;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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

#[derive(Clone)]
pub struct ProofDb {
    rpc: Option<Rpc>,
    parent: H256,
    db: Arc<Mutex<MemoryDB<Blake2Hasher>>>,
    nodes: Arc<Mutex<BTreeMap<H256, Vec<u8>>>>,
    // Serialize misses and recheck the cache after acquiring this gate. Concurrent requests for
    // the same node share the first result. Complete-key hints batch the entire lookup path.
    fetch_gate: Arc<Mutex<()>>,
    pub online: Arc<AtomicBool>,
    pub failure: Arc<Mutex<Option<String>>>,
}
impl ProofDb {
    pub fn new(rpc: Option<Rpc>, parent: H256) -> Self {
        let online = rpc.is_some();
        Self {
            rpc,
            parent,
            db: Default::default(),
            nodes: Default::default(),
            fetch_gate: Default::default(),
            online: Arc::new(AtomicBool::new(online)),
            failure: Default::default(),
        }
    }
    pub fn fetch(&self, child: Option<&ChildInfo>, paths: &[Vec<u8>]) -> Result<()> {
        if !self.online.load(Ordering::SeqCst) {
            return Err(Error::Invalid("offline proof cache miss".into()));
        }
        let rpc = self
            .rpc
            .as_ref()
            .ok_or_else(|| Error::Invalid("offline proof cache miss".into()))?;
        let keys: Vec<_> = paths.iter().map(|k| hex0(k)).collect();
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
        let nodes = proof["proof"].as_array().ok_or_else(|| {
            Error::Invalid("proof nodes are not an array; parent state may be pruned".into())
        })?;
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
        let mut db = self.db.lock();
        let mut nodes = self.nodes.lock();
        for bytes in fetched {
            let hash = HashDB::insert(&mut *db, EMPTY_PREFIX, &bytes);
            nodes.entry(hash).or_insert(bytes);
        }
    }
    pub fn export(&self) -> Vec<String> {
        self.nodes.lock().values().map(|v| hex0(v)).collect()
    }
}
impl TrieBackendStorage<Blake2Hasher> for ProofDb {
    fn get(&self, hash: &H256, prefix: Prefix) -> std::result::Result<Option<Vec<u8>>, String> {
        if let Some(node) = HashDB::get(&*self.db.lock(), hash, EMPTY_PREFIX) {
            return Ok(Some(node));
        }
        let _gate = self.fetch_gate.lock();
        if let Some(node) = HashDB::get(&*self.db.lock(), hash, EMPTY_PREFIX) {
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
            KEY_HINT.with(|h| {
                if let Some(key) = &*h.borrow()
                    && key.starts_with(bytes)
                    && prefix
                        .1
                        .is_none_or(|n| key.get(bytes.len()).is_some_and(|b| b & 0xf0 == n))
                {
                    path = key.clone();
                }
            });
            self.fetch(child.as_ref(), &[path.clone()])
                .map_err(|e| format!("proof fetch for {hash:?} at {}: {e}", hex0(&path)))?;
            HashDB::get(&*self.db.lock(), hash, EMPTY_PREFIX).map(Some).ok_or_else(|| format!("proof did not contain {hash:?} at {}; invalid proof or node pruned the parent state", hex0(&path)))
        })();
        if let Err(e) = &result {
            *self.failure.lock() = Some(e.clone());
        }
        result
    }
}
