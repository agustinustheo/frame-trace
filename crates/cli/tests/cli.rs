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
    assert_eq!(json["schema_version"], 2);
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
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = std::env::temp_dir().join(format!(
        "frame-trace-offline-cli-{}.json",
        std::process::id()
    ));
    let out = run(&[
        "block",
        "--offline",
        &fixture("replay/people-transfer"),
        "--unauthenticated-metadata",
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
                    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&request)));
                    let mut body = match &response {
                        Ok(body) => body.clone(),
                        Err(payload) => serde_json::json!({"error":{"code":-32099,"message":format!("mock handler assertion failed: {}", payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied()).unwrap_or("panic"))}}),
                    }; body["jsonrpc"] = "2.0".into(); body["id"] = request["id"].clone();
                    let body = body.to_string();
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    if let Err(payload) = response { std::panic::resume_unwind(payload); }
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
        if let Some(worker) = self.worker.take()
            && let Err(payload) = worker.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(payload);
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
fn cli_save_then_offline_ignores_unauthenticated_rpc_metadata() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        stderr.contains("root check: PASS") && !stderr.contains("unsupported metadata version V13"),
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
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        if request["method"] == "state_getRuntimeVersion"
            && attempts.fetch_add(1, Ordering::SeqCst) < 4
        {
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
    let output = run(&["call", "--help"]);
    let text = String::from_utf8_lossy(&output.stdout);
    for word in [
        "stdout",
        "--json",
        "OTLP/HTTP",
        "32 keys",
        "compiled Wasm",
        "cache limit",
        "--unauthenticated-metadata",
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

#[test]
fn call_offline_exports_measured_named_ops_and_self_contained_round_trip() {
    let _guard = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir =
        std::env::temp_dir().join(format!("frame-trace-call-roundtrip-{}", std::process::id()));
    let out = run(&[
        "call",
        "--offline",
        &fixture("replay/call-people-nonce"),
        "--unauthenticated-metadata",
        "--json",
        "-",
        "--save",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["call"]["method"], "AccountNonceApi_account_nonce");
    assert_eq!(json["call"]["result"], "0x00000000");
    assert!(json["call"]["decoded"].is_null()); // recorded metadata is V14
    assert_eq!(json["phases"][0]["kind"], "call");
    assert_eq!(json["phases"][0]["ops"][0]["item"], "System::Account");
    assert!(
        json["phases"][0]["ops"][0]["timing"]["duration_ns"]
            .as_u64()
            .unwrap()
            > 0
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("root check: NOT APPLICABLE"));
    assert!(stderr.contains("RPC calls: 0 (measured: 0)"));
    assert!(
        stderr.contains("V14 metadata has no runtime API types")
            && !stderr.contains("invalid metadata:")
    );
    let repeated = run(&[
        "call",
        "--offline",
        dir.to_str().unwrap(),
        "--summary",
        "--unauthenticated-metadata",
    ]);
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    assert!(
        String::from_utf8_lossy(&repeated.stdout)
            .contains("call AccountNonceApi_account_nonce at #1")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn call_arguments_require_one_input_mode_and_offline_cannot_contact_rpc() {
    for args in [
        vec![
            "call",
            "--rpc",
            "http://127.0.0.1:1",
            "--method",
            "AccountNonceApi_account_nonce",
        ],
        vec![
            "call",
            "--rpc",
            "http://127.0.0.1:1",
            "--method",
            "method",
            "--data",
            "0x",
            "--args",
            "[]",
        ],
        vec!["call", "--offline", "missing", "--oracle"],
        vec![
            "call",
            "--offline",
            "missing",
            "--rpc",
            "http://127.0.0.1:1",
        ],
        vec!["call", "--offline", "missing", "--data", "0x"],
    ] {
        let out = run(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn call_public_offline_decodes_result_and_labels_recorded_oracle() {
    let _guard = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let out = run(&[
        "call",
        "--offline",
        &fixture("replay/call-westend-nonce"),
        "--json",
        "-",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["call"]["decoded"], "520");
    assert_eq!(json["call"]["oracle_match"], true);
    assert_eq!(json["call"]["oracle_source"], "recorded (unverified)");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("state_call oracle: MATCH (recorded; unverified)")
    );
}

fn call_node(mismatch: bool) -> MockNode {
    let artifact = frame_trace_replay::Artifact::load(std::path::Path::new(&fixture(
        "replay/call-westend-nonce",
    )))
    .unwrap();
    MockNode::new(move |request| {
        let result = match request["method"].as_str().unwrap() {
            "chain_getHeader" => artifact.parent_header.clone(),
            "chain_getBlockHash" | "chain_getFinalizedHead" => {
                serde_json::json!(artifact.block_hash)
            }
            "state_getRuntimeVersion" => artifact.runtime_version.clone(),
            "state_getReadProof" => {
                serde_json::json!({"at":artifact.block_hash,"proof":artifact.proof_nodes})
            }
            "state_call" => serde_json::json!(if mismatch { "0xffffffff" } else { "0x08020000" }),
            method => panic!("unexpected method {method}; metadata must execute locally"),
        };
        serde_json::json!({"result":result})
    })
}

#[test]
fn malformed_args_and_bad_data_are_identified_before_connecting() {
    for (flag, value, expected) in [
        ("--args", "{", "--args: malformed JSON"),
        ("--data", "0xz", "--data: invalid SCALE hex"),
    ] {
        let out = run(&[
            "call",
            "--rpc",
            "http://127.0.0.1:1",
            "--method",
            "Unknown_method",
            flag,
            value,
        ]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1));
        assert!(
            stderr.contains(expected) && !stderr.contains("invalid metadata:"),
            "{stderr}"
        );
    }
}

#[test]
fn local_metadata_rejects_unknown_method_and_wrong_argument_types() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let node = call_node(false);
    for (method, args, expected) in [
        (
            "AccountNonceApi_account_nonc",
            "[]",
            "did you mean AccountNonceApi_account_nonce?",
        ),
        (
            "AccountNonceApi_account_nonce",
            "{\"account\":\"wrong type\"}",
            "--args: cannot encode",
        ),
        (
            "AccountNonceApi_account_nonce",
            "{\"account\":42}",
            "argument account: expected sp_core::crypto::AccountId32",
        ),
    ] {
        let out = run(&[
            "call", "--rpc", &node.url, "--method", method, "--args", args,
        ]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains(expected) && !stderr.contains("invalid metadata:"),
            "{stderr}"
        );
    }
}

#[test]
fn oracle_mismatch_exits_three_after_emitting_parseable_json_and_summary_to_stderr() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let node = call_node(true);
    let out = run(&["call", "--rpc", &node.url, "--method", "AccountNonceApi_account_nonce", "--args",
        &serde_json::json!({"account":frame_trace_replay::rpc::unhex("d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d").unwrap()}).to_string(), "--oracle", "--json", "-", "--summary"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["call"]["oracle_match"], false);
    assert_eq!(json["call"]["oracle_source"], "live");
    assert!(
        stderr.contains("MISMATCH") && stderr.contains("Top items by proof bytes"),
        "{stderr}"
    );
}

#[test]
fn metadata_results_have_bounded_hex_preview_and_full_json() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let node = call_node(false);
    let out = run(&[
        "call",
        "--rpc",
        &node.url,
        "--method",
        "Metadata_metadata",
        "--data",
        "0x",
        "--json",
        "-",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.len() < 3000 && stderr.contains("full value in --json"),
        "stderr bytes {}",
        stderr.len()
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let decoded = json["call"]["decoded"].as_str().unwrap();
    assert!(decoded.contains("0x") && decoded.len() > 100_000);
}

#[test]
fn unknown_block_hash_reports_block_not_found() {
    let node = MockNode::new(|_| serde_json::json!({"result":null}));
    let out = run(&[
        "call",
        "--rpc",
        &node.url,
        "--at",
        &format!("0x{}", "ff".repeat(32)),
        "--method",
        "Metadata_metadata_versions",
        "--data",
        "0x",
    ]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("block not found"));
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn mock_handler_assertions_are_reported_and_propagated_to_the_test_thread() {
    let node = MockNode::new(|_| panic!("handler invariant"));
    let rpc = frame_trace_replay::rpc::Rpc::new(&node.url).unwrap();
    let error = rpc
        .call("chain_getHeader", serde_json::json!([]))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("mock handler assertion failed: handler invariant")
    );
    assert!(matches!(
        error,
        frame_trace_replay::Error::RpcResponse { code: -32099, .. }
    ));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(node))).is_err());
}

#[test]
fn saved_block_json_drives_summary_and_multiblock_reports_without_replay() {
    let path = std::env::temp_dir().join(format!("frame-report-{}.json", std::process::id()));
    let path2 = path.with_extension("second.json");
    let output = run(&[
        "block",
        "--from-file",
        &fixture("model/two-extrinsics.json"),
        "--json",
        path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut block: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let count = block["totals"]["ops"].as_u64().unwrap();
    block["block_hash"] = "another block".into();
    std::fs::write(&path2, serde_json::to_vec(&block).unwrap()).unwrap();
    let summary = run(&["summary", "--from-json", path.to_str().unwrap()]);
    assert!(
        summary.status.success(),
        "{}",
        String::from_utf8_lossy(&summary.stderr)
    );
    assert!(summary.stderr.is_empty(), "summary should not replay");
    let report = run(&[
        "report",
        path.to_str().unwrap(),
        path2.to_str().unwrap(),
        "--json",
        "-",
    ]);
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(json["blocks"], 2);
    assert_eq!(json["totals"]["ops"], count * 2);
    let duplicate = run(&[
        "report",
        "--from-json",
        path.to_str().unwrap(),
        "--from-json",
        path.to_str().unwrap(),
    ]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate block hash"));
    for p in [path, path2] {
        std::fs::remove_file(p).unwrap();
    }
}

#[test]
fn report_last_pins_finalized_hash_and_exports_replay_statistics() {
    let _guard = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let artifact = frame_trace_replay::Artifact::load(std::path::Path::new(&fixture(
        "replay/people-transfer",
    )))
    .unwrap();
    let other = frame_trace_replay::Artifact::load(std::path::Path::new(&fixture(
        "replay/people-kill-prefix",
    )))
    .unwrap();
    // The mock aliases height 7 to the transfer fixture and height 8 to the kill
    // fixture; both pinned hashes and their execution proofs remain authentic.
    // This exercises concurrent scheduling independently of chain ancestry.
    let node = MockNode::new(move |request| {
        use serde_json::json;
        let method = request["method"].as_str().unwrap();
        if method == "chain_getFinalizedHead" {
            return json!({"result":other.block_hash});
        }
        if method == "chain_getBlockHash" {
            return json!({"result":if request["params"][0] == 7 {artifact.block_hash} else {other.block_hash}});
        }
        let at = if method == "state_getReadProof" {
            &request["params"][1]
        } else {
            &request["params"][0]
        };
        let first =
            *at == json!(artifact.block_hash) || *at == artifact.block["header"]["parentHash"];
        artifact_rpc(if first { &artifact } else { &other }, request)
    });
    let out = run(&["report", "--rpc", &node.url, "--last", "2", "--json", "-"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["blocks"], 2);
    assert!(json["totals"]["backend_cold"].as_u64().unwrap() > 0);
    assert_eq!(json["execution_proof_bytes"], 6479 + 6867);
    assert_eq!(json["totals"]["proof_bytes"], 6479 + 6867);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("root check: PASS").count(), 2);
    assert_eq!(stderr.matches("(measured: 0)").count(), 2);
}

#[test]
fn report_continues_after_bad_files_emits_errors_and_exits_nonzero() {
    let absent = fixture("model/absent-round5.json");
    let out = run(&[
        "report",
        &absent,
        &fixture("model/pre-e86f740.json"),
        "--json",
        "-",
    ]);
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["blocks"], 1);
    assert_eq!(json["errors"].as_array().unwrap().len(), 1);
    assert_eq!(json["errors"][0]["input"], absent);
    assert!(String::from_utf8_lossy(&out.stderr).contains("Errors"));
}

#[test]
fn report_help_and_write_errors_explain_options_and_paths() {
    let out = run(&["report", "--help"]);
    let text = String::from_utf8_lossy(&out.stdout);
    for expected in [
        "First block",
        "Last block",
        "Shared maximum",
        "compiled Wasm",
        "disk cache",
        "Maximum rows",
        "--sort-by",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    let out = run(&["summary", "--help"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("extrinsic, hook"));
    let path = std::env::temp_dir().join(format!(
        "frame-r5-absent-{}/report.json",
        std::process::id()
    ));
    let out = run(&[
        "report",
        &fixture("model/pre-e86f740.json"),
        "--json",
        path.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains(&format!("writing report JSON {}", path.display()))
    );
}

/// The traced People Wasm is the `:code` value node inside the `people-traced` artifact.
fn traced_people_wasm(tag: &str) -> std::path::PathBuf {
    let artifact =
        frame_trace_replay::Artifact::load(std::path::Path::new(&fixture("replay/people-traced")))
            .unwrap();
    let wasm = artifact
        .proof_nodes
        .iter()
        .map(|node| frame_trace_replay::rpc::unhex(node).unwrap())
        .max_by_key(Vec::len)
        .unwrap();
    assert!(
        wasm.len() > 1_000_000,
        "the largest node is the runtime code"
    );
    let path = std::env::temp_dir().join(format!(
        "frame-trace-traced-people-{tag}-{}.wasm",
        std::process::id()
    ));
    std::fs::write(&path, wasm).unwrap();
    path
}

#[test]
fn runtime_override_replays_untraced_block_with_traced_wasm_and_labels_it() {
    let _serial = REPLAY_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let wasm = traced_people_wasm("replay");
    let json = wasm.with_extension("json");
    let out = run(&[
        "block",
        "--offline",
        &fixture("replay/people-transfer"),
        "--runtime-override",
        wasm.to_str().unwrap(),
        "--json",
        json.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("root check: PASS"), "{stderr}");
    assert!(
        stderr.contains("RUNTIME OVERRIDE") && stderr.contains("code-hash check BYPASSED"),
        "{stderr}"
    );
    let block: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    let block_hash = block["block_hash"].clone();
    let label_code_hash = block["runtime_override"]["code_hash"].clone();
    let label = &block["runtime_override"];
    assert_eq!(
        label["chain_code_hash"],
        "0xee817a9ecd07f4c0f861dd6f2b5105fd316638b827d4d425ae062add4b651c1c"
    );
    assert_eq!(
        label["code_hash"],
        "0xba01b1de0c55e48c84c70a59381594c8540eb2482ac8e785908de120ce617074"
    );
    assert_eq!(label["path"], wasm.to_str().unwrap());
    assert_eq!(block["execution"]["root_check"], true);
    let wasm_spans: Vec<_> = block["runtime_spans"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["wasm"] == true)
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect();
    for expected in [
        "apply_extrinsic",
        "on_initialize: System",
        "transfer_keep_alive",
    ] {
        assert!(wasm_spans.iter().any(|n| n == expected), "{wasm_spans:?}");
    }

    // The chain's own code has no runtime spans and no override label.
    let plain = wasm.with_extension("plain.json");
    let out = run(&[
        "block",
        "--offline",
        &fixture("replay/people-kill-prefix"),
        "--json",
        plain.to_str().unwrap(),
    ]);
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("RUNTIME OVERRIDE"));
    let block: serde_json::Value = serde_json::from_slice(&std::fs::read(&plain).unwrap()).unwrap();
    assert!(block.get("runtime_override").is_none());
    assert!(
        block["runtime_spans"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["wasm"] == false)
    );

    // Saved JSON keeps the provenance: summary and (mixed) reports name the override first.
    let out = run(&["summary", "--from-json", json.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        stdout.starts_with("RUNTIME OVERRIDE: 1 of 1 block(s) used a runtime override"),
        "{stdout}"
    );
    assert!(stdout.contains("0xba01b1de0c55e48c84c70a59381594c8540eb2482ac8e785908de120ce617074"));
    let report = wasm.with_extension("report.json");
    let out = run(&[
        "report",
        json.to_str().unwrap(),
        plain.to_str().unwrap(),
        "--sort-by",
        "time",
        "--json",
        report.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.starts_with("RUNTIME OVERRIDE: 1 of 2 block(s) used a runtime override"),
        "{stdout}"
    );
    assert!(stdout.contains("Top items by storage time"), "{stdout}");
    let aggregate: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let overrides = aggregate["runtime_overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0]["block_hash"], block_hash);
    assert_eq!(overrides[0]["code_hash"], label_code_hash);
    assert_eq!(overrides[0]["path"], wasm.to_str().unwrap());
    assert_eq!(aggregate["blocks"], 2);

    // Saved artifacts never mix a substitute runtime's discovery with the chain's code.
    for command in ["block", "summary"] {
        let out = run(&[
            command,
            "--offline",
            &fixture("replay/people-transfer"),
            "--runtime-override",
            wasm.to_str().unwrap(),
            "--save",
            json.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(2), "{command}");
    }
    let out = run(&[
        "call",
        "--offline",
        &fixture("replay/call-people-nonce"),
        "--runtime-override",
        wasm.to_str().unwrap(),
        "--save",
        json.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    let out = run(&[
        "call",
        "--offline",
        &fixture("replay/call-people-nonce"),
        "--runtime-override",
        wasm.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("RUNTIME OVERRIDE"), "{stderr}");
    assert!(stderr.contains("RuntimeVersion matches"), "{stderr}");

    // Online replay through safe RPC accepts the same override.
    let artifact = frame_trace_replay::Artifact::load(std::path::Path::new(&fixture(
        "replay/people-transfer",
    )))
    .unwrap();
    let node = MockNode::new(move |request| artifact_rpc(&artifact, request));
    let out = run(&[
        "block",
        "--rpc",
        &node.url,
        "--block",
        "2",
        "--runtime-override",
        wasm.to_str().unwrap(),
        "--json",
        json.to_str().unwrap(),
    ]);
    drop(node);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("root check: PASS with RUNTIME OVERRIDE"),
        "{stderr}"
    );
    let block: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(block["runtime_override"]["code_hash"], label_code_hash);
    assert!(
        block["runtime_spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["wasm"] == true)
    );
    for path in [json, plain, report] {
        let _ = std::fs::remove_file(path);
    }
    let _ = std::fs::remove_file(wasm);
}

#[test]
fn bad_runtime_override_files_are_rejected_by_name_before_replay() {
    let wasm = traced_people_wasm("bad");
    let bytes = std::fs::read(&wasm).unwrap();
    let mut corrupt = b"\0asm\x01\0\0\0".to_vec();
    corrupt.extend([0xff; 64]);
    for (name, content) in [
        ("text", b"this is a text file\n".to_vec()),
        ("truncated", bytes[..bytes.len() / 2].to_vec()),
        ("corrupt", corrupt),
    ] {
        let path = wasm.with_extension(format!("{name}.wasm"));
        std::fs::write(&path, content).unwrap();
        for args in [
            vec!["block", "--offline", "unused-artifact"],
            vec!["call", "--offline", "unused-artifact"],
        ] {
            let mut args = args;
            args.extend(["--runtime-override", path.to_str().unwrap()]);
            let out = run(&args);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(1), "{name}: {stderr}");
            assert!(
                stderr.contains(&format!("--runtime-override {}", path.display())),
                "{name}: {stderr}"
            );
            assert!(
                !stderr.contains("unused-artifact") && !stderr.contains("root"),
                "{name}: {stderr}"
            );
        }
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_file(wasm).unwrap();
}
