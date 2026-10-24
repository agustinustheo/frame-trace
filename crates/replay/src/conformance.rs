//! Exercise operations absent from the eight People blocks. This is host/API
//! coverage against a complete local trie, NOT remote child-proof coverage.
#![allow(clippy::unwrap_used)]
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
    let db = ProofDb::new(None, H256::zero());
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
    assert!(
        d.spans
            .iter()
            .any(|s| s.name == "next_key_version_1" && s.duration_ns > 0)
    );
}

#[test]
fn call_and_import_expose_different_proof_size_policies() {
    let wasm = wat::parse_str(r#"(module
        (import "env" "ext_storage_proof_size_storage_proof_size_version_1" (func $size (result i64)))
        (memory (export "memory") 1)
        (global (export "__heap_base") i32 (i32.const 1024))
        (func (export "Probe_size") (param i32 i32) (result i64)
            (i64.store (i32.const 8) (call $size))
            (i64.const 34359738376)))"#).unwrap();
    let session = ReplaySession::new(None).unwrap();
    let db = ProofDb::new(None, H256::zero());
    for (context, expected) in [
        (CallContext::Offchain, u64::MAX),
        (CallContext::Onchain { import: true }, 0),
    ] {
        let (bytes, output) = execute_method(
            &db,
            H256::zero(),
            &wasm,
            None,
            &session.executor,
            "Probe_size",
            &[],
            H256::zero(),
            context,
            &Capture::new(),
        )
        .unwrap();
        assert_eq!(output, expected.to_le_bytes());
        assert_eq!(bytes, 0);
    }
}

#[test]
fn runtime_version_host_call_has_executor_backed_reader_for_calls_and_import() {
    let wasm = wat::parse_str(
        r#"(module
        (import "env" "ext_misc_runtime_version_version_1" (func $version (param i64) (result i64)))
        (memory (export "memory") 1)
        (global (export "__heap_base") i32 (i32.const 1024))
        (func (export "Probe_version") (param i32 i32) (result i64)
            (call $version (i64.const 0))))"#,
    )
    .unwrap();
    let session = ReplaySession::new(None).unwrap();
    let db = ProofDb::new(None, H256::zero());
    for context in [CallContext::Offchain, CallContext::Onchain { import: true }] {
        let (_, output) = execute_method(
            &db,
            H256::zero(),
            &wasm,
            None,
            &session.executor,
            "Probe_version",
            &[],
            H256::zero(),
            context,
            &Capture::new(),
        )
        .unwrap();
        // Invalid/empty code returns Option::None rather than trapping on a missing extension.
        assert_eq!(output, [0]);
    }
}

#[test]
fn missing_export_reports_runtime_api_method_without_metadata() {
    let wasm = wat::parse_str(
        r#"(module
        (memory (export "memory") 1)
        (global (export "__heap_base") i32 (i32.const 1024))
        (func (export "Probe_exists") (param i32 i32) (result i64) (i64.const 0)))"#,
    )
    .unwrap();
    let session = ReplaySession::new(None).unwrap();
    let error = execute_method(
        &ProofDb::new(None, H256::zero()),
        H256::zero(),
        &wasm,
        None,
        &session.executor,
        "MissingApi_method",
        &[],
        H256::zero(),
        CallContext::Offchain,
        &Capture::new(),
    )
    .unwrap_err();
    assert!(matches!(error, Error::MethodNotFound(_)), "{error}");
    assert_eq!(
        error.to_string(),
        "runtime API method MissingApi_method not found"
    );
}

#[test]
fn read_origins_observe_backend_access_recorder_warmth_and_overlay_for_all_read_kinds() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| {
        sp_io::storage::set(b"key", b"value");
        sp_io::default_child_storage::set(b"child", b"key", b"value");
    });
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let db = ProofDb::new(None, H256::zero());
    db.insert_nodes(raw.into_iter().map(|(_, (node, _))| node));
    let recorder = Recorder::default();
    let backend = TracedBackend::new(db, root, recorder.clone());
    let mut overlay = OverlayedChanges::default();
    let mut ext = sp_state_machine::Ext::new(&mut overlay, &backend, None);
    let capture = Capture::new();
    capture.0.lock().recorder = Some(recorder.clone());
    tracing::dispatcher::with_default(&tracing::Dispatch::new(capture.clone()), || {
        sp_externalities::set_and_run_with_externalities(&mut ext, || {
            let _ = sp_io::storage::get(b"key");
            let _ = sp_io::storage::get(b"key");
            let _ = sp_io::storage::exists(b"key");
            let _ = sp_externalities::with_externalities(|ext| ext.storage_hash(b"key"));
            sp_io::storage::set(b"key", b"overlay");
            let _ = sp_io::storage::get(b"key");
            let _ = sp_io::storage::exists(b"key");
            let _ = sp_externalities::with_externalities(|ext| ext.storage_hash(b"key"));
            let _ = sp_io::default_child_storage::get(b"child", b"key");
            let _ = sp_io::default_child_storage::exists(b"child", b"key");
            let _ = sp_externalities::with_externalities(|ext| {
                ext.child_storage_hash(&sp_core::storage::ChildInfo::new_default(b"child"), b"key")
            });
            sp_io::default_child_storage::set(b"child", b"key", b"overlay");
            let _ = sp_io::default_child_storage::get(b"child", b"key");
            let _ = sp_io::default_child_storage::exists(b"child", b"key");
            let _ = sp_externalities::with_externalities(|ext| {
                ext.child_storage_hash(&sp_core::storage::ChildInfo::new_default(b"child"), b"key")
            });
        });
    });
    let d = capture.0.lock();
    let reads: Vec<_> = d
        .events
        .iter()
        .filter(|e| e.fields.contains_key("origin"))
        .collect();
    assert_eq!(reads.len(), 13);
    let origins: Vec<_> = reads.iter().map(|e| e.fields["origin"].as_str()).collect();
    assert_eq!(
        origins,
        [
            "backend_cold",
            "backend_warm",
            "backend_warm",
            "backend_warm",
            "overlay",
            "overlay",
            "overlay",
            "backend_cold",
            "backend_warm",
            "backend_warm",
            "overlay",
            "overlay",
            "overlay"
        ]
    );
    let proof: usize = d
        .events
        .iter()
        .filter(|e| e.fields.get("method").is_some_and(|m| m != "BackendRead"))
        .filter_map(|e| e.fields.get("proof_bytes_added"))
        .map(|n| n.parse::<usize>().unwrap())
        .sum();
    assert_eq!(proof, recorder.estimate_encoded_size());
    assert!(
        reads[0].fields["proof_bytes_added"]
            .parse::<usize>()
            .unwrap()
            > 0
    );
}

#[test]
fn recorder_rollback_keeps_block_lifetime_warmth_but_reports_new_marginal_growth() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| sp_io::storage::set(b"key", b"value"));
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let db = ProofDb::new(None, H256::zero());
    db.insert_nodes(raw.into_iter().map(|(_, (node, _))| node));
    let recorder = Recorder::default();
    let backend = TracedBackend::new(db, root, recorder.clone());
    let mut overlay = OverlayedChanges::default();
    let mut extensions = Extensions::default();
    extensions.register(ProofSizeExt::new(recorder.clone()));
    let mut ext = sp_state_machine::Ext::new(&mut overlay, &backend, Some(&mut extensions));
    let capture = Capture::new();
    capture.0.lock().recorder = Some(recorder.clone());
    tracing::dispatcher::with_default(&tracing::Dispatch::new(capture.clone()), || {
        sp_externalities::set_and_run_with_externalities(&mut ext, || {
            // The SDK ignores runtime proof transactions; emulate an executor/host
            // recorder transaction to exercise actual recorder shrinkage as well.
            recorder.start_transaction();
            sp_io::storage::start_transaction();
            let _ = sp_io::storage::get(b"key");
            recorder.rollback_transaction().unwrap();
            sp_io::storage::rollback_transaction();
            let _ = sp_io::storage::get(b"key");
        });
    });
    let d = capture.0.lock();
    let reads: Vec<_> = d
        .events
        .iter()
        .filter(|e| e.fields.get("method").is_some_and(|m| m == "Get"))
        .collect();
    assert_eq!(reads[0].fields["origin"], "backend_cold");
    assert_eq!(reads[1].fields["origin"], "backend_warm");
    assert!(
        reads[1].fields["proof_bytes_added"]
            .parse::<usize>()
            .unwrap()
            > 0
    );
}
