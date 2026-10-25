//! E2: standalone safe-RPC block replay, with an offline proof verification pass.
mod backend;
mod capture;
mod proof;
mod rpc;

use anyhow::{anyhow, bail, Context, Result};
use backend::TracedBackend;
use capture::Capture;
use codec::{Decode, Encode};
use proof::ProofDb;
use rpc::{hex0, Rpc};
use sc_executor::WasmExecutor;
use serde_json::json;
use sp_core::{
    traits::{CallContext, RuntimeCode, WrappedRuntimeCode},
    Blake2Hasher, H256,
};
use sp_externalities::Extensions;
use sp_runtime::{
    generic,
    traits::{BlakeTwo256, Header as HeaderT},
    OpaqueExtrinsic,
};
use sp_state_machine::{Backend, OverlayedChanges, StateMachine};
use sp_trie::{proof_size_extension::ProofSizeExt, recorder::Recorder};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::atomic::Ordering, time::Instant};

#[cfg(test)]
mod conformance;

type Header = generic::Header<u32, BlakeTwo256>;
type Block = generic::Block<Header, OpaqueExtrinsic>;
type HostFunctions = (
    sp_io::SubstrateHostFunctions,
    cumulus_primitives_proof_size_hostfunction::storage_proof_size::HostFunctions,
    sp_statement_store::runtime_api::HostFunctions,
    sp_crypto_ec_utils::HostFunctionsRfc163,
);
type Executor = WasmExecutor<HostFunctions>;

fn execute(
    db: &ProofDb,
    root: H256,
    wasm: &[u8],
    heap_pages: Option<u64>,
    executor: &Executor,
    block: &Block,
    capture: &Capture,
) -> Result<(f64, usize)> {
    // Fresh recorder and overlay for EVERY run. Preloaded proof nodes must not
    // pre-populate the recorder: fee reclaim depends on its incremental size.
    let recorder = Recorder::<Blake2Hasher>::default();
    capture.0.lock().recorder = Some(recorder.clone());
    let backend = TracedBackend::new(db.clone(), root, recorder.clone());
    let mut overlay = OverlayedChanges::default();
    let mut extensions = Extensions::default();
    extensions.register(ProofSizeExt::new(recorder.clone()));
    let code_fetcher = WrappedRuntimeCode(wasm.into());
    let code = RuntimeCode {
        code_fetcher: &code_fetcher,
        heap_pages,
        hash: sp_io::hashing::blake2_256(wasm).to_vec(),
    };
    let data = block.encode();
    let dispatch = tracing::Dispatch::new(capture.clone());
    let start = Instant::now();
    let result = tracing::dispatcher::with_default(&dispatch, || {
        let span = tracing::info_span!(target: "e2", "replay");
        let _entered = span.enter();
        StateMachine::new(
            &backend,
            &mut overlay,
            executor,
            "Core_execute_block",
            &data,
            &mut extensions,
            &code,
            CallContext::Onchain { import: true },
        )
        .set_parent_hash(*block.header.parent_hash())
        .execute()
    });
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    result.map_err(|e| anyhow!("Core_execute_block: {e}"))?;
    Ok((elapsed, recorder.estimate_encoded_size()))
}

fn write(path: PathBuf, v: &impl serde::Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--offline") {
        return offline(&args);
    }
    if args.len() < 4 {
        bail!("usage: frame-trace-e2 HTTP_RPC BLOCK_HASH|head OUTPUT_DIR [WARM_REPEATS=5]");
    }
    let out = PathBuf::from(&args[3]);
    fs::create_dir_all(&out)?;
    let rpc = Rpc::new(args[1].clone())?;
    let total = Instant::now();
    let block_json = rpc.call(
        "chain_getBlock",
        if args[2] == "head" {
            json!([])
        } else {
            json!([args[2]])
        },
    )?;
    write(out.join("block.json"), &block_json)?;
    let mut block: Block =
        serde_json::from_value(block_json["block"].clone()).context("decode block JSON")?;
    let hash = block.header.hash();
    if args[2] != "head" && args[2].parse::<H256>()? != hash {
        bail!("block header hash does not match request");
    }
    let parent = *block.header.parent_hash();
    let parent_header: Header =
        serde_json::from_value(rpc.call("chain_getHeader", json!([parent]))?)?;
    if parent_header.hash() != parent {
        bail!("parent header hash does not match");
    }
    let root = *parent_header.state_root();
    let version = rpc.call("state_getRuntimeVersion", json!([parent]))?;
    write(out.join("parent-header.json"), &parent_header)?;
    write(out.join("runtime-version.json"), &version)?;
    block
        .header
        .digest_mut()
        .logs
        .retain(|d| !matches!(d, generic::DigestItem::Seal(_, _)));
    let db = ProofDb::new(rpc.clone(), parent);
    db.fetch(b":code")?;
    // Authenticate the runtime itself against the parent's real root. This
    // separate backend has no proof recorder and does not affect execution fees.
    let initial =
        sp_state_machine::TrieBackendBuilder::<_, Blake2Hasher>::new(db.clone(), root).build();
    let wasm = initial
        .storage(b":code")
        .map_err(|e| anyhow!(e))?
        .context("no runtime :code")?;
    let heap_pages = initial
        .storage(b":heappages")
        .map_err(|e| anyhow!(e))?
        .and_then(|b| u64::decode(&mut &b[..]).ok());
    let executor = WasmExecutor::<HostFunctions>::builder()
        .with_max_runtime_instances(2)
        .with_cache_path("/tmp/pm-audit/e2-target/wasm-cache")
        .build();
    let discovery = Capture::new();
    let discovered = execute(&db, root, &wasm, heap_pages, &executor, &block, &discovery);
    let acquisition_ms = total.elapsed().as_secs_f64() * 1000.;
    write(out.join("rpc-calls.json"), &*rpc.calls.lock())?;
    write(out.join("proof-misses.json"), &*db.misses.lock())?;
    write(
        out.join("proof.json"),
        &json!({"at": parent, "root": root, "proof": db.export()}),
    )?;
    write(
        out.join("discovery-events.json"),
        &discovery.0.lock().events,
    )?;
    if let Err(error) = &discovered {
        write(
            out.join("failure.json"),
            &json!({"hash": hash, "error": format!("{error:#}"),
            "rpc_counts": rpc.counts(), "acquisition_ms": acquisition_ms}),
        )?;
    }
    let (discovery_execute_ms, _) = discovered?;
    db.online.store(false, Ordering::Relaxed);
    let calls_before = rpc.calls.lock().len();
    let repeats: usize = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(5);
    if repeats == 0 {
        bail!("at least one warm replay required");
    }
    let mut times = Vec::new();
    let mut proof_bytes = 0;
    let mut measured = None;
    let mut warm_extrinsic_ms: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    for _ in 0..repeats {
        let capture = Capture::new();
        let (ms, bytes) = execute(&db, root, &wasm, heap_pages, &executor, &block, &capture)?;
        for (index, s) in capture
            .0
            .lock()
            .spans
            .iter()
            .filter(|s| s.wasm && s.name == "apply_extrinsic")
            .enumerate()
        {
            warm_extrinsic_ms
                .entry(index)
                .or_default()
                .push(s.duration_ns as f64 / 1_000_000.);
        }
        times.push(ms);
        proof_bytes = bytes;
        measured = Some(capture);
    }
    if rpc.calls.lock().len() != calls_before {
        bail!("offline verification unexpectedly used RPC");
    }
    let measured = measured.unwrap();
    let d = measured.0.lock();
    write(out.join("events.json"), &d.events)?;
    write(out.join("spans.json"), &d.spans)?;
    let mut state_methods: BTreeMap<String, usize> = BTreeMap::new();
    for e in &d.events {
        if e.target == "state" {
            *state_methods
                .entry(
                    e.fields
                        .get("method")
                        .cloned()
                        .unwrap_or_else(|| e.fields.get("message").cloned().unwrap_or_default()),
                )
                .or_default() += 1;
        }
    }
    let extrinsics: Vec<_> = d
        .spans
        .iter()
        .filter(|s| s.wasm && s.name == "apply_extrinsic")
        .enumerate()
        .map(|(index, s)| {
            json!({"index": index, "duration_ns": s.duration_ns, "span_id": s.id,
            "proof_start_bytes": s.proof_start_bytes, "proof_end_bytes": s.proof_end_bytes,
            "proof_delta_bytes": s.proof_end_bytes.saturating_sub(s.proof_start_bytes)})
        })
        .collect();
    let host_storage: Vec<_> = d.spans.iter().filter(|s| s.target.starts_with("sp_io::storage") || s.target.starts_with("sp_io::default_child_storage"))
        .map(|s| json!({"id": s.id, "parent_id": s.parent_id, "method": s.name, "duration_ns": s.duration_ns})).collect();
    write(out.join("storage-durations.json"), &host_storage)?;
    let summary = json!({"hash": hash, "number": block.header.number, "parent": parent,
        "root_check": true, "execution": "Core_execute_block", "offline_verification": true,
        "acquisition_ms": acquisition_ms, "discovery_execute_ms": discovery_execute_ms,
        "warm_replay_ms": times, "warm_rpc_calls": 0, "rpc_calls": calls_before,
        "warm_extrinsic_ms": warm_extrinsic_ms,
        "rpc_counts": rpc.counts(), "proof_nodes": db.export().len(), "fetched_node_bytes": db.bytes(),
        "recorded_execution_proof_bytes": proof_bytes, "state_methods": state_methods,
        "spans": d.spans.len(), "wasm_spans": d.spans.iter().filter(|s| s.wasm).count(),
        "parented_wasm_spans": d.spans.iter().filter(|s| s.wasm && s.parent_id.is_some()).count(),
        "storage_host_durations": host_storage.len(), "extrinsic_durations": extrinsics,
        "runtime": version, "wasm_bytes": wasm.len(), "wasm_blake2": hex0(&sp_io::hashing::blake2_256(&wasm)) });
    write(out.join("summary.json"), &summary)?;
    println!("{}", serde_json::to_string(&summary)?);
    Ok(())
}

fn offline(args: &[String]) -> Result<()> {
    let out = PathBuf::from(
        args.get(2)
            .context("usage: --offline ARTIFACT_DIR [--tamper-root]")?,
    );
    let read = |name: &str| -> Result<serde_json::Value> {
        Ok(serde_json::from_slice(&fs::read(out.join(name))?)?)
    };
    let mut block: Block = serde_json::from_value(read("block.json")?["block"].clone())?;
    let parent: Header = serde_json::from_value(read("parent-header.json")?)?;
    let proof = read("proof.json")?;
    if parent.hash() != *block.header.parent_hash() || proof["root"] != json!(parent.state_root()) {
        bail!("offline parent/root mismatch");
    }
    let db = ProofDb::new(Rpc::new("http://127.0.0.1:1".into())?, parent.hash());
    db.insert_nodes(
        proof["proof"]
            .as_array()
            .context("proof nodes")?
            .iter()
            .map(|v| rpc::unhex(v.as_str().unwrap()))
            .collect::<Result<Vec<_>>>()?,
    );
    db.online.store(false, Ordering::Relaxed);
    let initial = sp_state_machine::TrieBackendBuilder::<_, Blake2Hasher>::new(
        db.clone(),
        *parent.state_root(),
    )
    .build();
    let wasm = initial
        .storage(b":code")
        .map_err(|e| anyhow!(e))?
        .context("offline :code missing")?;
    let heap = initial
        .storage(b":heappages")
        .map_err(|e| anyhow!(e))?
        .and_then(|b| u64::decode(&mut &b[..]).ok());
    block
        .header
        .digest_mut()
        .logs
        .retain(|d| !matches!(d, generic::DigestItem::Seal(_, _)));
    let tamper = args.get(3).map(String::as_str) == Some("--tamper-root");
    if tamper {
        block.header.state_root = H256::repeat_byte(0x42);
    }
    let executor = WasmExecutor::<HostFunctions>::builder()
        .with_cache_path("/tmp/pm-audit/e2-target/wasm-cache")
        .build();
    let capture = Capture::new();
    let result = execute(
        &db,
        *parent.state_root(),
        &wasm,
        heap,
        &executor,
        &block,
        &capture,
    );
    let report = if tamper {
        if result.is_ok() {
            bail!("tampered state root was accepted");
        }
        let original: Block = serde_json::from_value(read("block.json")?["block"].clone())?;
        let computed_root = capture
            .0
            .lock()
            .events
            .iter()
            .rev()
            .find(|e| e.fields.get("method").map(String::as_str) == Some("StorageRoot"))
            .and_then(|e| e.fields.get("storage_root"))
            .cloned()
            .context("tamper run did not reach root calculation")?;
        if computed_root != hex::encode(original.header.state_root().as_bytes()) {
            bail!("tamper control failed before reproducing the authentic state root");
        }
        json!({"tampered_root_rejected": true, "computed_root": format!("0x{computed_root}"),
            "tampered_expected_root": block.header.state_root(), "error": format!("{:#}", result.unwrap_err()), "rpc_calls": 0})
    } else {
        let (ms, proof_bytes) = result?;
        json!({"root_check": true, "offline": true, "elapsed_ms": ms, "proof_bytes": proof_bytes, "rpc_calls": 0})
    };
    write(
        out.join(if tamper {
            "tamper-control.json"
        } else {
            "offline-verification.json"
        }),
        &report,
    )?;
    println!("{report}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
