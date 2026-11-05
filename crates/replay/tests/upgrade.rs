//! Runtime upgrades on Westend Asset Hub: spec 1025003 -> 1025004 at block 18,985,169.
//!
//! Block 18,985,169 executes with the old code and writes the new `:code`. Block 18,985,170
//! is the first block on the new code. Both are replayed by the ignored live test: saved,
//! they would add 2.7 MB (one block) or about 5 MB (both) of runtimes.
#![allow(clippy::unwrap_used)]
use codec::Decode;
use frame_metadata::{RuntimeMetadata, RuntimeMetadataPrefixed};
use frame_trace_decode::{AnnotatedBlock, Metadata, annotate};
use frame_trace_model::TracedBlock;
use frame_trace_replay::{Replay, ReplaySession, rpc::unhex};
use std::collections::BTreeMap;

const ENACTMENT: u64 = 18_985_169;
/// Blake2-256 of the stored `:code` bytes.
const SPEC_1025003: &str = "0x985f0daaaeec1ed429c305034a95dd233bdd292bef383e3d6643bb00f1c6a132";
const SPEC_1025004: &str = "0xc7a91f4aafad7a00424bf9d5c8029f2fac2b6ea87ee7ece41fcd3825a5aa8bfd";

fn code_hash(hex: &str) -> sp_core::H256 {
    hex.parse().unwrap()
}

fn westend_asset_hub_rpc() -> String {
    std::env::var("FRAME_TRACE_WESTEND_AH_RPC")
        .unwrap_or_else(|_| "wss://westend-asset-hub-rpc.polkadot.io".into())
}

/// `spec_version` from the `System::Version` constant of the metadata the replay produced.
fn metadata_spec_version(metadata: &str) -> u32 {
    let RuntimeMetadataPrefixed(_, metadata) =
        RuntimeMetadataPrefixed::decode(&mut &unhex(metadata).unwrap()[..]).unwrap();
    let RuntimeMetadata::V16(metadata) = metadata else {
        panic!("expected V16 metadata");
    };
    let system = metadata
        .pallets
        .iter()
        .find(|p| p.name == "System")
        .unwrap();
    let version = system
        .constants
        .iter()
        .find(|c| c.name == "Version")
        .unwrap();
    let mut bytes = &version.value[..];
    let (_spec_name, _impl_name, _authoring) = <(String, String, u32)>::decode(&mut bytes).unwrap();
    u32::decode(&mut bytes).unwrap()
}

fn annotated(replay: &Replay) -> AnnotatedBlock {
    let block = TracedBlock::from_replay(&replay.capture).unwrap();
    let metadata = Metadata::from_hex(&replay.artifact.metadata).unwrap();
    annotate(&block, Some(&metadata))
}

/// Ordered `(phase, op, name, value length)` for every read and write: the replay's result
/// without timings.
fn ops(block: &AnnotatedBlock) -> Vec<(String, String, String, Option<usize>)> {
    block
        .all_phases()
        .flat_map(|p| {
            p.ops
                .iter()
                .map(|o| (p.label.clone(), o.op.clone(), o.name.clone(), o.value_len))
        })
        .collect()
}

fn phase_labels(block: &AnnotatedBlock) -> Vec<&str> {
    block.all_phases().map(|p| p.label.as_str()).collect()
}

fn keyed_events(replay: &Replay) -> Vec<BTreeMap<String, String>> {
    replay
        .capture
        .events
        .iter()
        .filter(|e| e.target == "state" && e.fields.contains_key("key"))
        .map(|e| {
            let mut fields = e.fields.clone();
            for volatile in ["ext_id", "origin", "proof_bytes_added"] {
                fields.remove(volatile);
            }
            fields
        })
        .collect()
}

/// The first block after the upgrade.
fn check_first_block_on_new_code(replay: &Replay) {
    assert_eq!(replay.block_number, ENACTMENT + 1);
    assert_eq!(replay.runtime_code_hash, code_hash(SPEC_1025004));
    assert!(replay.root_check);
    assert_eq!(replay.measured_rpc_count, 0);
    assert_eq!(metadata_spec_version(&replay.artifact.metadata), 1_025_004);
    let block = annotated(replay);
    assert!(block.all_phases().all(|p| p.call_error.is_none()));
    assert_eq!(
        phase_labels(&block),
        [
            "initialization",
            "extrinsic 0: ParachainSystem.set_validation_data",
            "extrinsic 1: Timestamp.set",
            "post inherents (on_poll)",
            "finalization",
        ]
    );
    let initialization = &block.phases[0];
    assert!(
        initialization
            .ops
            .iter()
            .any(|o| o.op == "put" && o.name == "System::LastRuntimeUpgrade"),
        "on_runtime_upgrade bookkeeping runs in initialization"
    );
    assert!(
        !block
            .all_phases()
            .flat_map(|p| &p.ops)
            .any(|o| o.name == ":code" && o.op == "put")
    );
}

#[test]
#[ignore = "requires an archive Westend Asset Hub endpoint (FRAME_TRACE_WESTEND_AH_RPC)"]
fn runtime_upgrade_enactment_and_next_block_live() {
    let rpc = frame_trace_replay::rpc::Rpc::new(&westend_asset_hub_rpc()).unwrap();
    let session = ReplaySession::new(None).unwrap();
    // In order, through one session, as `report` and `follow` do.
    let enactment = frame_trace_replay::online_with_session(
        &rpc,
        "0xd0e2fe81d72dff6ba86daefdae21801ab0e7155760779c54c371db078d0ce55b",
        &session,
    )
    .unwrap();
    let next = frame_trace_replay::online_with_session(&rpc, "18985170", &session).unwrap();

    assert_eq!(enactment.block_number, ENACTMENT);
    assert!(enactment.root_check);
    assert_eq!(enactment.measured_rpc_count, 0);
    assert_eq!(
        metadata_spec_version(&enactment.artifact.metadata),
        1_025_003
    );
    let block = annotated(&enactment);
    assert!(block.all_phases().all(|p| p.call_error.is_none()));
    assert_eq!(
        phase_labels(&block),
        [
            "initialization",
            "extrinsic 0: ParachainSystem.set_validation_data",
            "extrinsic 1: Timestamp.set",
            "post inherents (on_poll)",
            "finalization",
        ]
    );
    let inherent = &block.phases[1];
    let code = inherent
        .ops
        .iter()
        .find(|o| o.op == "put" && o.name == ":code")
        .expect("the validation data inherent writes the new :code");
    assert!(code.value_len.unwrap() > 1_000_000);
    assert!(
        inherent
            .ops
            .iter()
            .any(|o| o.op == "clear" && o.name == "ParachainSystem::PendingValidationCode")
    );

    check_first_block_on_new_code(&next);
    assert_eq!(enactment.runtime_code_hash, code_hash(SPEC_1025003));

    // Each block alone, in a fresh session, gives the same result.
    for shared in [&enactment, &next] {
        let alone = frame_trace_replay::online_with_session(
            &rpc,
            &shared.block_number.to_string(),
            &ReplaySession::new(None).unwrap(),
        )
        .unwrap();
        assert_eq!(alone.runtime_code_hash, shared.runtime_code_hash);
        assert_eq!(alone.computed_state_root, shared.computed_state_root);
        assert_eq!(alone.artifact.metadata, shared.artifact.metadata);
        assert_eq!(keyed_events(&alone), keyed_events(shared));
        assert_eq!(ops(&annotated(&alone)), ops(&annotated(shared)));
    }
}
