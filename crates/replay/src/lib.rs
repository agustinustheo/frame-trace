//! Faithful block replay using safe RPC and SDK authenticated trie execution.
mod artifact;
pub mod call;
mod metadata;
mod session;
pub use session::ReplaySession;
mod backend;
mod capture;
mod panic_scope;
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
use std::{path::Path, sync::atomic::Ordering};

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
    #[error("RPC {method} error {code}: {message}")]
    RpcResponse {
        method: String,
        code: i64,
        message: String,
    },
    #[error("runtime API method {0} not found")]
    MethodNotFound(String),
    #[error("RPC error: {0}")]
    TransientRpc(String),
    #[error("RPC cancelled")]
    Cancelled,
    #[error("RPC unavailable: {0}")]
    Unavailable(String),
    #[error("invalid replay input: {0}")]
    Invalid(String),
    #[error(
        "runtime execution failed (root mismatch, unsupported host function, or invalid block): {0}"
    )]
    Execution(String),
    #[error("runtime rejected the call input: {0}")]
    CallExecution(String),
    #[error("artifact I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    /// Only transport/deadline/rate-limit failures are eligible for block retries.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::TransientRpc(_))
    }
}

// Only used by the explicitly enabled unauthenticated metadata fallback.
fn metadata_result(result: Result<Value>) -> Result<String> {
    match result {
        Ok(Value::String(text)) => Ok(text),
        Ok(Value::Null) | Err(Error::Unavailable(_)) => Ok(String::new()),
        Ok(_) => Err(Error::Rpc("state_getMetadata: non-string result".into())),
        Err(error) => Err(error),
    }
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
    #[serde(default)]
    pub kind: ArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<call::Call>,
    pub block_hash: H256,
    pub block: Value,
    pub parent_header: Value,
    pub runtime_version: Value,
    pub metadata: String,
    pub proof_nodes: Vec<String>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    #[default]
    Block,
    Call,
}
impl Artifact {
    fn validate(&self) -> Result<(Block, Header)> {
        self.check_limits()?;
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
        match (self.kind, &self.call) {
            (ArtifactKind::Block, None) => {}
            (ArtifactKind::Call, Some(call)) => {
                call.validate()?;
                if parent.hash() != self.block_hash || !block.extrinsics.is_empty() {
                    return Err(Error::Invalid(
                        "call state header mismatch or nonempty block body".into(),
                    ));
                }
                return Ok((block, parent));
            }
            _ => return Err(Error::Invalid("artifact kind/call mismatch".into())),
        }
        if parent.hash() != *block.header.parent_hash() {
            return Err(Error::Invalid("parent header hash mismatch".into()));
        }
        Ok((block, parent))
    }
}

/// A substitute runtime executed instead of the state's authenticated `:code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RuntimeOverride {
    /// Blake2-256 of the authenticated parent-state `:code` that was NOT executed.
    pub chain_code_hash: H256,
    /// Blake2-256 of the substitute Wasm that was executed.
    pub override_code_hash: H256,
}

/// Returns the code to execute: the authenticated `:code`, or the session's explicit override.
fn select_runtime(session: &ReplaySession, chain: Vec<u8>) -> (Vec<u8>, Option<RuntimeOverride>) {
    match &session.runtime_override {
        None => (chain, None),
        Some(wasm) => {
            let info = RuntimeOverride {
                chain_code_hash: H256::from(sp_io::hashing::blake2_256(&chain)),
                override_code_hash: H256::from(sp_io::hashing::blake2_256(wasm)),
            };
            (wasm.to_vec(), Some(info))
        }
    }
}

/// A successful result always passed Core's real root assertion and a zero-RPC measured run.
#[derive(Debug, Clone, Serialize)]
pub struct Replay {
    pub capture: ReplayCapture,
    pub block_number: u64,
    pub root_check: bool,
    pub computed_state_root: H256,
    pub expected_state_root: H256,
    pub rpc_count: usize,
    pub measured_rpc_count: usize,
    /// Sequential proof acquisition waves, excluding context RPC and retries.
    pub proof_round_trips: usize,
    pub proof_bytes: usize,
    pub artifact: Artifact,
    /// Set only when the session executed a substitute runtime.
    pub runtime_override: Option<RuntimeOverride>,
}

/// Fetch a number, hash, or `best` block. Context and proofs are pinned to its parent.
pub fn online(rpc: &Rpc, selector: &str) -> Result<Replay> {
    online_with_session(rpc, selector, ReplaySession::shared()?)
}

pub fn online_with_session(rpc: &Rpc, selector: &str, session: &ReplaySession) -> Result<Replay> {
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
        let value = rpc.call("chain_getBlockHash", json!([number]))?;
        if value.is_null() {
            return Err(Error::Invalid(format!("block {selector} not found")));
        }
        Some(
            serde_json::from_value::<H256>(value)
                .map_err(|e| Error::Rpc(format!("chain_getBlockHash: malformed response: {e}")))?,
        )
    };
    let response = rpc.call(
        "chain_getBlock",
        hash.map_or_else(|| json!([]), |h| json!([h])),
    )?;
    if response.is_null() || response.get("block").is_some_and(Value::is_null) {
        return Err(Error::Invalid(format!("block {selector} not found")));
    }
    let block: Block = serde_json::from_value(response["block"].clone())
        .map_err(|e| Error::Rpc(format!("chain_getBlock: malformed response: {e}")))?;
    if hash.is_some_and(|h| block.header.hash() != h) {
        return Err(Error::Invalid(
            "block header hash does not match request".into(),
        ));
    }
    let parent_hash = *block.header.parent_hash();
    let artifact = Artifact {
        format_version: 1,
        kind: ArtifactKind::Block,
        call: None,
        block_hash: block.header.hash(),
        block: response["block"].clone(),
        parent_header: rpc.call("chain_getHeader", json!([parent_hash]))?,
        runtime_version: rpc.call("state_getRuntimeVersion", json!([parent_hash]))?,
        metadata: String::new(),
        proof_nodes: Vec::new(),
    };
    let mut replay = replay_with_session(artifact, Some(rpc.clone()), session)?;
    replay.rpc_count = rpc.count().saturating_sub(initial_calls);
    Ok(replay)
}

pub fn offline(dir: &Path) -> Result<Replay> {
    replay(Artifact::load(dir)?, None)
        .map_err(|e| artifact::context(&dir.join("artifact.json.zst"), e))
}

/// Also accepts in-memory artifacts for callers that store them themselves.
pub fn replay(artifact: Artifact, rpc: Option<Rpc>) -> Result<Replay> {
    replay_with_session(artifact, rpc, ReplaySession::shared()?)
}
pub fn replay_with_session(
    artifact: Artifact,
    rpc: Option<Rpc>,
    session: &ReplaySession,
) -> Result<Replay> {
    with_maintenance(session, || replay_inner(artifact, rpc, session))
}

fn with_maintenance<T>(session: &ReplaySession, run: impl FnOnce() -> Result<T>) -> Result<T> {
    let result = run();
    let maintenance = session.prune_disk_cache();
    match result {
        Ok(value) => {
            maintenance?;
            Ok(value)
        }
        Err(error) => {
            if let Err(cache_error) = maintenance {
                eprintln!("warning: cache eviction: {cache_error}");
            }
            Err(error)
        }
    }
}

fn replay_inner(
    mut artifact: Artifact,
    rpc: Option<Rpc>,
    session: &ReplaySession,
) -> Result<Replay> {
    let (mut block, parent) = artifact.validate()?;
    if artifact.kind != ArtifactKind::Block {
        return Err(Error::Invalid("call artifact: use call --offline".into()));
    }
    let number = u64::from(*block.header.number());
    let root = *parent.state_root();
    block
        .header
        .digest_mut()
        .logs
        .retain(|d| !matches!(d, generic::DigestItem::Seal(_, _)));
    let db = open_db(&artifact, rpc.clone(), session, &parent)?;
    if rpc.is_some() {
        eprintln!(
            "replay #{number}: prefetching prior access paths ({} in flight, batched keys)",
            rpc.as_ref().map_or(1, Rpc::parallelism)
        );
        let mut hints = session.predicted_keys();
        hints.extend(storage_hints(&artifact.metadata));
        // Fetch missing full paths before even loading :code: its ancestor nodes change each block.
        if session.nodes.lock().nodes.is_empty() {
            hints.extend([b":code".to_vec(), b":heappages".to_vec()]);
        }
        db.prefetch(&hints.into_iter().take(4096).collect::<Vec<_>>())?;
    }
    let (wasm, heap_pages) = runtime_code(&db, root)?;
    let (wasm, runtime_override) = select_runtime(session, wasm);
    metadata::load(&mut artifact, &db, &wasm, heap_pages, session, &parent)?;
    let executor = session.executor_for(&wasm);
    db.mark_bootstrap();
    eprintln!("replay #{number}: compiling/loading runtime and discovering state");
    if rpc.is_some() && session.speculative_discovery {
        db.speculative.store(true, Ordering::SeqCst);
        for _ in 0..8 {
            let _ = execute(
                &db,
                root,
                &wasm,
                heap_pages,
                &executor,
                &block,
                &Capture::new(),
            );
            if !db.acquire_pending()? {
                break;
            }
        }
        db.speculative.store(false, Ordering::SeqCst);
        db.failure.lock().take();
    }
    execute(
        &db,
        root,
        &wasm,
        heap_pages,
        &executor,
        &block,
        &Capture::new(),
    )?;
    let capture = Capture::new();
    let (proof_bytes, computed_state_root) = measured_run(&db, rpc.as_ref(), || {
        execute(&db, root, &wasm, heap_pages, &executor, &block, &capture)
    })?;
    let rpc_count = rpc.as_ref().map_or(0, Rpc::count);
    let capture = captured(&capture, artifact.block_hash, parent.hash());
    persist_export(&db, &mut artifact)?;
    Ok(Replay {
        capture,
        block_number: number,
        root_check: computed_state_root == *block.header.state_root(),
        computed_state_root,
        expected_state_root: *block.header.state_root(),
        rpc_count,
        measured_rpc_count: 0,
        proof_round_trips: db.round_trips.load(Ordering::Relaxed),
        proof_bytes,
        artifact,
        runtime_override,
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
) -> Result<(usize, H256)> {
    let (proof_bytes, _) = execute_method(
        db,
        root,
        wasm,
        heap_pages,
        executor,
        "Core_execute_block",
        &block.encode(),
        *block.header.parent_hash(),
        CallContext::Onchain { import: true },
        capture,
    )?;
    let computed = capture
        .0
        .lock()
        .events
        .iter()
        .rev()
        .find(|e| e.target == "state" && e.fields.get("method").is_some_and(|s| s == "StorageRoot"))
        .and_then(|e| e.fields.get("storage_root"))
        .and_then(|s| {
            format!("0x{}", s.trim_start_matches("0x"))
                .parse::<H256>()
                .ok()
        })
        .ok_or_else(|| Error::Execution("runtime did not report a computed state root".into()))?;
    if computed != *block.header.state_root() {
        return Err(Error::Execution(format!(
            "state root mismatch: computed {computed:?}, header {:?}",
            block.header.state_root()
        )));
    }
    Ok((proof_bytes, computed))
}

#[allow(clippy::too_many_arguments)]
fn execute_method(
    db: &ProofDb,
    root: H256,
    wasm: &[u8],
    heap_pages: Option<u64>,
    executor: &Executor,
    method: &str,
    data: &[u8],
    parent_hash: H256,
    context: CallContext,
    capture: &Capture,
) -> Result<(usize, Vec<u8>)> {
    let _abort_guard = sp_panic_handler::AbortGuard::never_abort();
    let recorder = Recorder::<Blake2Hasher>::default();
    capture.0.lock().recorder = Some(recorder.clone());
    let backend = TracedBackend::new(db.clone(), root, recorder.clone());
    let mut overlay = OverlayedChanges::default();
    let mut extensions = Extensions::default();
    // SDK ExecutionExtensions always supplies the executor-backed version reader.
    extensions.register(sp_core::traits::ReadRuntimeVersionExt::new(
        executor.clone(),
    ));
    // Import with proof recording supplies ProofSizeExt; ordinary state_call does not.
    // Keep the backend recorder for reporting without exposing it to call runtimes.
    if matches!(context, CallContext::Onchain { import: true }) {
        extensions.register(ProofSizeExt::new(recorder.clone()));
    }
    let fetcher = WrappedRuntimeCode(wasm.into());
    let code = RuntimeCode {
        code_fetcher: &fetcher,
        heap_pages,
        hash: sp_io::hashing::blake2_256(wasm).to_vec(),
    };
    let dispatch = tracing::Dispatch::new(capture.clone());
    let result = panic_scope::catch(|| {
        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!(target: "frame_trace_replay", "replay");
            let _entered = span.enter();
            StateMachine::new(
                &backend,
                &mut overlay,
                executor,
                method,
                data,
                &mut extensions,
                &code,
                context,
            )
            .set_parent_hash(parent_hash)
            .execute()
        })
    });
    if let Some(failure) = db.failure.lock().take() {
        return Err(db.take_error(failure));
    }
    let execution_error = |text: String| {
        if method == "Core_execute_block" {
            Error::Execution(text)
        } else {
            Error::CallExecution(
                text.lines()
                    .take_while(|l| !l.to_ascii_lowercase().contains("backtrace"))
                    .take_while(|l| !l.trim_start().starts_with("0:"))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
    };
    let output = result
        .map_err(|e| {
            execution_error(
                e.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "executor panicked".into()),
            )
        })?
        .map_err(|e| {
            let text = e.to_string();
            if text.contains("Exported method") && text.contains("not found") {
                Error::MethodNotFound(method.into())
            } else {
                execution_error(text)
            }
        })?;
    Ok((recorder.estimate_encoded_size(), output))
}

/// Hash a fetched SDK header directly, so best-head polling cannot race a separate number lookup.
pub fn header_hash(header: &Value) -> Result<String> {
    let header: Header = serde_json::from_value(header.clone())?;
    Ok(format!("{:?}", header.hash()))
}

fn open_db(
    artifact: &Artifact,
    rpc: Option<Rpc>,
    session: &ReplaySession,
    state: &Header,
) -> Result<ProofDb> {
    let db = ProofDb::with_session(rpc, state.hash(), session.clone());
    db.set_root(*state.state_root());
    db.insert_nodes(
        artifact
            .proof_nodes
            .iter()
            .map(|s| unhex(s))
            .collect::<Result<Vec<_>>>()?,
    );
    Ok(db)
}

fn persist_export(db: &ProofDb, artifact: &mut Artifact) -> Result<()> {
    db.persist()?;
    artifact.proof_nodes = db.export();
    Ok(())
}

fn measured_run<T>(db: &ProofDb, rpc: Option<&Rpc>, run: impl FnOnce() -> Result<T>) -> Result<T> {
    db.online.store(false, Ordering::SeqCst);
    db.begin_measured();
    without_rpc(rpc, run)
}

fn without_rpc<T>(rpc: Option<&Rpc>, run: impl FnOnce() -> Result<T>) -> Result<T> {
    let before = rpc.map_or(0, Rpc::count);
    let result = run();
    if rpc.map_or(0, Rpc::count) != before {
        return Err(Error::Invalid("measured run used RPC".into()));
    }
    result
}

/// Metadata is only a source of speculative key hints; authentication always comes from trie traversal.
fn storage_hints(metadata: &str) -> Vec<Vec<u8>> {
    use frame_metadata::{RuntimeMetadata, RuntimeMetadataPrefixed};
    let Ok(bytes) = unhex(metadata) else {
        return Vec::new();
    };
    let Ok(RuntimeMetadataPrefixed(_, metadata)) = RuntimeMetadataPrefixed::decode(&mut &bytes[..])
    else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    macro_rules! hints {
        ($md:expr) => {
            for pallet in $md.pallets {
                if let Some(storage) = pallet.storage {
                    for entry in storage.entries {
                        let mut key =
                            sp_crypto_hashing::twox_128(storage.prefix.as_bytes()).to_vec();
                        key.extend(sp_crypto_hashing::twox_128(entry.name.as_bytes()));
                        keys.push(key);
                    }
                }
            }
        };
    }
    match metadata {
        RuntimeMetadata::V14(md) => hints!(md),
        RuntimeMetadata::V15(md) => hints!(md),
        RuntimeMetadata::V16(md) => hints!(md),
        _ => {}
    }
    keys
}

fn runtime_code(db: &ProofDb, root: H256) -> Result<(Vec<u8>, Option<u64>)> {
    let initial =
        sp_state_machine::TrieBackendBuilder::<_, Blake2Hasher>::new(db.clone(), root).build();
    // Complete-key hints acquire :code and :heappages in at most one call each.
    let wasm = proof::with_key_hint(b":code", || initial.storage(b":code"))
        .map_err(|e| db.take_error(db.failure.lock().take().unwrap_or(e)))?
        .ok_or_else(|| Error::Invalid("no authenticated state :code".into()))?;
    let heap_pages = proof::with_key_hint(b":heappages", || initial.storage(b":heappages"))
        .map_err(|e| db.take_error(db.failure.lock().take().unwrap_or(e)))?
        .map(|b| u64::decode(&mut &b[..]).map_err(|e| Error::Invalid(format!(":heappages: {e}"))))
        .transpose()?;
    Ok((wasm, heap_pages))
}

fn captured(capture: &Capture, block_hash: H256, parent_hash: H256) -> ReplayCapture {
    let data = capture.0.lock();
    ReplayCapture {
        block_hash: format!("{block_hash:?}"),
        parent_hash: format!("{parent_hash:?}"),
        events: data.events.clone(),
        spans: data.spans.iter().map(capture::Span::to_model).collect(),
        transitions: data.transitions.clone(),
        duration_ns: data
            .spans
            .iter()
            .filter_map(|s| s.end_ns)
            .max()
            .unwrap_or(0),
    }
}
