use codec::Encode;
use frame_trace_model::TracedBlock;
use frame_trace_replay::{
    Artifact, ArtifactKind, ReplaySession, call,
    rpc::{hex0, unhex},
};
use hash_db::{EMPTY_PREFIX, HashDB};
use serde_json::json;
use sp_core::Blake2Hasher;
use sp_runtime::{generic::Header, traits::BlakeTwo256};
use std::{collections::BTreeSet, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/replay");
    let original = Artifact::load(&base.join("people-transfer"))?;
    let parent: Header<u32, BlakeTwo256> = serde_json::from_value(original.parent_header.clone())?;
    let mut memory = sp_trie::MemoryDB::<Blake2Hasher>::default();
    for node in &original.proof_nodes {
        HashDB::insert(&mut memory, EMPTY_PREFIX, &unhex(node)?);
    }
    let backend = sp_state_machine::TrieBackendBuilder::new(memory, parent.state_root).build();
    // These legacy block fixtures predate metadata-only read proofs. Explicitly keep
    // their recorded V14 decoding metadata while regenerating the original call fixtures.
    let session = ReplaySession::new(None)?.with_rpc_metadata_fallback(true);
    let alice = "0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d";
    let ext = original.block["extrinsics"]
        .as_array()
        .ok_or("extrinsics")?
        .last()
        .and_then(|s| s.as_str())
        .ok_or("signed extrinsic")?;
    let mut info = unhex(ext)?;
    info.extend((info.len() as u32).encode());
    for (name, method, data) in [
        (
            "call-people-nonce",
            "AccountNonceApi_account_nonce",
            alice.to_owned(),
        ),
        (
            "call-people-fee",
            "TransactionPaymentApi_query_info",
            hex0(&info),
        ),
        (
            "call-people-metadata",
            "Metadata_metadata_versions",
            "0x".to_owned(),
        ),
    ] {
        let mut artifact = original.clone();
        artifact.kind = ArtifactKind::Call;
        artifact.block_hash = parent.hash();
        artifact.block = json!({"header": original.parent_header, "extrinsics": []});
        artifact.call = Some(call::Call {
            method: method.into(),
            data,
            result: None,
            oracle: None,
        });
        let replay = call::replay(artifact, None, &session, false)?;
        let trace = TracedBlock::from_call(&replay.capture)?;
        let mut keys: BTreeSet<Vec<u8>> = trace
            .ops
            .iter()
            .filter_map(|op| op.key.complete().map(Vec::from))
            .collect();
        keys.extend([b":code".to_vec(), b":heappages".to_vec()]);
        let proof = sp_state_machine::prove_read_on_trie_backend(&backend, keys)
            .map_err(|e| e.to_string())?;
        let mut artifact = replay.artifact;
        artifact.proof_nodes = proof.into_iter_nodes().map(|n| hex0(&n)).collect();
        let repeated = call::replay(artifact.clone(), None, &session, false)?;
        assert_eq!(repeated.result, replay.result);
        // Fixtures alone share large existing blobs; user --save stays self-contained.
        for value in std::iter::once(&mut artifact.metadata).chain(artifact.proof_nodes.iter_mut())
        {
            let bytes = unhex(value)?;
            let hash = hex::encode(sp_crypto_hashing::blake2_256(&bytes));
            if base.join("blobs").join(format!("{hash}.zst")).exists() {
                *value = format!("blob:{hash}");
            }
        }
        std::fs::create_dir_all(base.join(name))?;
        let raw = serde_json::to_vec(&artifact)?;
        std::fs::write(
            base.join(name).join("artifact.json.zst"),
            zstd::stream::encode_all(raw.as_slice(), 9)?,
        )?;
        println!(
            "{name}: {} ops, {} proof bytes, result {}",
            trace.ops.len(),
            replay.proof_bytes,
            hex0(&replay.result)
        );
    }
    Ok(())
}
