//! Segmentation, stats and duplicate detection on the fixtures in `fixtures/model/`.

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use std::ops::Range;

use frame_trace_model::{OpKind, PhaseKind, Stats, TracedBlock, rpc::parse_trace_block};

fn load(name: &str) -> TracedBlock {
    let path = format!("{}/../../fixtures/model/{name}", env!("CARGO_MANIFEST_DIR"));
    let json = std::fs::read_to_string(&path).expect("fixture exists");
    TracedBlock::from_trace(&parse_trace_block(&json).expect("valid trace")).expect("valid events")
}

/// Top-level phases, with the range of a nested `PostInherents` phase.
fn phases(block: &TracedBlock) -> Vec<(PhaseKind, Range<usize>, Option<Range<usize>>)> {
    block
        .phases
        .iter()
        .map(|p| {
            let nested = p.post_inherents.as_ref().map(|n| {
                assert_eq!(n.kind, PhaseKind::PostInherents);
                n.ops.clone()
            });
            (p.kind, p.ops.clone(), nested)
        })
        .collect()
}

#[test]
fn segments_a_block_with_two_extrinsics() {
    let block = load("two-extrinsics.json");
    assert_eq!(
        block.skipped_events, 1,
        "the Call event is not a storage op"
    );
    assert_eq!(block.ops.len(), 38);
    assert_eq!(
        phases(&block),
        vec![
            // 7..9 runs after the ApplyExtrinsic(0) marker but is initialize_block's tail.
            (PhaseKind::Initialization, 0..9, None),
            // Ends with the marker and the ExtrinsicWeightReclaimed cleanup (15).
            (PhaseKind::ApplyExtrinsic(0), 9..16, None),
            // The first signed extrinsic runs inherents_applied (16..19) before its own work.
            (PhaseKind::ApplyExtrinsic(1), 16..29, Some(16..19)),
            (PhaseKind::Finalization, 29..38, None),
        ]
    );
    assert_eq!(block.extrinsic_count(), 2);
}

#[test]
fn computes_phase_stats() {
    let block = load("two-extrinsics.json");
    let stats: Vec<Stats> = block.phases.iter().map(|p| p.stats).collect();
    let s = |ops, reads, writes, distinct_keys, bytes_read, bytes_written, duplicate_reads| Stats {
        ops,
        reads,
        writes,
        distinct_keys,
        bytes_read,
        bytes_written,
        duplicate_reads,
        truncated_keys: 0,
    };
    let expected = [
        s(9, 5, 4, 5, 44, 14, 2),
        s(7, 2, 5, 5, 12, 29, 0),
        // Extrinsic 1 without its nested post-inherents work.
        s(10, 5, 5, 7, 36, 159, 2),
        s(9, 3, 5, 6, 9, 5, 0),
    ];
    assert_eq!(stats, expected);
    let nested = block.phases[2].post_inherents.as_ref().unwrap();
    assert_eq!(nested.stats, s(3, 2, 1, 2, 8, 1, 0));
    assert_eq!(block.phase_ops(&block.phases[2]).count(), 10);

    let totals = block.totals();
    assert_eq!(
        (
            totals.ops,
            totals.reads,
            totals.writes,
            totals.duplicate_reads
        ),
        (38, 17, 20, 4)
    );
}

#[test]
fn flags_duplicate_reads_per_phase() {
    let block = load("two-extrinsics.json");
    let dups: Vec<usize> = block
        .ops
        .iter()
        .filter(|op| op.duplicate_read)
        .map(|op| op.seq)
        .collect();
    // System::Number twice in initialization; System::Account read again, then `Exists` on it.
    assert_eq!(dups, vec![3, 4, 22, 25]);
    // System::Number is read again in finalization, but that is a different phase.
    assert!(!block.ops[34].duplicate_read);
    assert_eq!(block.ops[25].kind, OpKind::Exists);
}

#[test]
fn extracts_extrinsic_bytes_from_extrinsic_data() {
    let block = load("two-extrinsics.json");
    let ext0 = block.phases[1].extrinsic.as_deref().map(hex::encode);
    assert_eq!(ext0.as_deref(), Some("280502000b80974e6fa101"));
    let ext1 = block.phases[2].extrinsic.as_ref().unwrap();
    assert_eq!(ext1.len(), 146);
}

#[test]
fn block_without_extrinsics_runs_inherents_applied_before_finalization() {
    let block = load("no-extrinsics.json");
    assert_eq!(
        phases(&block),
        vec![
            (PhaseKind::Initialization, 0..3, None),
            (PhaseKind::PostInherents, 3..6, None),
            // From the `:extrinsic_index` take in note_finished_extrinsics.
            (PhaseKind::Finalization, 6..10, None),
        ]
    );
}

#[test]
fn trace_without_markers_is_unsegmented() {
    let json = r#"{"blockHash":"0x01","parentHash":"0x00","events":[
        {"target":"state","data":{"stringValues":{"method":"Get","key":"aa","result":"None"}},"parentId":null},
        {"target":"state","data":{"stringValues":{"method":"Get","key":"aa","result":"None"}},"parentId":null}]}"#;
    let block = TracedBlock::from_trace(&parse_trace_block(json).unwrap()).unwrap();
    assert_eq!(block.phases.len(), 1);
    assert_eq!(block.phases[0].kind, PhaseKind::Unsegmented);
    assert_eq!(block.phases[0].stats.duplicate_reads, 1);
}

/// Block 1 of a `next-people-paseo` parachain (two inherents, `on_poll` work after them),
/// recorded from a real node. See `fixtures/model/real/README.md`.
#[test]
fn segments_a_real_parachain_block() {
    let block = load("real/people-paseo-block-1-trace-state.json");
    assert_eq!(block.ops.len(), 315);
    assert_eq!(
        phases(&block),
        vec![
            // 146 (MultiBlockMigrations::Cursor) and 147 (System::BlockHash) follow the
            // ApplyExtrinsic(0) marker but are initialize_block's tail.
            (PhaseKind::Initialization, 0..148, None),
            // set_validation_data, ending with the ExtrinsicWeightReclaimed cleanup at 199.
            (PhaseKind::ApplyExtrinsic(0), 148..200, None),
            // Timestamp.set, cleanup at 225.
            (PhaseKind::ApplyExtrinsic(1), 200..226, None),
            // No signed extrinsic, so inherents_applied (on_poll) runs before finalization.
            (PhaseKind::PostInherents, 226..249, None),
            (PhaseKind::Finalization, 249..315, None),
        ]
    );
    let is = |seq: usize, key: &[u8]| block.ops[seq].key.complete() == Some(key);
    let ewr = frame_trace_model::keys::EXTRINSIC_WEIGHT_RECLAIMED;
    assert!(is(199, &ewr) && is(225, &ewr));
    assert!(is(249, frame_trace_model::keys::EXTRINSIC_INDEX));
    assert!(block.phases[1].extrinsic.is_some() && block.phases[2].extrinsic.is_some());
}

/// Block 2 of the same chain: the two inherents plus a signed `Balances.transfer_keep_alive`.
#[test]
fn signed_extrinsic_owns_its_check_reads() {
    let block = load("real/people-paseo-block-2-trace-state.json");
    assert_eq!(block.ops.len(), 418);
    assert_eq!(
        phases(&block),
        vec![
            (PhaseKind::Initialization, 0..150, None),
            (PhaseKind::ApplyExtrinsic(0), 150..204, None),
            (PhaseKind::ApplyExtrinsic(1), 204..230, None),
            // 230..234 are the transfer's check reads (CheckGenesis, CheckMortality), then
            // inherents_applied (on_poll) runs inside it, then its own work from 248.
            (PhaseKind::ApplyExtrinsic(2), 230..347, Some(234..248)),
            (PhaseKind::Finalization, 347..418, None),
        ]
    );
    let transfer = &block.phases[3];
    let own: Vec<usize> = block.phase_ops(transfer).map(|op| op.seq).take(5).collect();
    assert_eq!(own, [230, 231, 232, 233, 248]);
    let block_hash_prefix = [
        frame_trace_model::keys::SYSTEM_PREFIX.as_slice(),
        &[
            0xa4, 0x47, 0x04, 0xb5, 0x68, 0xd2, 0x16, 0x67, 0x35, 0x6a, 0x5a, 0x05, 0x0c, 0x11,
            0x87, 0x46,
        ],
    ]
    .concat();
    assert!(block.ops[230].key.head.starts_with(&block_hash_prefix));
    assert!(transfer.extrinsic.is_some());
    assert_eq!(
        transfer.stats.ops + transfer.post_inherents.as_ref().unwrap().stats.ops,
        transfer.ops.len()
    );
}
