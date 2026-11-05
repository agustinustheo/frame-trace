//! A real contract call that reads and writes a child trie, replayed offline.
//!
//! Paseo Asset Hub #15,366,850 holds one `Revive.eth_transact` to contract
//! 0x0d78e5414aa8c45f40e4c40b0c862ff0486e3262. pallet-revive keeps each contract's storage
//! in its own default child trie, named by the trie id stored in the contract's account info.
#![allow(clippy::unwrap_used)]
use frame_trace_decode::{Metadata, annotate};
use frame_trace_model::{OpKind, TracedBlock};
use frame_trace_replay::{Artifact, replay};
use std::path::PathBuf;

const TRIE_ID: &str = "efdd7b91c3f1506385cbe9ee6e0255a6e4d12d4d2dd1f45975d5160872c45a2d";
const CONTRACT: &str = "0d78e5414aa8c45f40e4c40b0c862ff0486e3262";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/replay")
        .join(name)
}

#[test]
fn contract_call_reads_and_writes_its_child_trie() {
    let artifact = Artifact::load(&fixture("paseo-asset-hub-contract")).unwrap();
    // The fixture stores no metadata: the replay derives it from the runtime, and the names
    // below come from that.
    assert!(artifact.metadata.is_empty());
    let result = replay(artifact, None).unwrap();
    assert_eq!(result.block_number, 15_366_850);
    // Blake2-256 of the stored `:code`, the name of the runtime's fixture blob.
    assert_eq!(
        result.runtime_code_hash,
        "0xd85bfdb17cf421d6a75c9db9ba16cd713a840e42a02ca69becff7e3570b8debb"
            .parse()
            .unwrap()
    );
    assert!(result.root_check);
    assert_eq!((result.rpc_count, result.measured_rpc_count), (0, 0));

    let traced = TracedBlock::from_replay(&result.capture).unwrap();
    let child: Vec<_> = traced
        .ops
        .iter()
        .filter(|o| o.child_trie.is_some())
        .collect();
    let count = |kind| child.iter().filter(|o| o.kind == kind).count();
    assert_eq!((count(OpKind::Get), count(OpKind::Put)), (14, 4));
    assert_eq!(
        child.len(),
        18,
        "only reads and writes, no deletions or iteration"
    );
    let trie_id = hex::decode(TRIE_ID).unwrap();
    assert!(
        child
            .iter()
            .all(|o| o.child_trie.as_ref().unwrap().complete() == Some(&trie_id[..])),
        "every child op is on the called contract's trie"
    );

    let metadata = Metadata::from_hex(&result.artifact.metadata).unwrap();
    let block = annotate(&traced, Some(&metadata));
    assert!(block.all_phases().all(|p| p.call_error.is_none()));
    let call = block
        .phases
        .iter()
        .find(|p| p.call.as_deref() == Some("Revive.eth_transact"))
        .unwrap();
    for phase in block.all_phases() {
        let on_child = phase.ops.iter().filter(|o| o.child_trie.is_some()).count();
        let expected = if std::ptr::eq(phase, call) { 18 } else { 0 };
        assert_eq!(on_child, expected, "{}", phase.label);
    }
    for op in call.ops.iter().filter(|o| o.child_trie.is_some()) {
        assert_eq!(op.child_trie.as_deref(), Some(&*format!("0x{TRIE_ID}")));
        assert_eq!(op.item, format!("child:0x{}", &TRIE_ID[..16]));
    }

    // The child trie is the contract's: its account info, read in the same extrinsic before
    // the first child read, holds this trie id.
    let first_child = call
        .ops
        .iter()
        .position(|o| o.child_trie.is_some())
        .unwrap();
    let info = call.ops[..first_child]
        .iter()
        .find(|o| o.op == "get" && o.item == "Revive::AccountInfoOf" && o.key.ends_with(CONTRACT))
        .expect("the contract's account info is read first");
    let value = result
        .capture
        .events
        .iter()
        .filter(|e| e.target == "state" && e.fields.get("method").is_some_and(|m| m == "Get"))
        .find(|e| {
            e.fields
                .get("key")
                .is_some_and(|k| info.key.ends_with(k.trim_start_matches("0x")))
        })
        .and_then(|e| e.fields.get("result"))
        .unwrap();
    assert!(value.contains(TRIE_ID), "{value}");
}
