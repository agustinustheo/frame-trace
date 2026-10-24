//! Decoding against real Westend metadata (`fixtures/metadata/`).

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use frame_trace_decode::{KeyName, Metadata, annotate};
use frame_trace_model::{TracedBlock, rpc::parse_trace_block};
use sp_crypto_hashing::{blake2_128, twox_64, twox_128};

const ALICE: &str = "d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d";

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture exists")
}

fn westend(version: u32) -> Metadata {
    Metadata::from_bytes(&fixture(&format!("metadata/westend-v{version}.scale")))
        .expect("valid metadata")
}

fn prefix(pallet: &str, item: &str) -> Vec<u8> {
    [twox_128(pallet.as_bytes()), twox_128(item.as_bytes())].concat()
}

#[test]
fn loads_v14_and_v16() {
    for version in [14, 16] {
        let md = westend(version);
        assert_eq!(md.version(), version);
        assert_eq!(md.ss58_prefix(), 42);
        assert!(md.storage_item_count() > 100);
    }
}

#[test]
fn names_plain_and_map_items() {
    for md in [westend(14), westend(16)] {
        let number = md.key_name(None, &prefix("System", "Number"));
        assert_eq!(number.to_string(), "System::Number");
        assert!(matches!(number, KeyName::Storage { complete: true, .. }));

        // Blake2_128Concat<AccountId32>: decoded and shown as SS58.
        let alice = hex::decode(ALICE).unwrap();
        let key = [
            prefix("System", "Account"),
            blake2_128(&alice).to_vec(),
            alice,
        ]
        .concat();
        let name = md.key_name(None, &key);
        assert_eq!(
            name.to_string(),
            "System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)"
        );
        assert_eq!(name.pallet(), Some("System"));
        assert_eq!(name.item(), "System::Account");

        // Twox64Concat<u32>.
        let n = 100u32.to_le_bytes();
        let key = [
            prefix("System", "BlockHash"),
            twox_64(&n).to_vec(),
            n.to_vec(),
        ]
        .concat();
        assert_eq!(
            md.key_name(None, &key).to_string(),
            "System::BlockHash(100)"
        );
    }
}

#[test]
fn falls_back_gracefully() {
    let md = westend(16);

    // A map prefix (as in `clear_prefix` or iteration): named, no keys, not complete.
    let name = md.key_name(None, &prefix("System", "Account"));
    assert_eq!(name.to_string(), "System::Account");
    assert!(matches!(
        name,
        KeyName::Storage {
            complete: false,
            ..
        }
    ));

    // Known pallet, unknown item.
    let name = md.key_name(None, &prefix("System", "NoSuchItem"));
    assert_eq!(
        name,
        KeyName::Unknown {
            pallet: Some("System".into())
        }
    );

    // Garbage, short keys, and map keys that do not decode.
    assert_eq!(
        md.key_name(None, &[0xff; 40]),
        KeyName::Unknown { pallet: None }
    );
    assert_eq!(
        md.key_name(None, &[0x01]),
        KeyName::Unknown { pallet: None }
    );
    let bad = [prefix("System", "Account"), vec![0; 3]].concat();
    assert!(matches!(
        md.key_name(None, &bad),
        KeyName::Storage {
            complete: false,
            ..
        }
    ));
}

#[test]
fn names_well_known_and_child_keys() {
    let md = westend(14);
    assert_eq!(md.key_name(None, b":code").to_string(), ":code");
    assert_eq!(
        md.key_name(None, b":extrinsic_index").to_string(),
        ":extrinsic_index"
    );
    assert_eq!(md.key_name(None, b":heappages").to_string(), ":heappages");
    assert_eq!(
        md.key_name(None, b":child_storage:default:\x01\x02")
            .to_string(),
        ":child_storage:default:0x0102"
    );
    assert_eq!(
        md.key_name(Some(&[0xbe, 0xef]), b"anything").to_string(),
        "child:0xbeef"
    );
}

#[test]
fn names_extrinsics_from_a_westend_block() {
    let block: serde_json::Value =
        serde_json::from_slice(&fixture("metadata/westend-block-33443133-extrinsics.json"))
            .unwrap();
    let extrinsics: Vec<Vec<u8>> = block["extrinsics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| hex::decode(x.as_str().unwrap().trim_start_matches("0x")).unwrap())
        .collect();

    for md in [westend(14), westend(16)] {
        let names: Vec<String> = extrinsics
            .iter()
            .map(|x| md.call_name(x).unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "Timestamp.set",
                "ParaInherent.enter",
                "Balances.transfer_keep_alive"
            ]
        );
        assert!(md.call_name(&[0x04, 0xff]).is_err());
    }
}

#[test]
fn annotates_a_traced_block() {
    let json = String::from_utf8(fixture("model/two-extrinsics.json")).unwrap();
    let block = TracedBlock::from_trace(&parse_trace_block(&json).unwrap()).unwrap();
    let md = westend(16);
    let annotated = annotate(&block, Some(&md));

    let labels: Vec<&str> = annotated.phases.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "initialization",
            "extrinsic 0: Timestamp.set",
            "extrinsic 1",
            "finalization"
        ]
    );

    let ext1 = &annotated.phases[2];
    let read = &ext1.ops[1];
    assert_eq!(
        read.label,
        "get System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)"
    );
    assert_eq!(
        (read.pallet.as_deref(), read.item.as_str()),
        (Some("System"), "System::Account")
    );
    assert!(ext1.ops[2].duplicate_read);
    assert_eq!(annotated.phases[0].ops[0].label, "get System::Number");
    assert_eq!(
        annotated.phases[3].ops.last().unwrap().label,
        "storage_root"
    );

    // Without metadata only well-known keys get names.
    let bare = annotate(&block, None);
    assert_eq!(bare.phases[1].ops[0].label, "get :extrinsic_index");
    assert_eq!(bare.phases[1].label, "extrinsic 0");
}
