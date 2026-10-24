//! Forward the SDK backend unchanged, adding the iteration events Ext omits.
use crate::{
    proof::{ProofDb, with_child, with_key_hint},
    rpc::hex0,
};
use sp_core::{
    Blake2Hasher as H, H256,
    storage::{ChildInfo, StateVersion},
};
use sp_state_machine::{
    Backend, BackendTransaction, IterArgs, StorageIterator, TrieBackend, TrieBackendBuilder,
    UsageInfo,
};
use sp_trie::{MerkleValue, recorder::Recorder};
use std::time::Instant;

type Trie = TrieBackend<ProofDb, H>;
pub struct TracedBackend {
    pub trie: Trie,
    db: ProofDb,
}
impl std::fmt::Debug for TracedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TracedBackend")
    }
}
impl TracedBackend {
    pub fn new(db: ProofDb, root: H256, recorder: Recorder<H>) -> Self {
        Self {
            db: db.clone(),
            trie: TrieBackendBuilder::new(db, root)
                .with_recorder(recorder)
                .build(),
        }
    }
}
pub struct Iter {
    inner: <Trie as Backend<H>>::RawIter,
    child: Option<ChildInfo>,
}
impl StorageIterator<H> for Iter {
    type Backend = TracedBackend;
    type Error = String;
    fn next_key(&mut self, b: &TracedBackend) -> Option<Result<Vec<u8>, String>> {
        let start = Instant::now();
        let result = match &self.child {
            Some(c) => with_child(c, || self.inner.next_key(&b.trie)),
            None => self.inner.next_key(&b.trie),
        };
        tracing::trace!(target: "frame_trace_backend", method = "BackendIterNextKey",
            result = ?result, child_info = %self.child.as_ref().map(|c| hex0(c.storage_key())).unwrap_or_default(), duration_ns = start.elapsed().as_nanos() as u64);
        if b.db.speculative.load(std::sync::atomic::Ordering::SeqCst) {
            result.filter(Result::is_ok)
        } else {
            result
        }
    }
    fn next_pair(&mut self, b: &TracedBackend) -> Option<Result<(Vec<u8>, Vec<u8>), String>> {
        let start = Instant::now();
        let result = match &self.child {
            Some(c) => with_child(c, || self.inner.next_pair(&b.trie)),
            None => self.inner.next_pair(&b.trie),
        };
        tracing::trace!(target: "frame_trace_backend", method = "BackendIterNextPair", child_info = %self.child.as_ref().map(|c| hex0(c.storage_key())).unwrap_or_default(), duration_ns = start.elapsed().as_nanos() as u64);
        if b.db.speculative.load(std::sync::atomic::Ordering::SeqCst) {
            result.filter(Result::is_ok)
        } else {
            result
        }
    }
    fn was_complete(&self) -> bool {
        self.inner.was_complete()
    }
}

impl Backend<H> for TracedBackend {
    type Error = String;
    type TrieBackendStorage = ProofDb;
    type RawIter = Iter;
    fn storage(&self, k: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.db.remember_key(k);
        self.db
            .speculative_value(with_key_hint(k, || self.trie.storage(k)))
    }
    fn storage_hash(&self, k: &[u8]) -> Result<Option<H256>, String> {
        self.db.remember_key(k);
        self.db
            .speculative_value(with_key_hint(k, || self.trie.storage_hash(k)))
    }
    fn closest_merkle_value(&self, k: &[u8]) -> Result<Option<MerkleValue<H256>>, String> {
        self.db.remember_key(k);
        self.db
            .speculative_value(with_key_hint(k, || self.trie.closest_merkle_value(k)))
    }
    fn child_closest_merkle_value(
        &self,
        c: &ChildInfo,
        k: &[u8],
    ) -> Result<Option<MerkleValue<H256>>, String> {
        self.trie.storage(c.prefixed_storage_key().as_slice())?;
        with_child(c, || {
            with_key_hint(k, || self.trie.child_closest_merkle_value(c, k))
        })
    }
    fn child_storage(&self, c: &ChildInfo, k: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.trie.storage(c.prefixed_storage_key().as_slice())?;
        with_child(c, || with_key_hint(k, || self.trie.child_storage(c, k)))
    }
    fn child_storage_hash(&self, c: &ChildInfo, k: &[u8]) -> Result<Option<H256>, String> {
        self.trie.storage(c.prefixed_storage_key().as_slice())?;
        with_child(c, || {
            with_key_hint(k, || self.trie.child_storage_hash(c, k))
        })
    }
    fn next_storage_key(&self, k: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let start = Instant::now();
        let result = self.trie.next_storage_key(k);
        tracing::trace!(target: "frame_trace_backend", method = "BackendNextKey", key = %hex0(k),
            result = ?result.as_ref().map(|k| k.as_deref().map(hex0)), duration_ns = start.elapsed().as_nanos() as u64);
        self.db.speculative_value(result)
    }
    fn next_child_storage_key(&self, c: &ChildInfo, k: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let start = Instant::now();
        self.trie.storage(c.prefixed_storage_key().as_slice())?;
        let result = with_child(c, || self.trie.next_child_storage_key(c, k));
        tracing::trace!(target: "frame_trace_backend", method = "BackendChildNextKey", key = %hex0(k),
            child_info = %hex0(c.storage_key()), result = ?result, duration_ns = start.elapsed().as_nanos() as u64);
        self.db.speculative_value(result)
    }
    fn raw_iter(&self, args: IterArgs) -> Result<Iter, String> {
        let child = args.child_info.clone();
        if let Some(c) = &child {
            self.trie.storage(c.prefixed_storage_key().as_slice())?;
        }
        let inner = match &child {
            Some(c) => with_child(c, || self.trie.raw_iter(args)),
            None => self.trie.raw_iter(args),
        }?;
        Ok(Iter { inner, child })
    }
    fn storage_root<'a>(
        &self,
        delta: impl Iterator<Item = (&'a [u8], Option<&'a [u8]>)>,
        version: StateVersion,
    ) -> (H256, BackendTransaction<H>) {
        self.trie.storage_root(delta, version)
    }
    fn child_storage_root<'a>(
        &self,
        c: &ChildInfo,
        delta: impl Iterator<Item = (&'a [u8], Option<&'a [u8]>)>,
        version: StateVersion,
    ) -> (H256, bool, BackendTransaction<H>) {
        // Prime the authenticated top-level child root before selecting its namespace.
        let _ = self.trie.storage(c.prefixed_storage_key().as_slice());
        with_child(c, || self.trie.child_storage_root(c, delta, version))
    }
    fn register_overlay_stats(&self, _: &sp_state_machine::StateMachineStats) {}
    fn usage_info(&self) -> UsageInfo {
        UsageInfo::empty()
    }
}
