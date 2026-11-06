//! Proof nodes shared in memory by all replays of a session, within a byte budget.
//!
//! Each node is charged what it costs the process: its allocation, rounded as the allocator
//! rounds it, with the `Arc` header, plus the hash table that indexes it. When a new node would
//! exceed the budget, the least recently used nodes go first, one at a time. The disk cache,
//! when there is one, still has them.
use schnellru::{Limiter, LruMap};
use sp_core::H256;
use std::sync::Arc;

/// Measured on Westend Asset Hub: see `dev/performance.md`, "Memory of a long follow".
pub(crate) const DEFAULT_BUDGET: usize = 128 * 1024 * 1024;
/// The table may use at most this share of the budget. Its size doubles, so without a cap
/// one doubling late in a run could take memory the nodes would use better.
const TABLE_SHARE: usize = 4;
/// One table entry: the hash, the `Arc`, and two links.
const ENTRY: usize = 56;

/// The memory one `Arc<[u8]>` of `len` bytes takes: two reference counts, then the bytes,
/// rounded to the size classes of the macOS allocator. glibc rounds to 16 bytes as well,
/// with an 8-byte header that the 16-byte reference counts partly cover.
pub(crate) fn charge(len: usize) -> usize {
    let size = len + 2 * std::mem::size_of::<usize>();
    let quantum = match size {
        0..=1008 => 16,
        1009..=131_072 => 512,
        _ => 16 * 1024,
    };
    size.div_ceil(quantum) * quantum
}

/// Bytes of node allocations, plus the table's own allocation.
struct Budget {
    budget: usize,
    values: usize,
    table: usize,
    /// At most half of what the largest table allowed holds. A full table then makes room by
    /// rehashing in place: refused growth would make the map drop nodes until half of them
    /// were gone.
    max_len: usize,
    evicted: u64,
}
impl Budget {
    fn charged(&self) -> usize {
        self.values + self.table
    }
}
impl Limiter<H256, Arc<[u8]>> for Budget {
    type KeyToInsert<'a> = H256;
    type LinkType = u32;
    fn is_over_the_limit(&self, len: usize) -> bool {
        self.charged() > self.budget || len > self.max_len
    }
    fn on_insert(&mut self, _: usize, key: H256, value: Arc<[u8]>) -> Option<(H256, Arc<[u8]>)> {
        let cost = charge(value.len());
        // A node that would not fit with every other node gone is not kept at all.
        if cost + self.table > self.budget {
            return None;
        }
        self.values += cost;
        Some((key, value))
    }
    fn on_replace(
        &mut self,
        _: usize,
        _: &mut H256,
        _: H256,
        old: &mut Arc<[u8]>,
        new: &mut Arc<[u8]>,
    ) -> bool {
        // Content-addressed: a replacement has the same bytes, so the same charge.
        self.values = self.values - charge(old.len()) + charge(new.len());
        charge(new.len()) + self.table <= self.budget
    }
    fn on_removed(&mut self, _: &mut H256, value: &mut Arc<[u8]>) {
        self.values -= charge(value.len());
        self.evicted += 1;
    }
    fn on_cleared(&mut self) {
        self.values = 0;
    }
    fn on_grow(&mut self, table: usize) -> bool {
        // Nodes over the budget after growing are dropped one by one, oldest first.
        let grow = table <= self.budget / TABLE_SHARE;
        if grow {
            self.table = table;
        }
        grow
    }
}

/// What the shared node memory holds and how often replays found a node there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryCacheStats {
    /// Bytes charged: node allocations and the table.
    pub bytes: u64,
    pub budget: u64,
    pub entries: u64,
    /// Lookups past a block's own nodes that found the node in memory, and those that did not.
    pub hits: u64,
    pub misses: u64,
    /// Nodes dropped to stay within the budget.
    pub evicted: u64,
}

pub(crate) struct Nodes {
    map: LruMap<H256, Arc<[u8]>, Budget>,
    hits: u64,
    misses: u64,
}
impl Default for Nodes {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET)
    }
}
impl Nodes {
    pub fn with_budget(budget: usize) -> Self {
        // The largest table within its share: 56-byte entries and a control byte per bucket,
        // in a power of two of buckets, seven eighths of which can be full.
        let table =
            LruMap::<H256, Arc<[u8]>, Budget>::memory_usage_for_memory_budget(budget / TABLE_SHARE);
        let buckets = match table / (ENTRY + 1) {
            0 => 0,
            n => 1 << n.ilog2(),
        };
        Self {
            map: LruMap::new(Budget {
                budget,
                values: 0,
                table: 0,
                max_len: (buckets * 7 / 16).max(1),
                evicted: 0,
            }),
            hits: 0,
            misses: 0,
        }
    }
    /// A replay reads this node: it becomes the most recently used.
    pub fn get(&mut self, hash: &H256) -> Option<Arc<[u8]>> {
        let node = self.map.get(hash).cloned();
        if node.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        node
    }
    /// The shared allocation for this node, kept if it fits. Active replays share one
    /// allocation per content hash.
    pub fn intern(&mut self, hash: H256, bytes: Vec<u8>) -> Arc<[u8]> {
        if let Some(node) = self.map.get(&hash) {
            return node.clone();
        }
        let node: Arc<[u8]> = bytes.into();
        self.map.insert(hash, node.clone());
        // Whatever the table really allocated, so the charge stays exact.
        let table = self.map.memory_usage();
        self.map.limiter_mut().table = table;
        while self.map.limiter().is_over_the_limit(self.map.len())
            && self.map.pop_oldest().is_some()
        {}
        node
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    #[cfg(test)]
    pub fn peek(&self, hash: &H256) -> Option<&Arc<[u8]>> {
        self.map.peek(hash)
    }
    pub fn stats(&self) -> MemoryCacheStats {
        let limiter = self.map.limiter();
        MemoryCacheStats {
            bytes: limiter.charged() as u64,
            budget: limiter.budget as u64,
            entries: self.map.len() as u64,
            hits: self.hits,
            misses: self.misses,
            evicted: limiter.evicted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(i: u64, len: usize) -> (H256, Vec<u8>) {
        let mut bytes = vec![0; len.max(8)];
        bytes[..8].copy_from_slice(&i.to_le_bytes());
        (H256::from(sp_io::hashing::blake2_256(&bytes)), bytes)
    }
    #[test]
    fn the_charge_includes_the_arc_header_rounding_and_the_table() {
        assert_eq!(charge(0), 16);
        assert_eq!(charge(1), 32);
        assert_eq!(charge(100), 128);
        assert_eq!(charge(532), 560);
        assert_eq!(charge(1000), 1024);
        assert_eq!(charge(4000), 4096);
        let mut nodes = Nodes::with_budget(64 * 1024 * 1024);
        let mut payload = 0;
        for i in 0..1000 {
            let (hash, bytes) = node(i, 300);
            payload += bytes.len();
            nodes.intern(hash, bytes);
        }
        let stats = nodes.stats();
        assert_eq!(stats.entries, 1000);
        // 300 bytes of payload cost 320 with the header, and the table at least 56 per entry.
        assert!(
            stats.bytes >= payload as u64 + 1000 * (20 + 56),
            "{} charged for {payload} bytes of payload",
            stats.bytes
        );
        assert_eq!(stats.bytes, 1000 * 320 + nodes.map.memory_usage() as u64);
    }
    #[test]
    fn a_randomized_workload_stays_within_the_budget() {
        let budget = 2 * 1024 * 1024;
        let mut nodes = Nodes::with_budget(budget);
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut most = 0;
        for _ in 0..200_000 {
            let r = next();
            // Mostly new nodes of proof-node sizes, a few large ones, and reads of recent ones.
            let id = if r % 5 == 0 { r % 2000 } else { next() };
            let len = match r % 97 {
                0 => 300_000,
                1..=5 => 2000 + (r >> 20) as usize % 20_000,
                _ => 40 + (r >> 20) as usize % 520,
            };
            let (hash, bytes) = node(id, len);
            if r % 3 == 0 {
                nodes.get(&hash);
            } else {
                nodes.intern(hash, bytes);
            }
            let stats = nodes.stats();
            assert!(stats.bytes <= budget as u64, "{stats:?}");
            assert!(nodes.map.memory_usage() <= budget / TABLE_SHARE);
            most = most.max(stats.bytes);
        }
        // Not trivially small either: eviction keeps the memory in use.
        assert!(most > budget as u64 * 3 / 4, "{most}");
        let stats = nodes.stats();
        assert!(stats.evicted > 0 && stats.hits > 0 && stats.misses > 0);
        let values: usize = nodes.map.iter().map(|(_, v)| charge(v.len())).sum();
        assert_eq!(stats.bytes as usize, values + nodes.map.memory_usage());
    }
    #[test]
    fn eviction_drops_the_least_recently_used_and_keeps_recent_nodes() {
        let mut nodes = Nodes::with_budget(1024 * 1024);
        let mut hashes = Vec::new();
        for i in 0..20_000 {
            let (hash, bytes) = node(i, 500);
            // Node 0 is read by every block, as `:code` is.
            if let Some(&first) = hashes.first() {
                assert!(
                    nodes.get(&first).is_some(),
                    "a node read each block was dropped"
                );
            }
            let (before, table) = (nodes.stats().entries, nodes.map.memory_usage());
            nodes.intern(hash, bytes);
            hashes.push(hash);
            assert!(nodes.peek(&hash).is_some(), "the newest node was not kept");
            // One node in, at most one node out, unless the table just doubled: then the nodes
            // its new memory takes go at once.
            assert!(
                nodes.stats().entries >= before || nodes.map.memory_usage() > table,
                "{before} then {:?}",
                nodes.stats()
            );
        }
        let stats = nodes.stats();
        assert!(stats.evicted > 0);
        // Not cleared at once: most of the budget is still in use, by the newest nodes.
        assert!(stats.entries > 1000, "{stats:?}");
        let kept = stats.entries as usize - 1;
        assert!(
            hashes[hashes.len() - kept..]
                .iter()
                .all(|h| nodes.peek(h).is_some())
        );
        assert!(nodes.peek(&hashes[1]).is_none());
    }
    #[test]
    fn a_full_table_drops_one_node_at_a_time() {
        // Small nodes: the table, not the bytes, is what fills up.
        let budget = 1024 * 1024;
        let mut nodes = Nodes::with_budget(budget);
        let mut most = 0;
        for i in 0..200_000 {
            let (before, table) = (nodes.stats().entries, nodes.map.memory_usage());
            let (hash, bytes) = node(i, 8);
            nodes.intern(hash, bytes);
            let stats = nodes.stats();
            assert!(
                stats.entries >= before || nodes.map.memory_usage() > table,
                "{before} then {stats:?}"
            );
            assert!(stats.bytes <= budget as u64);
            assert!(nodes.map.memory_usage() <= budget / TABLE_SHARE);
            most = most.max(stats.entries);
        }
        let stats = nodes.stats();
        assert!(stats.evicted > 0);
        assert_eq!(stats.entries, most, "{stats:?}");
        assert_eq!(stats.entries as usize, nodes.map.limiter().max_len);
    }
    #[test]
    fn a_node_larger_than_the_budget_is_not_kept_and_empties_nothing() {
        let mut nodes = Nodes::with_budget(64 * 1024);
        let (small, bytes) = node(1, 100);
        nodes.intern(small, bytes);
        let (large, bytes) = node(2, 128 * 1024);
        let shared = nodes.intern(large, bytes);
        assert_eq!(shared.len(), 128 * 1024);
        assert!(nodes.peek(&large).is_none());
        assert!(nodes.peek(&small).is_some());
    }
    #[test]
    fn concurrent_replays_share_one_allocation_within_the_budget() {
        let budget = 1024 * 1024;
        let nodes = Arc::new(parking_lot::Mutex::new(Nodes::with_budget(budget)));
        std::thread::scope(|scope| {
            for worker in 0..8u64 {
                let nodes = nodes.clone();
                scope.spawn(move || {
                    for i in 0..5000 {
                        // Half the nodes are common to all workers.
                        let id = if i % 2 == 0 { i } else { worker << 32 | i };
                        let (hash, bytes) = node(id, 200 + (i as usize % 300));
                        let mine = nodes.lock().intern(hash, bytes.clone());
                        assert_eq!(&*mine, bytes.as_slice());
                        // Another worker may have dropped it since, but never changed it.
                        if let Some(theirs) = nodes.lock().get(&hash) {
                            assert_eq!(&*theirs, bytes.as_slice());
                        }
                        assert!(nodes.lock().stats().bytes <= budget as u64);
                    }
                });
            }
        });
        let nodes = nodes.lock();
        let stats = nodes.stats();
        let values: usize = nodes.map.iter().map(|(_, v)| charge(v.len())).sum();
        assert_eq!(stats.bytes as usize, values + nodes.map.memory_usage());
        assert!(stats.bytes <= budget as u64);
    }
}
