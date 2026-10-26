#![allow(clippy::unwrap_used)]
use super::*;
use hash_db::{EMPTY_PREFIX, HashDB};
use serde_json::json;
use sp_core::storage::{ChildInfo, StateVersion};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
};

type Requests = Arc<Mutex<Vec<Value>>>;
fn server(
    handler: impl Fn(&Value) -> Value + Send + Sync + 'static,
) -> (Rpc, Requests, Arc<AtomicBool>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let handler = Arc::new(handler);
    let worker = thread::spawn(move || {
        let mut workers = Vec::new();
        while !stopped.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("accept: {e}"),
            };
            let handler = handler.clone();
            let captured = captured.clone();
            workers.push(thread::spawn(move || {
                // Accepted sockets inherit O_NONBLOCK on macOS. Use blocking request/response I/O.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        len = value.trim().parse().unwrap();
                    }
                }
                let mut bytes = vec![0; len];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                captured.lock().unwrap().push(request.clone());
                let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&request)));
                let body = match &response {
                    Ok(result) => json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                    Err(payload) => json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32099,"message":format!("mock handler assertion failed: {}", payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied()).unwrap_or("panic"))}}),
                }.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
                if let Err(payload) = response { std::panic::resume_unwind(payload); }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    (Rpc::new(&url).unwrap(), requests, stop, worker)
}
fn stop(stop: Arc<AtomicBool>, thread: thread::JoinHandle<()>) {
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
}

#[test]
fn allowlist_rejects_unsafe_and_unproved_storage_and_unpinned_metadata() {
    let rpc = Rpc::new("http://127.0.0.1:1").unwrap();
    for method in [
        "state_traceBlock",
        "state_getStorage",
        "author_submitExtrinsic",
        "state_getKeysPaged",
    ] {
        assert!(matches!(
            rpc.call(method, json!([])),
            Err(Error::MethodNotAllowed(_))
        ));
    }
    assert!(rpc.call("state_getMetadata", json!([])).is_err());
    assert_eq!(rpc.count(), 0);
}

#[test]
fn authenticates_remote_child_proofs_and_offline_child_root_changes() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| {
        for (i, child) in [b"child-one".as_slice(), b"child-two".as_slice()]
            .into_iter()
            .enumerate()
        {
            sp_io::default_child_storage::set(child, b"c1", &[i as u8 + 1; 80]);
            sp_io::default_child_storage::set(child, b"c2", &[2; 80]);
            sp_io::default_child_storage::set(child, b"d1", &[3; 80]);
        }
    });
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let mut memory = sp_trie::MemoryDB::<Blake2Hasher>::default();
    for (_, (node, _)) in raw {
        HashDB::insert(&mut memory, EMPTY_PREFIX, &node);
    }
    let trie = sp_state_machine::TrieBackendBuilder::new(memory, root).build();
    let parent = H256::repeat_byte(7);
    let (rpc, requests, halt, worker) = server(move |request| {
        let method = request["method"].as_str().unwrap();
        let proof = match method {
            "state_getReadProof" => {
                assert_eq!(request["params"][1], json!(parent));
                let keys: Vec<_> = request["params"][0]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| unhex(v.as_str().unwrap()).unwrap())
                    .collect();
                sp_state_machine::prove_read_on_trie_backend(&trie, &keys).unwrap()
            }
            "state_getChildReadProof" => {
                assert_eq!(request["params"][2], json!(parent));
                let prefixed = unhex(request["params"][0].as_str().unwrap()).unwrap();
                let id = prefixed.strip_prefix(b":child_storage:default:").unwrap();
                assert!([b"child-one".as_slice(), b"child-two".as_slice()].contains(&id));
                // Keys are child-local, never SDK keyspace-prefixed. Serve only the
                // requested paths, forcing demand paging during iteration and root collapse.
                let child = ChildInfo::new_default(id);
                let keys: Vec<_> = request["params"][1]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| unhex(v.as_str().unwrap()).unwrap())
                    .collect();
                sp_state_machine::prove_child_read_on_trie_backend(&trie, &child, &keys).unwrap()
            }
            other => panic!("unexpected method {other}"),
        };
        json!({"at": parent, "proof": proof.into_iter_nodes().map(|n| rpc::hex0(&n)).collect::<Vec<_>>()})
    });
    let db = ProofDb::new(Some(rpc.clone()), parent);
    let backend = TracedBackend::new(db.clone(), root, Recorder::default());
    for (i, id) in [b"child-one".as_slice(), b"child-two".as_slice()]
        .into_iter()
        .enumerate()
    {
        let child = ChildInfo::new_default(id);
        assert_eq!(
            backend.child_storage(&child, b"c1").unwrap(),
            Some(vec![i as u8 + 1; 80])
        );
        assert_eq!(
            backend.next_child_storage_key(&child, b"c1").unwrap(),
            Some(b"c2".to_vec())
        );
    }
    let child = ChildInfo::new_default(b"child-one");
    let delta = [(b"c1".as_slice(), None), (b"c2".as_slice(), None)];
    let (discovered, empty, _) =
        backend.child_storage_root(&child, delta.into_iter(), StateVersion::V1);
    assert!(!empty);
    let calls = rpc.count();
    let requests = requests.lock().unwrap();
    for id in [b"child-one".as_slice(), b"child-two".as_slice()] {
        assert!(
            requests
                .iter()
                .any(|r| r["method"] == "state_getChildReadProof"
                    && r["params"][0]
                        == rpc::hex0(ChildInfo::new_default(id).prefixed_storage_key().as_slice())
                    && r["params"][1] == json!(["0x6331"]))
        );
    }
    drop(requests);
    db.online.store(false, Ordering::SeqCst);
    let backend = TracedBackend::new(db.clone(), root, Recorder::default());
    let (changed, empty, _) =
        backend.child_storage_root(&child, delta.into_iter(), StateVersion::V1);
    assert!(!empty);
    assert_eq!(changed, discovered);
    assert_eq!(rpc.count(), calls);
    assert!(db.failure.lock().is_none());
    stop(halt, worker);
}

#[test]
fn concurrent_node_misses_share_one_fetch() {
    let node = vec![0u8]; // SDK's canonical empty trie node.
    let root = H256::from(sp_io::hashing::blake2_256(&node));
    // Use a nonempty serialized trie node for a genuine miss (empty roots are synthesized).
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| sp_io::storage::set(b"key", b"value"));
    seed.commit_all().unwrap();
    let (raw, state_root) = seed.into_raw_snapshot();
    assert_ne!(state_root, root);
    let nodes: Vec<_> = raw
        .into_iter()
        .map(|(_, (node, _))| rpc::hex0(&node))
        .collect();
    let parent = H256::repeat_byte(9);
    let (rpc, _, halt, worker) = server(move |_| json!({"at":parent,"proof":nodes}));
    let db = ProofDb::new(Some(rpc.clone()), parent);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let db = db.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                sp_state_machine::TrieBackendStorage::get(&db, &state_root, EMPTY_PREFIX)
                    .unwrap()
                    .unwrap()
            })
        })
        .collect();
    for worker in workers {
        assert!(!worker.join().unwrap().is_empty());
    }
    assert_eq!(rpc.count(), 1);
    stop(halt, worker);
}

#[test]
fn rejects_proofs_for_a_different_parent() {
    let parent = H256::repeat_byte(3);
    let (rpc, _, halt, worker) = server(move |_| json!({"at": H256::repeat_byte(4),"proof":[]}));
    let db = ProofDb::new(Some(rpc), parent);
    assert!(
        db.fetch(None, &[b"key".to_vec()])
            .unwrap_err()
            .to_string()
            .contains("different parent")
    );
    stop(halt, worker);
}

#[test]
fn websocket_transport_obeys_json_rpc_and_pinning() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let mut ws = tungstenite::accept(listener.accept().unwrap().0).unwrap();
        let request: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "chain_getBlockHash");
        ws.send(tungstenite::Message::Text(
            json!({"jsonrpc":"2.0","id":request["id"],"result":H256::repeat_byte(5)})
                .to_string()
                .into(),
        ))
        .unwrap();
    });
    let rpc = Rpc::new(&url).unwrap();
    assert_eq!(
        rpc.call("chain_getBlockHash", json!([1])).unwrap(),
        json!(H256::repeat_byte(5))
    );
    assert_eq!(rpc.count(), 1);
    worker.join().unwrap();
}

#[test]
fn online_acquisition_pins_context_and_measures_with_zero_rpc() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/replay/people-transfer");
    let artifact = Artifact::load(&path).unwrap();
    let expected = artifact.clone();
    let mut memory = sp_trie::MemoryDB::<Blake2Hasher>::default();
    for node in &artifact.proof_nodes {
        HashDB::insert(&mut memory, EMPTY_PREFIX, &unhex(node).unwrap());
    }
    let parent_header: Header = serde_json::from_value(artifact.parent_header.clone()).unwrap();
    let trie =
        sp_state_machine::TrieBackendBuilder::new(memory, *parent_header.state_root()).build();
    let parent = artifact.block["header"]["parentHash"].clone();
    let (rpc, requests, halt, worker) = server(move |request| {
        match request["method"].as_str().unwrap() {
            "chain_getBlockHash" => {
                assert_eq!(request["params"], json!([2]));
                json!(artifact.block_hash)
            }
            "chain_getBlock" => {
                assert_eq!(request["params"], json!([artifact.block_hash]));
                json!({"block":artifact.block})
            }
            "chain_getHeader" => {
                assert_eq!(request["params"], json!([parent]));
                artifact.parent_header.clone()
            }
            "state_getRuntimeVersion" => {
                assert_eq!(request["params"], json!([parent]));
                artifact.runtime_version.clone()
            }
            "state_getMetadata" => {
                assert_eq!(request["params"], json!([parent]));
                json!("") // No speculative hints: exercise real execution-driven demand paging.
            }
            "state_getReadProof" => {
                assert_eq!(request["params"][1], parent);
                let keys: Vec<_> = request["params"][0]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| unhex(v.as_str().unwrap()).unwrap())
                    .collect();
                // Legacy fixtures lack some metadata-only branches. Return their partial proof;
                // metadata can be unavailable while Core_execute_block still authenticates.
                match sp_state_machine::prove_read_on_trie_backend(&trie, &keys) {
                    Ok(proof) => {
                        json!({"at":parent,"proof":proof.into_iter_nodes().map(|n| rpc::hex0(&n)).collect::<Vec<_>>()})
                    }
                    Err(_) => json!({"at":parent,"proof":[]}),
                }
            }
            method => panic!("unexpected RPC {method}"),
        }
    });
    let session = ReplaySession::new(None)
        .unwrap()
        .with_speculative_discovery(false);
    let result = online_with_session(&rpc, "2", &session).unwrap();
    assert!(result.root_check);
    assert_eq!(result.measured_rpc_count, 0);
    assert!(
        result.rpc_count > 10,
        "must demand page multiple requested paths"
    );
    assert_eq!(result.artifact.block_hash, expected.block_hash);
    assert_eq!(requests.lock().unwrap().len(), result.rpc_count);
    let offline = replay(result.artifact, None).unwrap();
    assert_eq!(offline.expected_state_root, offline.computed_state_root);
    stop(halt, worker);
}

#[test]
fn measured_run_rpc_guard_detects_network_activity() {
    let (rpc, requests, halt, worker) = server(|_| json!(null));
    assert_eq!(without_rpc(Some(&rpc), || Ok(())).unwrap(), ((), 0));
    let error = without_rpc(Some(&rpc), || rpc.call("chain_getHeader", json!([])))
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed replay used RPC"), "{error}");
    assert_eq!(requests.lock().unwrap().len(), 1);
    stop(halt, worker);
}

#[test]
fn proof_prefetch_batches_keys_and_bounds_parallel_requests() {
    use std::sync::atomic::AtomicUsize;
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (a, p) = (active.clone(), peak.clone());
    let parent = H256::repeat_byte(11);
    let (rpc, requests, halt, worker) = server(move |request| {
        let n = a.fetch_add(1, Ordering::SeqCst) + 1;
        p.fetch_max(n, Ordering::SeqCst);
        assert!(request["params"][0].as_array().unwrap().len() <= 32);
        thread::sleep(std::time::Duration::from_millis(50));
        a.fetch_sub(1, Ordering::SeqCst);
        json!({"at":parent,"proof":[]})
    });
    let db = ProofDb::new(Some(rpc), parent);
    db.prefetch(
        &(0u32..300)
            .map(|n| n.to_le_bytes().to_vec())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 10);
    assert!((2..=8).contains(&peak.load(Ordering::SeqCst)));
    stop(halt, worker);
}

#[test]
fn shared_nodes_are_reauthenticated_at_each_blocks_root() {
    let session = ReplaySession::new(None).unwrap();
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| sp_io::storage::set(b"key", b"value"));
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let nodes: Vec<_> = raw.into_iter().map(|(_, (node, _))| node).collect();
    let rpc = Rpc::new("http://127.0.0.1:1").unwrap();
    let first = ProofDb::with_session(Some(rpc.clone()), H256::repeat_byte(1), session.clone());
    first.insert_nodes(nodes);
    let second = ProofDb::with_session(Some(rpc.clone()), H256::repeat_byte(2), session);
    let backend = TracedBackend::new(second, root, Recorder::default());
    assert_eq!(backend.storage(b"key").unwrap(), Some(b"value".to_vec()));
    assert_eq!(rpc.count(), 0);
    // The same nodes cannot authenticate a different root.
    let backend = TracedBackend::new(first.clone(), H256::repeat_byte(99), Recorder::default());
    first.online.store(false, Ordering::SeqCst);
    assert!(backend.storage(b"key").is_err());
    assert!(
        first
            .failure
            .lock()
            .as_ref()
            .unwrap()
            .contains("missing/invalid")
    );
}

#[test]
fn unknown_host_import_error_names_the_import() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/replay/people-transfer");
    let artifact = Artifact::load(&path).unwrap();
    let (block, parent) = artifact.validate().unwrap();
    let db = ProofDb::new(None, parent.hash());
    db.insert_nodes(artifact.proof_nodes.iter().map(|s| unhex(s).unwrap()));
    let wasm = wat::parse_str(
        r#"(module
        (import "env" "ext_frame_trace_unknown_version_1" (func))
        (memory (export "memory") 1)
        (global (export "__heap_base") i32 (i32.const 1024))
        (func (export "Core_execute_block") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    let session = ReplaySession::new(None).unwrap();
    let error = execute(
        &db,
        *parent.state_root(),
        &wasm,
        None,
        &session.executor,
        &block,
        &Capture::new(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("ext_frame_trace_unknown_version_1"),
        "{error}"
    );
}

#[test]
fn stalled_websocket_handshake_has_deadline_and_can_be_cancelled() {
    use std::time::{Duration, Instant};
    for cancelled in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let token = rpc::CancellationToken::new();
        let rpc = Rpc::with_options(&url, 1, Duration::from_secs(4), token.clone()).unwrap();
        let (ready, connected) = std::sync::mpsc::channel();
        let node = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            ready.send(()).unwrap();
            stream.set_nonblocking(false).unwrap();
            let mut buffer = Vec::new();
            let _ = stream.read_to_end(&mut buffer); // client dropping the cancelled future closes it
        });
        let cancellation = thread::spawn(move || {
            connected.recv_timeout(Duration::from_secs(15)).unwrap();
            if cancelled {
                token.cancel();
            }
        });
        let start = Instant::now();
        let error = rpc.call("chain_getHeader", json!([])).unwrap_err();
        if cancelled {
            assert!(matches!(error, Error::Cancelled), "{error}");
        } else {
            assert!(error.is_transient(), "{error}");
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        cancellation.join().unwrap();
        node.join().unwrap();
    }
}

#[test]
fn persistent_node_cache_reuses_and_heals_corrupt_nodes() {
    use sp_state_machine::TrieBackendStorage;
    let dir = std::env::temp_dir().join(format!("frame-trace-node-cache-{}", std::process::id()));
    let session = ReplaySession::new(Some(&dir)).unwrap();
    let bytes = vec![1, 2, 3, 4];
    let hash = H256::from(sp_io::hashing::blake2_256(&bytes));
    let rpc = Rpc::new("http://127.0.0.1:1").unwrap();
    let first = ProofDb::with_session(Some(rpc.clone()), H256::repeat_byte(1), session);
    first.insert_nodes([bytes.clone()]);
    first.persist().unwrap();
    let second = ProofDb::with_session(
        Some(rpc.clone()),
        H256::repeat_byte(2),
        ReplaySession::new(Some(&dir)).unwrap(),
    );
    assert_eq!(
        TrieBackendStorage::get(&second, &hash, EMPTY_PREFIX).unwrap(),
        Some(bytes)
    );
    assert_eq!(rpc.count(), 0);
    let path = dir.join("nodes").join(format!("{hash:x}"));
    std::fs::write(&path, b"corrupt").unwrap();
    let parent = H256::repeat_byte(3);
    let (healing_rpc, _, halt, worker) =
        server(move |_| json!({"at":parent,"proof":[rpc::hex0(&[1,2,3,4])]}));
    let third = ProofDb::with_session(
        Some(healing_rpc.clone()),
        parent,
        ReplaySession::new(Some(&dir)).unwrap(),
    );
    assert_eq!(
        TrieBackendStorage::get(&third, &hash, EMPTY_PREFIX).unwrap(),
        Some(vec![1, 2, 3, 4])
    );
    third.persist().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), vec![1, 2, 3, 4]);
    assert_eq!(healing_rpc.count(), 1);
    // Persist also replaces corruption which has not yet been read by this block.
    std::fs::write(&path, b"bad again").unwrap();
    third.persist().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), vec![1, 2, 3, 4]);
    stop(halt, worker);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn branch_proofs_batch_all_immediate_sibling_paths() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| {
        for key in 0u8..=255 {
            sp_io::storage::set(&[key, 1], &[key; 80]);
        }
    });
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let mut memory = sp_trie::MemoryDB::<Blake2Hasher>::default();
    for (_, (node, _)) in raw {
        HashDB::insert(&mut memory, EMPTY_PREFIX, &node);
    }
    let trie = sp_state_machine::TrieBackendBuilder::new(memory, root).build();
    let parent = H256::repeat_byte(12);
    let (rpc, requests, halt, worker) = server(move |request| {
        let keys: Vec<_> = request["params"][0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| unhex(v.as_str().unwrap()).unwrap())
            .collect();
        let proof = sp_state_machine::prove_read_on_trie_backend(&trie, &keys).unwrap();
        json!({"at":parent,"proof":proof.into_iter_nodes().map(|n| rpc::hex0(&n)).collect::<Vec<_>>()})
    });
    let db = ProofDb::new(Some(rpc), parent);
    sp_state_machine::TrieBackendStorage::get(&db, &root, EMPTY_PREFIX)
        .unwrap()
        .unwrap();
    let requests = requests.lock().unwrap();
    let keys = requests[0]["params"][0].as_array().unwrap();
    assert_eq!(keys.len(), 17);
    for nibble in 0..16 {
        assert!(keys.contains(&json!(rpc::hex0(&[nibble << 4]))));
    }
    assert_eq!(proof::sibling_paths(&[0xa0], true).len(), 16);
    assert!(proof::sibling_paths(&[0xa0], true).contains(&vec![0xf0]));
    drop(requests);
    stop(halt, worker);
}

#[test]
fn metadata_fallback_only_accepts_unavailable_or_pruned() {
    assert!(
        metadata_result(Err(Error::Unavailable("pruned".into())))
            .unwrap()
            .is_empty()
    );
    assert!(metadata_result(Ok(Value::Null)).unwrap().is_empty());
    for error in [
        Error::Cancelled,
        Error::TransientRpc("retries exhausted".into()),
        Error::Rpc("malformed".into()),
    ] {
        assert!(metadata_result(Err(error)).is_err());
    }
    assert!(metadata_result(Ok(json!({"bad":"metadata"}))).is_err());
    assert_eq!(metadata_result(Ok(json!("0x1234"))).unwrap(), "0x1234");
}

#[test]
fn prefetch_skips_cached_paths_only_when_covered_from_the_new_root() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| {
        for key in 0..=255u8 {
            sp_io::storage::set(&[key, 1], &[key; 80]);
        }
    });
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let nodes: Vec<_> = raw.into_iter().map(|(_, (node, _))| node).collect();
    let all = nodes.clone();
    let parent = H256::repeat_byte(30);
    let (rpc, requests, halt, worker) = server(
        move |_| json!({"at":parent,"proof":all.iter().map(|n| rpc::hex0(n)).collect::<Vec<_>>()}),
    );
    let session = ReplaySession::new(None).unwrap();
    let first = ProofDb::with_session(Some(rpc.clone()), parent, session.clone());
    first.insert_nodes(nodes);
    let warm = ProofDb::with_session(Some(rpc.clone()), parent, session.clone());
    warm.set_root(root);
    let keys: Vec<_> = (0..=255u8).map(|k| vec![k, 1]).collect();
    warm.prefetch(&keys).unwrap();
    warm.fetch(None, &[vec![255, 255]]).unwrap(); // authenticated absence is also covered
    assert_eq!(rpc.count(), 0);
    assert!(
        warm.export().is_empty(),
        "coverage probes must not bloat artifacts"
    );
    let backend = sp_state_machine::TrieBackendBuilder::new(warm.clone(), root).build();
    assert_eq!(backend.storage(&[42, 1]).unwrap(), Some(vec![42; 80]));
    assert!(
        !warm.export().is_empty(),
        "execution imports actually used nodes"
    );
    let changed = ProofDb::with_session(Some(rpc.clone()), parent, session);
    changed.set_root(H256::repeat_byte(99)); // old nodes cannot authenticate a new root
    changed.fetch(None, &[vec![1, 1]]).unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    stop(halt, worker);
}

#[test]
fn speculative_pending_preserves_top_and_both_child_namespaces() {
    use sp_state_machine::TrieBackendStorage;
    let parent = H256::repeat_byte(31);
    let (rpc, requests, halt, worker) = server(move |_| json!({"at":parent,"proof":[]}));
    let db = ProofDb::new(Some(rpc), parent);
    db.speculative.store(true, Ordering::SeqCst);
    let missing = H256::repeat_byte(99);
    assert!(TrieBackendStorage::get(&db, &missing, EMPTY_PREFIX).is_err());
    for id in [b"one".as_slice(), b"two".as_slice()] {
        let child = ChildInfo::new_default(id);
        proof::with_child(&child, || {
            assert!(TrieBackendStorage::get(&db, &missing, (child.keyspace(), None)).is_err());
        });
    }
    db.acquire_pending().unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests.iter().any(|r| r["method"] == "state_getReadProof"));
    for id in [b"one".as_slice(), b"two".as_slice()] {
        assert!(
            requests
                .iter()
                .any(|r| r["method"] == "state_getChildReadProof"
                    && r["params"][0]
                        == rpc::hex0(ChildInfo::new_default(id).prefixed_storage_key().as_slice()))
        );
    }
    drop(requests);
    stop(halt, worker);
}

#[test]
fn disk_cache_limit_evicts_oldest_nodes_and_wasm_together() {
    let dir = std::env::temp_dir().join(format!("frame-trace-eviction-{}", std::process::id()));
    let session = ReplaySession::with_cache_max_bytes(Some(&dir), 10).unwrap();
    let old = dir.join("nodes/old");
    let newer = dir.join("wasm/new");
    std::fs::write(&old, [0; 8]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
        .unwrap();
    std::fs::write(&newer, [0; 8]).unwrap();
    session.prune_disk_cache().unwrap();
    assert!(!old.exists());
    assert!(newer.exists());
    let abandoned = dir.join("nodes/abandoned.tmp");
    std::fs::write(&abandoned, [0; 20]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&abandoned)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
        .unwrap();
    // Even a failed replay runs maintenance and preserves the original execution error.
    let mut artifact = Artifact::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/replay/people-transfer"),
    )
    .unwrap();
    artifact.format_version = 999;
    assert!(
        replay_with_session(artifact, None, &session)
            .unwrap_err()
            .to_string()
            .contains("unsupported artifact version")
    );
    assert!(!abandoned.exists());
    let zero = ReplaySession::with_cache_max_bytes(Some(&dir), 0).unwrap();
    zero.prune_disk_cache().unwrap();
    assert!(!newer.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn warm_session_fetches_changed_external_runtime_code_at_its_full_key() {
    fn trie(
        code: u8,
    ) -> (
        sp_state_machine::TrieBackend<sp_trie::MemoryDB<Blake2Hasher>, Blake2Hasher>,
        Vec<Vec<u8>>,
    ) {
        let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
        seed.execute_with(|| sp_io::storage::set(b":code", &[code; 1024]));
        seed.commit_all().unwrap();
        let (raw, root) = seed.into_raw_snapshot();
        let mut memory = sp_trie::MemoryDB::<Blake2Hasher>::default();
        let mut nodes = Vec::new();
        for (_, (node, _)) in raw {
            HashDB::insert(&mut memory, EMPTY_PREFIX, &node);
            nodes.push(node);
        }
        (
            sp_state_machine::TrieBackendBuilder::new(memory, root).build(),
            nodes,
        )
    }
    let (_, old) = trie(1);
    let (new, _) = trie(2);
    let root = *new.root();
    let parent = H256::repeat_byte(33);
    let (rpc, requests, halt, worker) = server(move |request| {
        let keys: Vec<_> = request["params"][0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| unhex(v.as_str().unwrap()).unwrap())
            .collect();
        let proof = sp_state_machine::prove_read_on_trie_backend(&new, &keys).unwrap();
        json!({"at":parent,"proof":proof.into_iter_nodes().map(|n| rpc::hex0(&n)).collect::<Vec<_>>()})
    });
    let db = ProofDb::new(Some(rpc), parent);
    db.insert_nodes(old);
    db.set_root(root);
    let backend = TracedBackend::new(db, root, Recorder::default());
    assert_eq!(backend.storage(b":code").unwrap(), Some(vec![2; 1024]));
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["params"][0] == json!([rpc::hex0(b":code")]))
    );
    stop(halt, worker);
}

#[test]
fn runtime_calls_pin_state_and_report_matching_and_mismatching_oracles() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/replay/call-people-nonce");
    let original = Artifact::load(&path).unwrap();
    let fixture = original.clone();
    let (rpc, requests, halt, worker) = server(move |request| {
        let hash = fixture.block_hash;
        match request["method"].as_str().unwrap() {
            "chain_getHeader" => fixture.parent_header.clone(),
            "chain_getFinalizedHead" | "chain_getBlockHash" => json!(hash),
            "state_getRuntimeVersion" => fixture.runtime_version.clone(),
            "state_getMetadata" => json!(fixture.metadata),
            "state_getReadProof" => json!({"at":hash,"proof":fixture.proof_nodes}),
            "state_call" if request["params"][0] == "Metadata_metadata_at_version" => {
                json!(rpc::hex0(&Some(unhex(&fixture.metadata).unwrap()).encode()))
            }
            "state_call" => json!(fixture.call.as_ref().unwrap().result),
            other => panic!("unexpected method {other}"),
        }
    });
    for selector in [
        "best".to_owned(),
        "finalized".into(),
        "1".into(),
        format!("{:?}", original.block_hash),
    ] {
        let prepared = call::prepare(&rpc, &selector, "AccountNonceApi_account_nonce").unwrap();
        assert_eq!(prepared.block_hash, original.block_hash);
        assert_eq!(prepared.parent_header, original.parent_header);
    }
    let mut artifact = call::prepare(&rpc, "1", "AccountNonceApi_account_nonce").unwrap();
    artifact.call.as_mut().unwrap().data = original.call.as_ref().unwrap().data.clone();
    let replay = call::replay(
        artifact,
        Some(rpc.clone()),
        &ReplaySession::new(None).unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(replay.oracle_match, Some(true));
    assert_eq!(replay.measured_rpc_count, 0);
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "state_call")
            .all(|r| r["params"][2] == json!(original.block_hash))
    );
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "state_getReadProof")
            .all(|r| r["params"][1] == json!(original.block_hash))
    );
    stop(halt, worker);
    let hash = original.block_hash;
    let nodes = original.proof_nodes.clone();
    let (rpc, _, halt, worker) = server(move |request| {
        if request["method"] == "state_getReadProof" {
            return json!({"at":hash,"proof":nodes});
        }
        assert_eq!(request["method"], "state_call");
        assert_eq!(request["params"][2], json!(hash));
        json!("0x01000000")
    });
    let replay = call::replay(
        original,
        Some(rpc),
        &ReplaySession::new(None).unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(replay.oracle_match, Some(false));
    let repeated = call::replay(
        replay.artifact,
        None,
        &ReplaySession::new(None).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(repeated.oracle_match, Some(false));
    stop(halt, worker);
}

#[test]
fn state_call_requires_explicit_hash_and_valid_hex_before_network() {
    let rpc = Rpc::new("http://127.0.0.1:1").unwrap();
    for params in [
        json!([]),
        json!(["method", "0x"]),
        json!(["method", "0x", "best"]),
        json!(["method", "bad hex", H256::zero()]),
    ] {
        assert!(matches!(rpc.call("state_call", params), Err(Error::Rpc(_))));
    }
    assert_eq!(rpc.count(), 0);
}

#[test]
fn prediction_history_includes_miss_paths_and_expires_after_three_replays() {
    use sp_state_machine::TrieBackendStorage;
    let parent = H256::repeat_byte(51);
    let (rpc, _, halt, worker) = server(move |_| json!({"at":parent,"proof":[]}));
    let session = ReplaySession::new(None).unwrap();
    let db = ProofDb::with_session(Some(rpc), parent, session.clone());
    // Root/iteration requests have no full storage-key hint. Keep their sibling paths too.
    let _ = TrieBackendStorage::get(&db, &H256::repeat_byte(52), (&[0xab], None));
    db.remember_key(b"complete storage key");
    db.persist().unwrap();
    let hints = session.predicted_keys();
    assert!(hints.contains(b"complete storage key".as_slice()));
    assert!(hints.contains(&vec![0xab, 0x50]));
    for i in 0..3 {
        session.remember_paths([vec![i]].into_iter().collect());
    }
    assert!(
        !session
            .predicted_keys()
            .contains(b"complete storage key".as_slice())
    );
    stop(halt, worker);
}

#[test]
fn wrong_predictions_preserve_results_and_unused_fetched_nodes_stay_out_of_artifacts() {
    let mut seed = sp_state_machine::TestExternalities::<Blake2Hasher>::default();
    seed.execute_with(|| {
        for key in 0..=255u8 {
            sp_io::storage::set(&[key, 1], &[key; 80]);
        }
    });
    seed.commit_all().unwrap();
    let (raw, root) = seed.into_raw_snapshot();
    let nodes: Vec<_> = raw.into_iter().map(|(_, (node, _))| node).collect();
    let parent = H256::repeat_byte(61);
    let (rpc, _, halt, worker) = server(
        move |_| json!({"at":parent,"proof":nodes.iter().map(|node| rpc::hex0(node)).collect::<Vec<_>>()}),
    );
    let session = ReplaySession::new(None).unwrap();
    session.remember_paths((0..255u8).map(|i| vec![i, 255]).collect());
    let db = ProofDb::with_session(Some(rpc), parent, session.clone());
    db.set_root(root);
    db.prefetch(&session.predicted_keys().into_iter().collect::<Vec<_>>())
        .unwrap();
    assert!(db.export().is_empty());
    db.mark_bootstrap();
    db.begin_measured();
    db.online.store(false, Ordering::SeqCst);
    let backend = sp_state_machine::TrieBackendBuilder::new(db.clone(), root).build();
    assert_eq!(backend.storage(&[42, 1]).unwrap(), Some(vec![42; 80]));
    let exported = db.export();
    assert!(
        exported.len() < 20,
        "only the requested path is needed, got {}",
        exported.len()
    );
    let offline = ProofDb::new(None, parent);
    offline.insert_nodes(exported.iter().map(|node| unhex(node).unwrap()));
    let backend = sp_state_machine::TrieBackendBuilder::new(offline, root).build();
    assert_eq!(backend.storage(&[42, 1]).unwrap(), Some(vec![42; 80]));
    stop(halt, worker);
}

mod runtime_override {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/replay")
            .join(name)
    }
    /// The traced People runtime is the largest value in the `people-traced` artifact.
    fn traced_wasm() -> Vec<u8> {
        Artifact::load(&fixture("people-traced"))
            .unwrap()
            .proof_nodes
            .iter()
            .map(|n| unhex(n).unwrap())
            .max_by_key(Vec::len)
            .unwrap()
    }
    fn leb(bytes: &[u8], at: &mut usize) -> usize {
        let (mut value, mut shift) = (0usize, 0);
        loop {
            let byte = bytes[*at];
            *at += 1;
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }
    /// Decompress and add one to the embedded `runtime_version` spec_version, in place.
    fn bump_spec_version(code: &[u8]) -> Vec<u8> {
        let mut wasm = sp_maybe_compressed_blob::decompress(code, 50 << 20)
            .unwrap()
            .into_owned();
        let mut at = 8;
        while at < wasm.len() {
            let id = wasm[at];
            at += 1;
            let size = leb(&wasm, &mut at);
            let end = at + size;
            if id == 0 {
                let mut name_at = at;
                let len = leb(&wasm, &mut name_at);
                if &wasm[name_at..name_at + len] == b"runtime_version" {
                    let mut p = &wasm[name_at + len..end];
                    let before = p.len();
                    for _ in 0..2 {
                        let n = codec::Compact::<u32>::decode(&mut p).unwrap().0 as usize;
                        p = &p[n..];
                    }
                    // authoring_version, then spec_version.
                    let offset = name_at + len + (before - p.len()) + 4;
                    let spec = u32::from_le_bytes(wasm[offset..offset + 4].try_into().unwrap());
                    assert_eq!(spec, 3004000);
                    wasm[offset..offset + 4].copy_from_slice(&(spec + 1).to_le_bytes());
                    return wasm;
                }
            }
            at = end;
        }
        panic!("no runtime_version section");
    }

    #[test]
    fn same_source_traced_override_passes_root_check_with_matching_version_and_substitute_metadata()
    {
        let wasm = traced_wasm();
        let session = ReplaySession::new(None)
            .unwrap()
            .with_runtime_override(Some(wasm.clone()));
        let result = replay_with_session(
            Artifact::load(&fixture("people-transfer")).unwrap(),
            None,
            &session,
        )
        .unwrap();
        assert!(result.root_check);
        assert!(result.capture.spans.iter().any(|s| s.wasm));
        let info = result.runtime_override.unwrap();
        assert!(info.version_differences.is_empty(), "{info:?}");
        let substitute = H256::from(sp_io::hashing::blake2_256(&wasm));
        assert_eq!(info.override_code_hash, substitute);
        assert_ne!(info.chain_code_hash, substitute);
        // Metadata was generated by executing the substitute, never the chain's code.
        let cached = session.metadata.lock();
        assert!(cached.contains_key(&substitute), "{:?}", cached.keys());
        assert!(!cached.contains_key(&info.chain_code_hash));
    }

    #[test]
    fn changed_spec_version_is_reported_before_execution() {
        let session = ReplaySession::new(None)
            .unwrap()
            .with_runtime_override(Some(bump_spec_version(&traced_wasm())));
        let result = replay_with_session(
            Artifact::load(&fixture("people-transfer")).unwrap(),
            None,
            &session,
        )
        .unwrap();
        // Storage is unchanged, so the root check passes; the version difference is still flagged.
        assert!(result.root_check);
        assert_eq!(
            result.runtime_override.unwrap().version_differences,
            vec!["spec_version 3004000 -> 3004001".to_owned()]
        );
    }

    #[test]
    fn unreadable_versions_are_stated_and_failures_blame_the_override() {
        let notes = version_differences(Err("boom".into()), Err("bang".into()));
        assert_eq!(
            notes,
            vec![
                "could not read the chain runtime version: boom".to_owned(),
                "could not read the override runtime version: bang".to_owned()
            ]
        );
        let plain = ReplaySession::new(None).unwrap();
        let session = plain.clone().with_runtime_override(Some(vec![0]));
        let text =
            override_hint(&session, Error::Execution("state root mismatch".into())).to_string();
        assert!(
            text.contains("the runtime override may be the cause"),
            "{text}"
        );
        assert!(
            !override_hint(&plain, Error::Execution("x".into()))
                .to_string()
                .contains("override")
        );
    }

    #[test]
    fn a_different_runtime_fails_the_root_check_and_names_the_override() {
        let westend = Artifact::load(&fixture("westend-asset-hub"))
            .unwrap()
            .proof_nodes
            .iter()
            .map(|n| unhex(n).unwrap())
            .max_by_key(Vec::len)
            .unwrap();
        let session = ReplaySession::new(None)
            .unwrap()
            .with_runtime_override(Some(westend));
        let error = replay_with_session(
            Artifact::load(&fixture("people-transfer")).unwrap(),
            None,
            &session,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("the runtime override may be the cause"),
            "{error}"
        );
    }

    #[test]
    fn override_files_must_be_runtime_wasm() {
        let wasm = traced_wasm();
        assert!(check_runtime_wasm(&wasm).is_ok());
        let plain = sp_maybe_compressed_blob::decompress(&wasm, 50 << 20).unwrap();
        assert!(check_runtime_wasm(&plain).is_ok());
        for (bad, expected) in [
            (b"hello, not wasm".to_vec(), "not a Wasm module"),
            (wasm[..wasm.len() / 2].to_vec(), "decompress"),
            (plain[..plain.len() / 2].to_vec(), "invalid Wasm module"),
        ] {
            let error = check_runtime_wasm(&bad).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
}

#[test]
fn parent_context_overlaps_requests_and_forks_isolate_counters() {
    use crate::rpc::CancellationToken;
    use std::time::Duration;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let rpc = Rpc::with_options(
        &format!("ws://{}", listener.local_addr().unwrap()),
        2,
        Duration::from_secs(10),
        CancellationToken::new(),
    )
    .unwrap();
    let fork = rpc.fork();
    assert_eq!(fork.parallelism(), 2);
    let parent = sp_core::H256::repeat_byte(7);
    let server = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut pending = Vec::new();
        while pending.len() < 2 {
            let (stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "context calls were serialized"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let request: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(request["params"], json!([parent]));
            pending.push((socket, request));
        }
        // Neither request receives a response until both are in flight.
        for (mut socket, request) in pending {
            socket
                .send(tungstenite::Message::Text(
                    json!({
                        "jsonrpc":"2.0", "id":request["id"], "result":request["method"]
                    })
                    .to_string()
                    .into(),
                ))
                .unwrap();
        }
    });
    let (header, version) = parent_context(&fork, parent).unwrap();
    assert_eq!(header, "chain_getHeader");
    assert_eq!(version, "state_getRuntimeVersion");
    assert_eq!(fork.count(), 2);
    assert_eq!(rpc.count(), 0);
    server.join().unwrap();
}
