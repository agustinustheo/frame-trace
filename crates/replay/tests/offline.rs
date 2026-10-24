//! Real signed blocks and the public parachain execute without a node or any network access.
#![allow(clippy::unwrap_used)]
use frame_trace_model::{OpKind, PhaseKind, ReplayEvent, TracedBlock, rpc::parse_trace_block};
use frame_trace_replay::{Artifact, Error, replay};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/replay")
        .join(name)
}
fn read(name: &str, file: &str) -> Vec<u8> {
    zstd::stream::decode_all(std::fs::File::open(fixture(name).join(file)).unwrap()).unwrap()
}
fn keyed_events(events: &[ReplayEvent]) -> Vec<BTreeMap<String, String>> {
    events
        .iter()
        .filter(|e| e.target == "state" && e.fields.contains_key("key"))
        .map(|e| {
            let mut fields = e.fields.clone();
            fields.remove("ext_id");
            fields.remove("origin");
            fields.remove("proof_bytes_added");
            fields
        })
        .collect()
}
fn keyed_phases(block: &TracedBlock) -> Vec<(PhaseKind, Vec<(OpKind, String)>)> {
    block
        .phases
        .iter()
        .flat_map(|p| std::iter::once(p).chain(p.post_inherents.as_deref()))
        .map(|p| {
            (
                p.kind,
                block
                    .phase_ops(p)
                    .filter(|o| {
                        matches!(
                            o.kind,
                            OpKind::Get
                                | OpKind::Hash
                                | OpKind::Exists
                                | OpKind::Put
                                | OpKind::Clear
                                | OpKind::Append
                        )
                    })
                    .map(|o| (o.kind, o.key.to_hex()))
                    .collect(),
            )
        })
        .collect()
}
fn check(name: &str, wasm: bool) {
    let result = replay(Artifact::load(&fixture(name)).unwrap(), None).unwrap();
    if name.starts_with("people-") {
        assert!(!result.artifact.metadata.is_empty());
    }
    assert!(result.root_check);
    assert_eq!((result.rpc_count, result.measured_rpc_count), (0, 0));
    assert!(result.proof_bytes > 0);
    assert_eq!(result.capture.spans.iter().any(|s| s.wasm), wasm);
    assert!(!result.capture.transitions.is_empty());
    assert!(
        result
            .capture
            .events
            .windows(2)
            .all(|w| w[0].at_ns <= w[1].at_ns)
    );
    let block = TracedBlock::from_replay(&result.capture).unwrap();
    assert!(block.ops.iter().all(|op| op.timing.is_some()));
    assert!(
        block
            .ops
            .iter()
            .any(|op| op.timing.unwrap().duration_ns > 0)
    );
    for method in ["ClearPrefix", "StorageRoot"] {
        assert!(
            result.capture.events.iter().any(|e| e.target == "state"
                && e.fields.get("method").is_some_and(|m| m == method)
                && !e.fields.contains_key("key")),
            "missing keyless {method}"
        );
    }
    assert!(block.ops.iter().any(|op| op.kind == OpKind::Iteration));
    for span in &result.capture.spans {
        assert!(span.end_ns.is_some_and(|end| end >= span.start_ns));
        if let Some(parent) = span.parent_id {
            let parent = result
                .capture
                .spans
                .iter()
                .find(|s| s.id == parent)
                .unwrap();
            assert!(parent.start_ns <= span.start_ns);
            assert!(parent.end_ns >= span.end_ns);
        }
    }
    if fixture(name).join("oracle.json.zst").exists() {
        let oracle =
            parse_trace_block(std::str::from_utf8(&read(name, "oracle.json.zst")).unwrap())
                .unwrap();
        assert_eq!(
            oracle.block_hash.trim_start_matches("0x"),
            result.capture.block_hash.trim_start_matches("0x")
        );
        let expected: Vec<BTreeMap<String, String>> = oracle
            .events
            .iter()
            .filter(|e| e.target == "state" && e.field("key").is_some())
            .map(|e| {
                e.data
                    .string_values
                    .iter()
                    .filter(|(k, _)| k.as_str() != "ext_id")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .collect();
        assert_eq!(
            keyed_events(&result.capture.events),
            expected,
            "entire ordered keyed event list, including values"
        );
        assert_eq!(
            keyed_phases(&block),
            keyed_phases(&TracedBlock::from_trace(&oracle).unwrap()),
            "identical phase ownership/boundaries for all oracle ops"
        );
    } else {
        // Public safe endpoints do not expose the unsafe oracle. Compare E2's independently
        // saved execution, with a separately repeated real root check here.
        let events: Vec<ReplayEvent> =
            serde_json::from_slice(&read(name, "reference-events.json.zst")).unwrap();
        assert_eq!(keyed_events(&result.capture.events), keyed_events(&events));
    }
    let metadata = frame_trace_decode::Metadata::from_hex(&result.artifact.metadata).unwrap();
    let annotated = frame_trace_decode::annotate(&block, Some(&metadata));
    assert!(annotated.phases.iter().all(|p| p.call_error.is_none()));
    if name == "people-transfer" {
        assert!(
            annotated
                .phases
                .iter()
                .any(|p| p.call.as_deref() == Some("Balances.transfer_keep_alive"))
        );
    }
    if name == "people-kill-prefix" {
        assert_eq!(
            block
                .ops
                .iter()
                .filter(|o| o.kind == OpKind::ClearPrefix)
                .count(),
            2
        );
    }
}

#[test]
fn signed_people_transfer() {
    check("people-transfer", false);
}
#[test]
fn prefix_deletion_people_block() {
    check("people-kill-prefix", false);
}
#[test]
fn traced_runtime_tree_and_batch() {
    check("people-traced", true);
}
#[test]
fn public_westend_asset_hub() {
    check("westend-asset-hub", false);
}
#[test]
fn tampered_expected_root_traps_in_core() {
    let mut artifact = Artifact::load(&fixture("people-transfer")).unwrap();
    artifact.block["header"]["stateRoot"] = Value::String(format!("0x{}", "42".repeat(32)));
    // Recompute the header hash so validation reaches the runtime's actual root assertion.
    let header: sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256> =
        serde_json::from_value(artifact.block["header"].clone()).unwrap();
    artifact.block_hash = header.hash();
    assert!(matches!(replay(artifact, None), Err(Error::Execution(_))));
}
#[test]
fn tampered_parent_and_missing_proof_fail() {
    let mut artifact = Artifact::load(&fixture("people-transfer")).unwrap();
    artifact.parent_header["stateRoot"] = Value::String(format!("0x{}", "42".repeat(32)));
    assert!(
        replay(artifact, None)
            .unwrap_err()
            .to_string()
            .contains("parent header hash mismatch")
    );
    let mut artifact = Artifact::load(&fixture("people-transfer")).unwrap();
    artifact.proof_nodes.clear();
    assert!(
        replay(artifact, None)
            .unwrap_err()
            .to_string()
            .contains("missing/invalid at path")
    );
}
#[test]
fn artifact_round_trip_and_version_rejection() {
    let dir = std::env::temp_dir().join(format!("frame-trace-artifact-{}", std::process::id()));
    let mut artifact = Artifact::load(&fixture("people-transfer")).unwrap();
    // Auxiliary metadata must not prevent a verified replay from being saved.
    artifact.metadata = "malformed auxiliary metadata".into();
    artifact.save(&dir).unwrap();
    assert_eq!(Artifact::load(&dir).unwrap().metadata, artifact.metadata);
    assert_eq!(
        Artifact::load(&dir).unwrap().proof_nodes,
        artifact.proof_nodes
    );
    std::fs::remove_dir_all(dir).unwrap();
    artifact.format_version = 2;
    assert!(
        replay(artifact, None)
            .unwrap_err()
            .to_string()
            .contains("unsupported artifact version")
    );
}
#[test]
#[ignore = "requires public network and recent parent state"]
fn public_network() {
    let rpc = frame_trace_replay::rpc::Rpc::new("wss://westend-asset-hub-rpc.polkadot.io").unwrap();
    let result = frame_trace_replay::online(&rpc, "best").unwrap();
    assert!(result.root_check);
    assert_eq!(result.measured_rpc_count, 0);
}

#[test]
fn tampered_runtime_proof_node_is_rejected() {
    let mut artifact = Artifact::load(&fixture("people-transfer")).unwrap();
    let runtime_node = artifact
        .proof_nodes
        .iter_mut()
        .max_by_key(|s| s.len())
        .unwrap();
    runtime_node.replace_range(2..4, "42");
    assert!(
        replay(artifact, None)
            .unwrap_err()
            .to_string()
            .contains("missing/invalid at path")
    );
}

#[test]
fn missing_execution_node_is_an_error_with_sdk_panic_handler_in_subprocess() {
    const CHILD: &str = "FRAME_TRACE_PROOF_PANIC_CHILD";
    if std::env::var_os(CHILD).is_some() {
        sp_panic_handler::set("regression test", "test");
        let original = Artifact::load(&fixture("people-transfer")).unwrap();
        // Preserve the code-loading proof, and remove a node demanded by execution itself.
        use hash_db::{EMPTY_PREFIX, HashDB};
        use sp_core::Blake2Hasher;
        let mut memory = sp_trie::MemoryDB::<Blake2Hasher>::default();
        for node in &original.proof_nodes {
            HashDB::insert(
                &mut memory,
                EMPTY_PREFIX,
                &frame_trace_replay::rpc::unhex(node).unwrap(),
            );
        }
        let parent: sp_runtime::generic::Header<u32, sp_runtime::traits::BlakeTwo256> =
            serde_json::from_value(original.parent_header.clone()).unwrap();
        let trie = sp_state_machine::TrieBackendBuilder::new(memory, parent.state_root).build();
        let code = sp_state_machine::prove_read_on_trie_backend(
            &trie,
            [b":code".as_slice(), b":heappages".as_slice()],
        )
        .unwrap();
        let protected: std::collections::BTreeSet<_> = code
            .into_iter_nodes()
            .map(|n| frame_trace_replay::rpc::hex0(&n))
            .collect();
        let index = original
            .proof_nodes
            .iter()
            .position(|n| !protected.contains(n) && n.len() > 130)
            .unwrap();
        for corrupt in [false, true] {
            let mut artifact = original.clone();
            let missing = artifact.proof_nodes[index].clone();
            if corrupt {
                artifact.proof_nodes[index].replace_range(2..4, "42");
            } else {
                artifact.proof_nodes.remove(index);
            }
            let error = replay(artifact, None).unwrap_err().to_string();
            assert!(
                error.contains("proof node") && error.contains("missing/invalid at path"),
                "{error}"
            );
            assert!(!error.contains("offline proof cache miss"));
            let hash =
                sp_crypto_hashing::blake2_256(&frame_trace_replay::rpc::unhex(&missing).unwrap());
            assert!(error.contains(&hex::encode(hash)), "{error}");
        }
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "missing_execution_node_is_an_error_with_sdk_panic_handler_in_subprocess",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "subprocess {:?}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("Externalities not allowed"),
        "{stderr}"
    );
}

#[test]
fn corrupt_artifact_files_and_limits_name_the_path() {
    let dir = std::env::temp_dir().join(format!("frame-trace-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("artifact.json.zst");
    for bytes in [
        b"not zstd".to_vec(),
        zstd::stream::encode_all(b"invalid json".as_slice(), 1).unwrap(),
        zstd::stream::encode_all(vec![b' '; 64 * 1024 * 1024 + 1].as_slice(), 1).unwrap(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(
            Artifact::load(&dir)
                .unwrap_err()
                .to_string()
                .contains(path.to_str().unwrap())
        );
    }
    let original = Artifact::load(&fixture("people-transfer")).unwrap();
    for (nodes, expected) in [
        (vec!["0x00".to_owned(); 100_001], "proof count"),
        (
            vec![format!("0x{}", "00".repeat(24 * 1024 * 1024 + 1))],
            "total proof bytes",
        ),
    ] {
        let mut artifact = original.clone();
        artifact.proof_nodes = nodes;
        let raw = serde_json::to_vec(&artifact).unwrap();
        std::fs::write(&path, zstd::stream::encode_all(raw.as_slice(), 1).unwrap()).unwrap();
        let error = Artifact::load(&dir).unwrap_err().to_string();
        assert!(
            error.contains(expected) && error.contains(path.to_str().unwrap()),
            "{error}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_artifact_saves_are_atomic_and_leave_no_temporary_files() {
    let dir = std::env::temp_dir().join(format!(
        "frame-trace-concurrent-save-{}",
        std::process::id()
    ));
    let original = Artifact::load(&fixture("people-transfer")).unwrap();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for i in 0..8 {
            let mut artifact = original.clone();
            artifact.metadata = format!("writer-{i}");
            let dir = &dir;
            handles.push(scope.spawn(move || artifact.save(dir).unwrap()));
        }
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let artifact = Artifact::load(&dir).unwrap();
    assert!(artifact.metadata.starts_with("writer-"));
    assert_eq!(artifact.proof_nodes, original.proof_nodes);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
