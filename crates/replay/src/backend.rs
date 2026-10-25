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

// Wrap the SDK recorder: observe hashes incrementally, never copy its proof.
// The lifetime set deliberately survives SDK recorder transaction rollback.
#[derive(Clone)]
struct ObservedRecorder {
    inner: Recorder<H>,
    seen: std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<H256>>>,
}
struct ObservedTrieRecorder<'a> {
    inner: sp_trie::recorder::TrieRecorder<'a, H>,
    seen: &'a parking_lot::Mutex<std::collections::HashSet<H256>>,
}
impl sp_trie::TrieRecorder<H256> for ObservedTrieRecorder<'_> {
    fn record(&mut self, access: trie_db::TrieAccess<'_, H256>) {
        let hash = match &access {
            trie_db::TrieAccess::NodeOwned { hash, .. }
            | trie_db::TrieAccess::EncodedNode { hash, .. }
            | trie_db::TrieAccess::Value { hash, .. } => Some(*hash),
            _ => None,
        };
        if let Some(hash) = hash {
            self.seen.lock().insert(hash);
        }
        self.inner.record(access);
    }
    fn trie_nodes_recorded_for_key(&self, key: &[u8]) -> trie_db::RecordedForKey {
        self.inner.trie_nodes_recorded_for_key(key)
    }
}
impl sp_trie::TrieRecorderProvider<H> for ObservedRecorder {
    type Recorder<'a> = ObservedTrieRecorder<'a>;
    fn drain_storage_proof(self) -> Option<sp_trie::StorageProof> {
        Some(self.inner.drain_storage_proof())
    }
    fn as_trie_recorder(&self, root: H256) -> Self::Recorder<'_> {
        ObservedTrieRecorder {
            inner: self.inner.as_trie_recorder(root),
            seen: &self.seen,
        }
    }
}
type Trie = TrieBackend<ProofDb, H, sp_trie::cache::LocalTrieCache<H>, ObservedRecorder>;
pub struct TracedBackend {
    trie: Trie,
    db: ProofDb,
    recorder: Recorder<H>,
    seen: std::sync::Arc<parking_lot::Mutex<std::collections::HashSet<H256>>>,
}
impl std::fmt::Debug for TracedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TracedBackend")
    }
}
impl TracedBackend {
    fn read_event(
        &self,
        key: &[u8],
        child: Option<&ChildInfo>,
        before: usize,
        value: Option<H256>,
        seen_before: usize,
    ) {
        tracing::trace!(target: "frame_trace_backend", method = "BackendRead", key = %hex0(key),
            child_info = %child.map(|c| hex0(c.storage_key())).unwrap_or_default(),
            value_hash = %value.map(|h| format!("{h:?}")).unwrap_or_else(|| "none".into()),
            proof_bytes_added = self.recorder.estimate_encoded_size().saturating_sub(before),
            cold = self.seen.lock().len() > seen_before);
    }

    pub fn new(db: ProofDb, root: H256, recorder: Recorder<H>) -> Self {
        let seen = Default::default();
        let observed = ObservedRecorder {
            inner: recorder.clone(),
            seen: std::sync::Arc::clone(&seen),
        };
        Self {
            seen,
            db: db.clone(),
            recorder: recorder.clone(),
            trie: TrieBackendBuilder::new_with_cache(db, root, ())
                .with_optional_cache(None::<sp_trie::cache::LocalTrieCache<H>>)
                .with_recorder(observed)
                .build(),
        }
    }
}
pub struct Iter {
    inner: <Trie as Backend<H>>::RawIter,
    child: Option<ChildInfo>,
    prefix: Vec<u8>,
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
        tracing::trace!(target: "frame_trace_backend", method = "BackendIterNextKey", prefix = %hex0(&self.prefix),
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
        tracing::trace!(target: "frame_trace_backend", method = "BackendIterNextPair", prefix = %hex0(&self.prefix), child_info = %self.child.as_ref().map(|c| hex0(c.storage_key())).unwrap_or_default(), duration_ns = start.elapsed().as_nanos() as u64);
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
        let before = self.recorder.estimate_encoded_size();
        let seen_before = self.seen.lock().len();
        let result = self
            .db
            .speculative_value(with_key_hint(k, || self.trie.storage(k)));
        self.read_event(
            k,
            None,
            before,
            result
                .as_ref()
                .ok()
                .and_then(|v| v.as_ref())
                .map(|v| H256::from(sp_io::hashing::blake2_256(v))),
            seen_before,
        );
        result
    }
    fn storage_hash(&self, k: &[u8]) -> Result<Option<H256>, String> {
        self.db.remember_key(k);
        let before = self.recorder.estimate_encoded_size();
        let seen_before = self.seen.lock().len();
        let result = self
            .db
            .speculative_value(with_key_hint(k, || self.trie.storage_hash(k)));
        self.read_event(
            k,
            None,
            before,
            result.as_ref().ok().copied().flatten(),
            seen_before,
        );
        result
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
        let before = self.recorder.estimate_encoded_size();
        let seen_before = self.seen.lock().len();
        self.trie.storage(c.prefixed_storage_key().as_slice())?;
        let result = with_child(c, || with_key_hint(k, || self.trie.child_storage(c, k)));
        self.read_event(
            k,
            Some(c),
            before,
            result
                .as_ref()
                .ok()
                .and_then(|v| v.as_ref())
                .map(|v| H256::from(sp_io::hashing::blake2_256(v))),
            seen_before,
        );
        result
    }
    fn child_storage_hash(&self, c: &ChildInfo, k: &[u8]) -> Result<Option<H256>, String> {
        let before = self.recorder.estimate_encoded_size();
        let seen_before = self.seen.lock().len();
        self.trie.storage(c.prefixed_storage_key().as_slice())?;
        let result = with_child(c, || {
            with_key_hint(k, || self.trie.child_storage_hash(c, k))
        });
        self.read_event(
            k,
            Some(c),
            before,
            result.as_ref().ok().copied().flatten(),
            seen_before,
        );
        result
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
        let prefix = args.prefix.unwrap_or_default().to_vec();
        if let Some(c) = &child {
            self.trie.storage(c.prefixed_storage_key().as_slice())?;
        }
        let inner = match &child {
            Some(c) => with_child(c, || self.trie.raw_iter(args)),
            None => self.trie.raw_iter(args),
        }?;
        Ok(Iter {
            inner,
            child,
            prefix,
        })
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
