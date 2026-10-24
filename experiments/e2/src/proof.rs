//! Demand-paged, authenticated trie nodes. The trie supplies the nibble path on
//! a database miss; a read proof for that path includes the missing node, even
//! when that path is an absent/partial storage key. No fake roots or values.
use crate::rpc::{hex0, unhex, Rpc};
use anyhow::{bail, Context, Result};
use hash_db::{HashDB, Prefix, EMPTY_PREFIX};
use parking_lot::Mutex;
use serde_json::{json, Value};
use sp_core::{Blake2Hasher, H256};
use sp_state_machine::TrieBackendStorage;
use sp_trie::MemoryDB;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

thread_local! { static KEY_HINT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) }; }

// Prefer a complete logical lookup key over a partial trie-node path. One
// proof then supplies the rest of that lookup's path. Root calculation and
// iteration still work without hints, including missing sibling nodes.
pub fn with_key_hint<T>(key: &[u8], f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Vec<u8>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            KEY_HINT.with(|h| h.replace(self.0.take()));
        }
    }
    let previous = KEY_HINT.with(|h| h.replace(Some(key.to_vec())));
    let _restore = Restore(previous);
    f()
}

#[derive(Clone)]
pub struct ProofDb {
    pub rpc: Rpc,
    pub parent: H256,
    db: Arc<Mutex<MemoryDB<Blake2Hasher>>>,
    nodes: Arc<Mutex<BTreeMap<H256, Vec<u8>>>>,
    pub online: Arc<AtomicBool>,
    pub misses: Arc<Mutex<Vec<Value>>>,
}

impl ProofDb {
    pub fn new(rpc: Rpc, parent: H256) -> Self {
        Self {
            rpc,
            parent,
            db: Default::default(),
            nodes: Default::default(),
            online: Arc::new(AtomicBool::new(true)),
            misses: Default::default(),
        }
    }

    pub fn fetch(&self, path: &[u8]) -> Result<()> {
        if !self.online.load(Ordering::Relaxed) {
            bail!("offline proof cache miss at {}", hex0(path));
        }
        let proof = self
            .rpc
            .call("state_getReadProof", json!([[hex0(path)], self.parent]))?;
        if proof["at"] != json!(self.parent) {
            bail!("read proof returned a different block");
        }
        let mut fetched = Vec::new();
        for node in proof["proof"]
            .as_array()
            .context("read proof is not an array")?
        {
            fetched.push(unhex(node.as_str().context("non-string proof node")?)?);
        }
        self.insert_nodes(fetched);
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
    pub fn bytes(&self) -> usize {
        self.nodes.lock().values().map(Vec::len).sum()
    }
}

impl TrieBackendStorage<Blake2Hasher> for ProofDb {
    fn get(&self, hash: &H256, prefix: Prefix) -> std::result::Result<Option<Vec<u8>>, String> {
        if let Some(node) = HashDB::get(&*self.db.lock(), hash, EMPTY_PREFIX) {
            return Ok(Some(node));
        }
        let mut path = prefix.0.to_vec();
        if let Some(padded_nibble) = prefix.1 {
            path.push(padded_nibble);
        }
        KEY_HINT.with(|h| {
            if let Some(key) = &*h.borrow() {
                let matches = key.starts_with(prefix.0)
                    && prefix.1.is_none_or(|nibble| {
                        key.get(prefix.0.len()).is_some_and(|b| b & 0xf0 == nibble)
                    });
                if matches {
                    path = key.clone();
                }
            }
        });
        self.misses
            .lock()
            .push(json!({"hash": hash, "path": hex0(&path), "odd_nibble": prefix.1.is_some()}));
        self.fetch(&path)
            .map_err(|e| format!("proof fetch for {hash:?}, path {}: {e:#}", hex0(&path)))?;
        HashDB::get(&*self.db.lock(), hash, EMPTY_PREFIX).map(Some)
            .ok_or_else(|| format!("proof for nibble path {} did not contain {hash:?}; child trie or unsupported path", hex0(&path)))
    }
}
