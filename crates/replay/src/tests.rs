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
    handler: impl Fn(&Value) -> Value + Send + 'static,
) -> (Rpc, Requests, Arc<AtomicBool>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let worker = thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("accept: {e}"),
            };
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
                reader.read_line(&mut line).unwrap();
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
            let result = handler(&request);
            let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
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
        "state_call",
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
    let parent = artifact.block["header"]["parentHash"].clone();
    let (rpc, requests, halt, worker) =
        server(move |request| match request["method"].as_str().unwrap() {
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
                json!(artifact.metadata)
            }
            "state_getReadProof" => {
                assert_eq!(request["params"][1], parent);
                json!({"at":parent,"proof":artifact.proof_nodes})
            }
            method => panic!("unexpected RPC {method}"),
        });
    let result = online(&rpc, "2").unwrap();
    assert!(result.root_check);
    assert_eq!(result.measured_rpc_count, 0);
    assert_eq!(result.rpc_count, 6);
    assert_eq!(result.artifact.block_hash, expected.block_hash);
    assert_eq!(
        result.artifact.proof_nodes.len(),
        expected.proof_nodes.len()
    );
    assert_eq!(requests.lock().unwrap().len(), 6);
    stop(halt, worker);
}
