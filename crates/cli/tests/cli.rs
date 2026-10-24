//! Runs the binary on the handcrafted fixtures.

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_frame-trace");

fn fixture(path: &str) -> String {
    format!("{}/../../fixtures/{path}", env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().expect("binary runs")
}

#[test]
fn summary_prints_phase_and_item_tables() {
    let out = run(&[
        "summary",
        "--from-file",
        &fixture("model/two-extrinsics.json"),
        "--metadata",
        &fixture("metadata/westend-v14.scale"),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "extrinsic 0: Timestamp.set",
        "System::Account",
        "get System::Number",
        "total",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
}

#[test]
fn block_writes_json() {
    let path =
        std::env::temp_dir().join(format!("frame-trace-cli-test-{}.json", std::process::id()));
    let out = run(&[
        "block",
        "--from-file",
        &fixture("model/two-extrinsics.json"),
        "--metadata",
        &fixture("metadata/westend-v16.scale"),
        "--json",
        path.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(json["phases"][1]["call"], "Timestamp.set");
    assert_eq!(json["totals"]["duplicate_reads"], 4);
}

#[test]
fn rpc_source_is_not_implemented_yet() {
    let out = run(&["block", "--rpc", "ws://127.0.0.1:9944", "--block", "1"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not implemented yet"));
}
