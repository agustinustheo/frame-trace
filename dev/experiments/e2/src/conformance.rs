//! Exercise operations absent from the eight People blocks. This is host/API
//! coverage against a complete local trie, NOT remote child-proof coverage.
use super::*;
use sp_core::storage::StateVersion;
use std::collections::BTreeSet;

#[test]
fn captures_keyless_child_iteration_and_overlay_accesses() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| {
        sp_io::storage::set(b"a", b"one");
        sp_io::storage::set(b"b", b"two");
        sp_io::default_child_storage::set(b"child", b"c1", b"three");
        sp_io::default_child_storage::set(b"child", b"c2", b"four");
    });
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let db = ProofDb::new(Rpc::new("http://127.0.0.1:1".into()).unwrap(), H256::zero());
    db.insert_nodes(raw.into_iter().map(|(_, (node, _))| node));
    db.online.store(false, Ordering::Relaxed);
    let backend = TracedBackend::new(db, root, Recorder::default());
    let mut overlay = OverlayedChanges::default();
    let mut ext = sp_state_machine::Ext::new(&mut overlay, &backend, None);
    let capture = Capture::new();
    let dispatch = tracing::Dispatch::new(capture.clone());
    tracing::dispatcher::with_default(&dispatch, || {
        sp_externalities::set_and_run_with_externalities(&mut ext, || {
            sp_io::storage::set(b"aa", b"overlay");
            assert_eq!(sp_io::storage::get(b"aa").unwrap().as_ref(), b"overlay");
            assert_eq!(sp_io::storage::next_key(b"a"), Some(b"aa".to_vec()));
            assert_eq!(
                sp_io::default_child_storage::next_key(b"child", b"c1"),
                Some(b"c2".to_vec())
            );
            let _ = sp_io::default_child_storage::clear_prefix(b"child", b"c1", None);
            let _ = sp_io::default_child_storage::root(b"child", StateVersion::V1);
            let _ = sp_io::default_child_storage::storage_kill(b"child", None);
            let _ = sp_io::storage::clear_prefix(b"b", None);
            let _ = sp_io::storage::root(StateVersion::V1);
        });
    });
    let d = capture.0.lock();
    let methods: BTreeSet<_> = d
        .events
        .iter()
        .filter_map(|e| e.fields.get("method").map(String::as_str))
        .collect();
    for expected in [
        "Put",
        "Get",
        "ClearPrefix",
        "ChildKill",
        "ChildClearPrefix",
        "ChildStorageRoot",
        "StorageRoot",
        "BackendNextKey",
        "BackendChildNextKey",
        "BackendIterNextKey",
    ] {
        assert!(
            methods.contains(expected),
            "missing {expected}: {methods:?}"
        );
    }
    assert!(d
        .spans
        .iter()
        .any(|s| s.name == "next_key_version_1" && s.duration_ns > 0));
}
