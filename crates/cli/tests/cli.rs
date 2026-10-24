//! Runs the binary on the handcrafted fixtures.

#![allow(clippy::unwrap_used)] // Tests may panic on bad fixtures.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Command, Output},
};

static REPLAY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
fn rpc_source_reports_connection_failure() {
    let out = run(&["block", "--rpc", "ws://127.0.0.1:1", "--block", "1"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("connect ws://127.0.0.1:1"));
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

#[test]
fn offline_block_reports_root_and_writes_measured_named_json() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap();
    let path = std::env::temp_dir().join(format!(
        "frame-trace-offline-cli-{}.json",
        std::process::id()
    ));
    let out = run(&[
        "block",
        "--offline",
        &fixture("replay/people-transfer"),
        "--json",
        path.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("root check: PASS; RPC calls: 0 (measured: 0)"),
        "{stderr}"
    );
    let block: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(block["timing"]["duration_ns"].as_u64().unwrap() > 0);
    assert!(
        block["phases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["call"] == "Balances.transfer_keep_alive")
    );
    assert!(block["phases"][0]["ops"][0]["timing"].is_object());
}

struct MockNode {
    url: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl MockNode {
    fn new(
        handler: impl Fn(&serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    ) -> Self {
        use std::{
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            thread,
            time::Duration,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            while !halt.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let handler = handler.clone();
                workers.push(thread::spawn(move || {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap()); let mut len = 0;
                    loop {
                        let mut line = String::new(); if reader.read_line(&mut line).unwrap_or(0) == 0 { return; }
                        if line == "\r\n" { break; }
                        if let Some(v) = line.to_lowercase().strip_prefix("content-length:") { len = v.trim().parse().unwrap(); }
                    }
                    let mut raw = vec![0; len]; if reader.read_exact(&mut raw).is_err() { return; }
                    let request: serde_json::Value = serde_json::from_slice(&raw).unwrap();
                    let mut body = handler(&request); body["jsonrpc"] = "2.0".into(); body["id"] = request["id"].clone();
                    let body = body.to_string();
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for MockNode {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn artifact_rpc(
    artifact: &frame_trace_replay::Artifact,
    request: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::json;
    json!({"result": match request["method"].as_str().unwrap() {
        "chain_getFinalizedHead" | "chain_getBlockHash" => json!(artifact.block_hash),
        "chain_getBlock" => json!({"block":artifact.block}),
        "chain_getHeader" if request["params"].as_array().unwrap().is_empty() => artifact.block["header"].clone(),
        "chain_getHeader" if request["params"][0] == json!(artifact.block_hash) => artifact.block["header"].clone(),
        "chain_getHeader" => artifact.parent_header.clone(),
        "state_getRuntimeVersion" => artifact.runtime_version.clone(),
        // Metadata V13 with its opaque payload encoded as an empty Vec. Execution must survive.
        "state_getMetadata" => json!("0x6d6574610d00"),
        "state_getReadProof" => json!({"at":artifact.block["header"]["parentHash"],"proof":artifact.proof_nodes}),
        method => panic!("unexpected method {method}"),
    }})
}

#[test]
fn cli_save_then_offline_and_summary_survive_unsupported_metadata() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap();
    let artifact = frame_trace_replay::Artifact::load(std::path::Path::new(&fixture(
        "replay/people-transfer",
    )))
    .unwrap();
    let node = MockNode::new(move |request| artifact_rpc(&artifact, request));
    let dir = std::env::temp_dir().join(format!("frame-trace-cli-save-{}", std::process::id()));
    let output = run(&[
        "block",
        "--rpc",
        &node.url,
        "--block",
        "2",
        "--save",
        dir.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("root check: PASS") && stderr.contains("unsupported metadata version V13"),
        "{stderr}"
    );
    drop(node); // The round trip must work after the mock node has stopped.
    let saved = frame_trace_replay::Artifact::load(&dir).unwrap();
    assert!(saved.proof_nodes.iter().all(|node| node.starts_with("0x")));
    for command in ["block", "summary"] {
        let output = run(&[command, "--offline", dir.to_str().unwrap()]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("RPC calls: 0 (measured: 0)"));
        assert!(!output.stdout.is_empty());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn follow_mock_node_retries_failed_blocks_observes_new_heads_and_cancels() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap();
    use std::{
        process::Stdio,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };
    let artifact = frame_trace_replay::Artifact::load(std::path::Path::new(&fixture(
        "replay/people-transfer",
    )))
    .unwrap();
    let stage = Arc::new(AtomicUsize::new(0));
    let advance = stage.clone();
    let failures = Arc::new(AtomicUsize::new(0));
    let attempts = failures.clone();
    let mut new_header = artifact.block["header"].clone();
    new_header["parentHash"] = serde_json::json!(artifact.block_hash);
    new_header["number"] = "0x3".into();
    let new_hash = frame_trace_replay::header_hash(&new_header).unwrap();
    let expected_new = new_hash.clone();
    let node = MockNode::new(move |request| {
        if request["method"] == "chain_getBlock" && request["params"][0] == new_hash {
            return serde_json::json!({"result": null});
        }
        if request["method"] == "state_getMetadata" && attempts.fetch_add(1, Ordering::SeqCst) < 4 {
            return serde_json::json!({"error":{"code":429,"message":"too many requests"}});
        }
        if request["method"] == "chain_getFinalizedHead" && advance.load(Ordering::SeqCst) > 0 {
            return serde_json::json!({"result":new_hash});
        }
        if request["method"] == "chain_getHeader"
            && request["params"][0] == new_hash
            && advance.load(Ordering::SeqCst) > 0
        {
            return serde_json::json!({"result":new_header});
        }
        artifact_rpc(&artifact, request)
    });
    let log = std::env::temp_dir().join(format!(
        "frame-trace-follow-test-{}.log",
        std::process::id()
    ));
    let file = std::fs::File::create(&log).unwrap();
    let mut child = Command::new(BIN)
        .args(["follow", "--rpc", &node.url])
        .stdout(Stdio::null())
        .stderr(file)
        .spawn()
        .unwrap();
    let start = Instant::now();
    let mut passed = false;
    while start.elapsed() < Duration::from_secs(120) {
        let text = std::fs::read_to_string(&log).unwrap();
        if text.contains("traced finalized block #2") {
            stage.store(1, Ordering::SeqCst);
        }
        if text.contains(&format!("block {expected_new} not found"))
            && text.contains("retrying block")
            && text.contains(&format!("terminal block {expected_new}"))
        {
            passed = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let stopped = Instant::now();
    Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    while child.try_wait().unwrap().is_none() && stopped.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let status = child.wait().unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::remove_file(log).unwrap();
    assert!(passed && status.success(), "{status:?}: {text}");
    assert!(stopped.elapsed() < Duration::from_secs(10));
    assert!(failures.load(Ordering::SeqCst) >= 5);
    assert!(
        text.contains("in-flight=")
            && text.contains("terminal=")
            && text.contains("follow stopped")
            && !text.contains("panicked at")
    );
}

#[test]
fn follow_ctrl_c_interrupts_a_stalled_websocket_handshake() {
    use std::{
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let mut child = Command::new(BIN)
        .args(["follow", "--rpc", &url])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (mut stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(false).unwrap();
    let node = thread::spawn(move || {
        let _ = stream.read_to_end(&mut Vec::new());
    });
    let start = Instant::now();
    Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    while child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    assert!(child.wait().unwrap().success());
    assert!(start.elapsed() < Duration::from_secs(10));
    node.join().unwrap();
}

#[test]
fn help_describes_best_concurrency_cache_and_otlp_default() {
    let output = run(&["block", "--help"]);
    let text = String::from_utf8_lossy(&output.stdout);
    for word in [
        "best",
        "--proof-concurrency",
        "--cache-dir",
        "--cache-max-bytes",
        "http://127.0.0.1:4318",
    ] {
        assert!(text.contains(word), "{text}");
    }
    let output = run(&["follow", "--help"]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("http://127.0.0.1:4318")
            && text.contains("--best")
            && text.contains("finalized")
    );
}
