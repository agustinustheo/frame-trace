//! Small calls at the existing People #1 state, without any network connection.
#![allow(clippy::unwrap_used)]
use frame_trace_decode::Metadata;
use frame_trace_model::{OpKind, TracedBlock};
use frame_trace_replay::{Artifact, ReplaySession, call, rpc::unhex};
use std::path::PathBuf;
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/replay")
        .join(name)
}
fn replay(name: &str) -> call::CallReplay {
    call::replay(
        Artifact::load(&fixture(name)).unwrap(),
        None,
        &ReplaySession::new(None)
            .unwrap()
            .with_rpc_metadata_fallback(name.starts_with("call-people")),
        false,
    )
    .unwrap()
}
#[test]
fn offline_nonce_call_authenticates_and_times_account_read() {
    let replay = replay("call-people-nonce");
    assert_eq!(replay.block_number, 1);
    assert_eq!(replay.result, [0; 4]);
    assert_eq!(
        (
            replay.rpc_count,
            replay.measured_rpc_count,
            replay.proof_round_trips
        ),
        (0, 0, 0)
    );
    assert_eq!(replay.oracle_match, None);
    let trace = TracedBlock::from_call(&replay.capture).unwrap();
    assert_eq!(trace.ops.len(), 1);
    assert_eq!(trace.ops[0].kind, OpKind::Get);
    assert!(trace.ops[0].timing.unwrap().duration_ns > 0);
    assert_eq!(trace.phases.len(), 1);
    let md = Metadata::from_hex(&replay.artifact.metadata).unwrap();
    let key = trace.ops[0].key.complete().unwrap();
    assert!(
        md.key_name(None, key)
            .to_string()
            .starts_with("System::Account(")
    );
    assert!(
        md.decode_runtime_result("AccountNonceApi_account_nonce", &replay.result)
            .is_err()
    );
    assert_eq!(replay.proof_bytes, 1286);
}
#[test]
fn offline_transaction_payment_call_reads_fee_multiplier() {
    let replay = replay("call-people-fee");
    let trace = TracedBlock::from_call(&replay.capture).unwrap();
    assert_eq!(trace.ops.len(), 1);
    assert_eq!(replay.proof_bytes, 801);
    let md = Metadata::from_hex(&replay.artifact.metadata).unwrap();
    assert!(
        md.key_name(None, trace.ops[0].key.complete().unwrap())
            .to_string()
            .contains("NextFeeMultiplier")
    );
    assert_eq!(
        replay.result,
        unhex("0x42117985ee0701000000e0e200000000000000000000000000").unwrap()
    );
}
#[test]
fn offline_metadata_call_preserves_empty_trace() {
    let replay = replay("call-people-metadata");
    assert_eq!(
        replay.result,
        unhex("0x0c0e0000000f00000010000000").unwrap()
    );
    assert_eq!(replay.proof_bytes, 0);
    let trace = TracedBlock::from_call(&replay.capture).unwrap();
    assert!(trace.ops.is_empty());
    assert_eq!(trace.phases.len(), 1);
    assert!(trace.timing.unwrap().duration_ns > 0);
}
#[test]
fn offline_calls_reject_wrong_result_state_and_missing_proofs() {
    let original = Artifact::load(&fixture("call-people-nonce")).unwrap();
    let session = ReplaySession::new(None).unwrap();
    let mut artifact = original.clone();
    artifact.call.as_mut().unwrap().result = Some("0x01000000".into());
    assert!(
        call::replay(artifact, None, &session, false)
            .unwrap_err()
            .to_string()
            .contains("recorded call result")
    );
    let mut artifact = original.clone();
    artifact.parent_header["stateRoot"] = serde_json::json!(format!("0x{}", "42".repeat(32)));
    assert!(call::replay(artifact, None, &session, false).is_err());
    let mut artifact = original.clone();
    artifact.proof_nodes.clear();
    assert!(call::replay(artifact, None, &session, false).is_err());
    let mut artifact = original.clone();
    artifact.call.as_mut().unwrap().data = "0xff".into();
    assert!(call::replay(artifact, None, &session, false).is_err());
    assert!(frame_trace_replay::replay(original.clone(), None).is_err());
    assert!(call::replay(original, None, &session, true).is_err());
}

#[test]
fn offline_public_nonce_decodes_v16_and_matches_recorded_node_oracle() {
    let replay = replay("call-westend-nonce");
    assert_eq!(replay.block_number, 18_561_759);
    assert_eq!(replay.result, 520u32.to_le_bytes());
    assert_eq!(replay.oracle_match, Some(true));
    assert_eq!((replay.rpc_count, replay.measured_rpc_count), (0, 0));
    let md = Metadata::from_hex(&replay.artifact.metadata).unwrap();
    assert_eq!(
        md.decode_runtime_result("AccountNonceApi_account_nonce", &replay.result)
            .unwrap(),
        "520"
    );
    let trace = TracedBlock::from_call(&replay.capture).unwrap();
    assert_eq!(trace.ops.len(), 1);
    assert!(
        md.key_name(None, trace.ops[0].key.complete().unwrap())
            .to_string()
            .starts_with("System::Account(")
    );
    assert_eq!(replay.proof_bytes, 3713);
}

#[test]
fn offline_dry_run_set_empty_code_uses_version_reader_and_matches_recorded_oracle() {
    let replay = replay("call-westend-dryrun");
    assert_eq!(replay.block_number, 18_563_012);
    assert_eq!(replay.oracle_match, Some(true));
    assert_eq!(replay.proof_bytes, 3047);
    assert_eq!(replay.measured_rpc_count, 0);
    assert!(!replay.artifact.metadata.is_empty());
    let trace = TracedBlock::from_call(&replay.capture).unwrap();
    assert_eq!(trace.ops.len(), 24);
}

#[test]
fn stored_metadata_is_ignored_and_unused_nodes_are_not_exported() {
    let mut artifact = Artifact::load(&fixture("call-westend-nonce")).unwrap();
    let original = artifact.clone();
    artifact.metadata = "lying metadata".into();
    let junk = "0xdeadbeefcafebabe";
    artifact.proof_nodes.push(junk.into());
    let replay = call::replay(artifact, None, &ReplaySession::new(None).unwrap(), false).unwrap();
    assert_eq!(replay.result, 520u32.to_le_bytes());
    assert_eq!(replay.artifact.metadata, original.metadata);
    assert!(!replay.artifact.proof_nodes.iter().any(|node| node == junk));
    assert!(
        call::replay(
            replay.artifact,
            None,
            &ReplaySession::new(None).unwrap(),
            false
        )
        .is_ok()
    );
}
