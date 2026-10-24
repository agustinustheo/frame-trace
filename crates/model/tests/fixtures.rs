//! Segmentation, stats and duplicate detection on the handcrafted fixtures in `fixtures/model/`.

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use frame_trace_model::{OpKind, PhaseKind, Stats, TracedBlock, rpc::parse_trace_block};

fn load(name: &str) -> TracedBlock {
    let path = format!("{}/../../fixtures/model/{name}", env!("CARGO_MANIFEST_DIR"));
    let json = std::fs::read_to_string(&path).expect("fixture exists");
    TracedBlock::from_trace(&parse_trace_block(&json).expect("valid trace")).expect("valid events")
}

#[test]
fn segments_a_block_with_two_extrinsics() {
    let block = load("two-extrinsics.json");
    assert_eq!(
        block.skipped_events, 1,
        "the Call event is not a storage op"
    );
    assert_eq!(block.ops.len(), 30);

    let phases: Vec<_> = block
        .phases
        .iter()
        .map(|p| (p.kind, p.ops.clone()))
        .collect();
    assert_eq!(
        phases,
        vec![
            (PhaseKind::Initialization, 0..7),
            (PhaseKind::ApplyExtrinsic(0), 7..13),
            (PhaseKind::ApplyExtrinsic(1), 13..21),
            // 21..26 is the gap closed by the Finalization marker, folded into Finalization.
            (PhaseKind::Finalization, 21..30),
        ]
    );
    assert_eq!(block.extrinsic_count(), 2);
}

#[test]
fn computes_phase_stats() {
    let block = load("two-extrinsics.json");
    let stats: Vec<Stats> = block.phases.iter().map(|p| p.stats).collect();
    let expected = [
        Stats {
            ops: 7,
            reads: 3,
            writes: 4,
            distinct_keys: 3,
            bytes_read: 12,
            bytes_written: 14,
            duplicate_reads: 2,
        },
        Stats {
            ops: 6,
            reads: 2,
            writes: 4,
            distinct_keys: 4,
            bytes_read: 12,
            bytes_written: 29,
            duplicate_reads: 0,
        },
        Stats {
            ops: 8,
            reads: 4,
            writes: 4,
            distinct_keys: 6,
            bytes_read: 32,
            bytes_written: 11,
            duplicate_reads: 2,
        },
        Stats {
            ops: 9,
            reads: 2,
            writes: 6,
            distinct_keys: 6,
            bytes_read: 8,
            bytes_written: 5,
            duplicate_reads: 0,
        },
    ];
    assert_eq!(stats, expected);

    let totals = block.totals();
    assert_eq!(
        (
            totals.ops,
            totals.reads,
            totals.writes,
            totals.duplicate_reads
        ),
        (30, 11, 18, 4)
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
    assert_eq!(dups, vec![3, 4, 15, 18]);
    // System::Number is read again in finalization, but that is a different phase.
    assert!(!block.ops[26].duplicate_read);
    assert_eq!(block.ops[18].kind, OpKind::Exists);
}

#[test]
fn extracts_extrinsic_bytes_from_extrinsic_data() {
    let block = load("two-extrinsics.json");
    let ext0 = block.phases[1].extrinsic.as_deref().map(hex::encode);
    assert_eq!(ext0.as_deref(), Some("280502000b80974e6fa101"));
    assert_eq!(block.phases[2].extrinsic, None);
}

#[test]
fn block_without_extrinsics_has_no_apply_phase() {
    let block = load("no-extrinsics.json");
    let kinds: Vec<_> = block
        .phases
        .iter()
        .map(|p| (p.kind, p.ops.clone()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (PhaseKind::Initialization, 0..3),
            (PhaseKind::Finalization, 3..6)
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
