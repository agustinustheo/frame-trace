//! Runs the binary on the handcrafted fixtures.

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Command, Output},
};

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
        "System::Account(5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY)",
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

#[test]
fn summary_reads_hex_metadata_and_a_real_trace() {
    let out = run(&[
        "summary",
        "--from-file",
        &fixture("model/real/people-paseo-block-1-trace-state.json"),
        "--metadata",
        &fixture("model/real/people-paseo-block-1-metadata.hex"),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "extrinsic 0: ParachainSystem.set_validation_data",
        "extrinsic 1: Timestamp.set",
        "post inherents (on_poll)",
        "System::BlockWeight",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
}

#[test]
fn duplicate_reads_are_grouped_by_key_not_by_name() {
    let get = |key: &str| {
        format!(
            r#"{{"target":"state","data":{{"stringValues":{{"method":"Get","key":"{key}","result":"None"}}}},"parentId":null}}"#
        )
    };
    let exists = r#"{"target":"state","data":{"stringValues":{"method":"Exists","key":"aa","result":"true"}},"parentId":null}"#;
    let json = format!(
        r#"{{"blockHash":"0x01","parentHash":"0x00","events":[{},{},{},{},{exists}]}}"#,
        get("aa"),
        get("aa"),
        get("bb"),
        get("bb"),
    );
    let path = std::env::temp_dir().join(format!("frame-trace-dups-{}.json", std::process::id()));
    std::fs::write(&path, json).unwrap();
    let out = run(&["summary", "--from-file", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let rows: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("unsegmented") && l.contains("<unknown>"))
        .collect();
    // aa: re-read by get and exists (2), bb: once (1). One row per key.
    assert_eq!(rows.len(), 2, "{stdout}");
    assert!(rows[0].contains("[0xaa]") && rows[0].trim_end_matches('│').trim_end().ends_with('2'));
    assert!(rows[1].contains("[0xbb]") && rows[1].trim_end_matches('│').trim_end().ends_with('1'));
}

/// A one-shot OTLP/HTTP receiver: answers `400` to the first request and `200` afterwards.
fn rejecting_receiver() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for (i, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).ok();
            let status = if i == 0 { "400 Bad Request" } else { "200 OK" };
            let response =
                format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            stream.write_all(response.as_bytes()).ok();
        }
    });
    url
}

#[test]
fn block_fails_when_the_collector_rejects_spans() {
    let url = rejecting_receiver();
    let out = run(&[
        "block",
        "--from-file",
        &fixture("model/real/people-paseo-block-1-trace-state.json"),
        "--otlp",
        &url,
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    // 1 block + 5 phases + 315 ops, all in the first (rejected) batch.
    assert!(
        stderr.contains("321 of 321 spans were not exported"),
        "{stderr}"
    );
}

#[test]
fn summary_shows_post_inherents_inside_the_first_signed_extrinsic() {
    let out = run(&[
        "summary",
        "--from-file",
        &fixture("model/real/people-paseo-block-2-trace-state.json"),
        "--metadata",
        &fixture("model/real/people-paseo-block-1-metadata.hex"),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let rows: Vec<&str> = stdout.lines().filter(|l| l.contains(" ┆ ")).collect();
    let transfer = rows
        .iter()
        .position(|l| l.contains("extrinsic 2: Balances.transfer_keep_alive"))
        .unwrap();
    assert!(
        rows[transfer + 1].contains("└ post inherents (on_poll)"),
        "{stdout}"
    );
    // 103 own ops (check reads + its work), 14 nested.
    assert!(rows[transfer].contains("┆ 103 ┆"), "{stdout}");
    assert!(rows[transfer + 1].contains("┆  14 ┆"), "{stdout}");
}

#[test]
fn child_trie_keys_are_named_and_grouped_by_key() {
    let get = |key: &str| {
        format!(
            r#"{{"target":"state","data":{{"stringValues":{{"method":"ChildGet","child_info":"01","key":"{key}","result":"None"}}}},"parentId":null}}"#
        )
    };
    let json = format!(
        r#"{{"blockHash":"0x01","parentHash":"0x00","events":[{},{},{},{}]}}"#,
        get("aa"),
        get("aa"),
        get("bb"),
        get("bb"),
    );
    let path = std::env::temp_dir().join(format!("frame-trace-child-{}.json", std::process::id()));
    std::fs::write(&path, json).unwrap();
    let out = run(&["summary", "--from-file", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let rows: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("unsegmented") && l.contains("child:"))
        .collect();
    assert_eq!(rows.len(), 2, "{stdout}");
    assert!(
        rows[0].contains("child:0x01[0xaa]") && !rows[0].contains(" ["),
        "{stdout}"
    );
    assert!(rows[1].contains("child:0x01[0xbb]"), "{stdout}");
}
