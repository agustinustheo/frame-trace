//! Faithful block replay using safe RPC and SDK authenticated trie execution.
mod backend;
mod capture;
mod proof;
pub mod rpc;

use backend::TracedBackend;
use capture::Capture;
use codec::{Decode, Encode};
use frame_trace_model::ReplayCapture;
use proof::ProofDb;
use rpc::{Rpc, unhex};
use sc_executor::WasmExecutor;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sp_core::{
    Blake2Hasher, H256,
    traits::{CallContext, RuntimeCode, WrappedRuntimeCode},
};
use sp_externalities::Extensions;
use sp_runtime::{
    OpaqueExtrinsic, generic,
    traits::{BlakeTwo256, Header as HeaderT},
};
use sp_state_machine::{Backend, OverlayedChanges, StateMachine};
use sp_trie::{proof_size_extension::ProofSizeExt, recorder::Recorder};
use std::{
    fs,
    path::Path,
    sync::{OnceLock, atomic::Ordering},
};

#[cfg(test)]
mod conformance;
#[cfg(test)]
mod tests;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("RPC method not allowed: {0}")]
    MethodNotAllowed(String),
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("invalid replay input: {0}")]
    Invalid(String),
    #[error(
        "Core_execute_block failed (root mismatch, unsupported host function, or invalid block): {0}"
    )]
    Execution(String),
    #[error("artifact I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact JSON: {0}")]
    Json(#[from] serde_json::Error),
}

type Header = generic::Header<u32, BlakeTwo256>;
type Block = generic::Block<Header, OpaqueExtrinsic>;
// stable2609 omni-node's host functions. Missing imports are deliberately NOT allowed.
type HostFunctions = (
    sp_io::SubstrateHostFunctions,
    cumulus_primitives_proof_size_hostfunction::storage_proof_size::HostFunctions,
    sp_statement_store::runtime_api::HostFunctions,
    sp_crypto_ec_utils::HostFunctionsRfc163,
);
type Executor = WasmExecutor<HostFunctions>;

/// Portable v1 artifact. Metadata is auxiliary decoding data, never execution input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub format_version: u32,
    pub block_hash: H256,
    pub block: Value,
    pub parent_header: Value,
    pub runtime_version: Value,
    pub metadata: String,
    pub proof_nodes: Vec<String>,
}
impl Artifact {
    pub fn load(dir: &Path) -> Result<Self> {
        let bytes = zstd::stream::decode_all(fs::File::open(dir.join("artifact.json.zst"))?)?;
        let artifact: Self = serde_json::from_slice(&bytes)?;
        artifact.validate()?;
        Ok(artifact)
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(dir)?;
        let bytes = zstd::stream::encode_all(serde_json::to_vec(self)?.as_slice(), 9)?;
        // Write a complete file before replacing the artifact.
        fs::write(dir.join("artifact.json.zst.tmp"), bytes)?;
        fs::rename(
            dir.join("artifact.json.zst.tmp"),
            dir.join("artifact.json.zst"),
        )?;
        Ok(())
    }
    fn validate(&self) -> Result<(Block, Header)> {
        if self.format_version != 1 {
            return Err(Error::Invalid(format!(
                "unsupported artifact version {}",
                self.format_version
            )));
        }
        let block: Block = serde_json::from_value(self.block.clone())?;
        let parent: Header = serde_json::from_value(self.parent_header.clone())?;
        if block.header.hash() != self.block_hash {
            return Err(Error::Invalid("block header hash mismatch".into()));
        }
        if parent.hash() != *block.header.parent_hash() {
            return Err(Error::Invalid("parent header hash mismatch".into()));
        }
        Ok((block, parent))
    }
}

/// A successful result always passed Core's real root assertion and a zero-RPC measured run.
#[derive(Debug, Clone, Serialize)]
pub struct Replay {
    pub capture: ReplayCapture,
    pub block_number: u64,
    pub root_check: bool,
    pub rpc_count: usize,
    pub measured_rpc_count: usize,
    pub proof_bytes: usize,
    pub artifact: Artifact,
}

/// Fetch a number, hash, or `best` block. Context and proofs are pinned to its parent.
pub fn online(rpc: &Rpc, selector: &str) -> Result<Replay> {
    let initial_calls = rpc.count();
    let hash = if selector == "best" {
        None
    } else if selector.starts_with("0x") {
        Some(
            selector
                .parse::<H256>()
                .map_err(|e| Error::Invalid(format!("block hash: {e}")))?,
        )
    } else {
        let number: u32 = selector
            .parse()
            .map_err(|e| Error::Invalid(format!("block number: {e}")))?;
        Some(serde_json::from_value::<H256>(
            rpc.call("chain_getBlockHash", json!([number]))?,
        )?)
    };
    let response = rpc.call(
        "chain_getBlock",
        hash.map_or_else(|| json!([]), |h| json!([h])),
    )?;
    let block: Block = serde_json::from_value(response["block"].clone())?;
    if hash.is_some_and(|h| block.header.hash() != h) {
        return Err(Error::Invalid(
            "block header hash does not match request".into(),
        ));
    }
    let parent_hash = *block.header.parent_hash();
    let artifact = Artifact {
        format_version: 1,
        block_hash: block.header.hash(),
        block: response["block"].clone(),
        parent_header: rpc.call("chain_getHeader", json!([parent_hash]))?,
        runtime_version: rpc.call("state_getRuntimeVersion", json!([parent_hash]))?,
        metadata: rpc
            .call("state_getMetadata", json!([parent_hash]))?
            .as_str()
            .ok_or_else(|| Error::Invalid("metadata is not hex".into()))?
            .to_owned(),
        proof_nodes: Vec::new(),
    };
    let mut replay = replay(artifact, Some(rpc.clone()))?;
    replay.rpc_count = rpc.count().saturating_sub(initial_calls);
    Ok(replay)
}

pub fn offline(dir: &Path) -> Result<Replay> {
    replay(Artifact::load(dir)?, None)
}

/// Also accepts in-memory artifacts for callers that store them themselves.
pub fn replay(mut artifact: Artifact, rpc: Option<Rpc>) -> Result<Replay> {
    let (mut block, parent) = artifact.validate()?;
    let number = u64::from(*block.header.number());
    let root = *parent.state_root();
    block
        .header
        .digest_mut()
        .logs
        .retain(|d| !matches!(d, generic::DigestItem::Seal(_, _)));
    let db = ProofDb::new(rpc.clone(), parent.hash());
    db.insert_nodes(
        artifact
            .proof_nodes
            .iter()
            .map(|s| unhex(s))
            .collect::<Result<Vec<_>>>()?,
    );
    if rpc.is_some() && artifact.proof_nodes.is_empty() {
        db.fetch(None, &[b":code".to_vec(), b":heappages".to_vec()])?;
    }
    let initial =
        sp_state_machine::TrieBackendBuilder::<_, Blake2Hasher>::new(db.clone(), root).build();
    // Complete-key hints acquire :code and :heappages in at most one call each.
    let wasm = proof::with_key_hint(b":code", || initial.storage(b":code"))
        .map_err(|e| Error::Invalid(db.failure.lock().take().unwrap_or(e)))?
        .ok_or_else(|| {
            Error::Invalid("no authenticated parent :code; state may be pruned".into())
        })?;
    let heap_pages = proof::with_key_hint(b":heappages", || initial.storage(b":heappages"))
        .map_err(|e| Error::Invalid(db.failure.lock().take().unwrap_or(e)))?
        .map(|b| u64::decode(&mut &b[..]).map_err(|e| Error::Invalid(format!(":heappages: {e}"))))
        .transpose()?;
    // Reuse compiled modules across follow blocks. Trie overlays and recorders remain fresh.
    static EXECUTOR: OnceLock<Executor> = OnceLock::new();
    let executor = EXECUTOR.get_or_init(|| {
        Executor::builder()
            .with_max_runtime_instances(2)
            .with_allow_missing_host_functions(false)
            .build()
    });
    execute(
        &db,
        root,
        &wasm,
        heap_pages,
        executor,
        &block,
        &Capture::new(),
    )?;
    db.online.store(false, Ordering::SeqCst);
    let before = rpc.as_ref().map_or(0, Rpc::count);
    let capture = Capture::new();
    let proof_bytes = execute(&db, root, &wasm, heap_pages, executor, &block, &capture)?;
    let rpc_count = rpc.as_ref().map_or(0, Rpc::count);
    if rpc_count != before {
        return Err(Error::Invalid("measured run used RPC".into()));
    }
    let d = capture.0.lock();
    let capture = ReplayCapture {
        block_hash: format!("{:?}", artifact.block_hash),
        parent_hash: format!("{:?}", parent.hash()),
        events: serde_json::from_value(serde_json::to_value(&d.events)?)?,
        spans: serde_json::from_value(serde_json::to_value(&d.spans)?)?,
        transitions: d.transitions.clone(),
        duration_ns: d.spans.iter().filter_map(|s| s.end_ns).max().unwrap_or(0),
    };
    artifact.proof_nodes = db.export();
    Ok(Replay {
        capture,
        block_number: number,
        root_check: true,
        rpc_count,
        measured_rpc_count: 0,
        proof_bytes,
        artifact,
    })
}

#[allow(clippy::too_many_arguments)]
fn execute(
    db: &ProofDb,
    root: H256,
    wasm: &[u8],
    heap_pages: Option<u64>,
    executor: &Executor,
    block: &Block,
    capture: &Capture,
) -> Result<usize> {
    let recorder = Recorder::<Blake2Hasher>::default();
    capture.0.lock().recorder = Some(recorder.clone());
    let backend = TracedBackend::new(db.clone(), root, recorder.clone());
    let mut overlay = OverlayedChanges::default();
    let mut extensions = Extensions::default();
    extensions.register(ProofSizeExt::new(recorder.clone()));
    let fetcher = WrappedRuntimeCode(wasm.into());
    let code = RuntimeCode {
        code_fetcher: &fetcher,
        heap_pages,
        hash: sp_io::hashing::blake2_256(wasm).to_vec(),
    };
    let dispatch = tracing::Dispatch::new(capture.clone());
    let result = tracing::dispatcher::with_default(&dispatch, || {
        let span = tracing::info_span!(target: "frame_trace_replay", "replay");
        let _entered = span.enter();
        StateMachine::new(
            &backend,
            &mut overlay,
            executor,
            "Core_execute_block",
            &block.encode(),
            &mut extensions,
            &code,
            CallContext::Onchain { import: true },
        )
        .set_parent_hash(*block.header.parent_hash())
        .execute()
    });
    if let Some(failure) = db.failure.lock().take() {
        return Err(Error::Invalid(failure));
    }
    result.map_err(|e| Error::Execution(e.to_string()))?;
    Ok(recorder.estimate_encoded_size())
}

/// Hash a fetched SDK header directly, so best-head polling cannot race a separate number lookup.
pub fn header_hash(header: &Value) -> Result<String> {
    let header: Header = serde_json::from_value(header.clone())?;
    Ok(format!("{:?}", header.hash()))
}
