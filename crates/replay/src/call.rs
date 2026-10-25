//! Runtime API execution at a pinned state. Proofs authenticate reads; no block root assertion applies.
use crate::*;
use rpc::hex0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    pub method: String,
    /// SCALE-encoded arguments.
    pub data: String,
    /// Recorded output, checked again during offline execution.
    pub result: Option<String>,
    /// Optional node state_call output, kept separately from locally executed output.
    pub oracle: Option<String>,
}
impl Call {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.method.is_empty() || self.method.len() > 1024 {
            return Err(Error::Invalid(
                "runtime API method must have 1..1024 bytes".into(),
            ));
        }
        unhex(&self.data)?;
        for value in [&self.result, &self.oracle].into_iter().flatten() {
            unhex(value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CallReplay {
    pub capture: ReplayCapture,
    pub block_number: u64,
    pub result: Vec<u8>,
    pub oracle_match: Option<bool>,
    pub rpc_count: usize,
    pub measured_rpc_count: usize,
    pub proof_round_trips: usize,
    pub proof_bytes: usize,
    pub artifact: Artifact,
    /// Set only when the session executed a substitute runtime.
    pub runtime_override: Option<crate::RuntimeOverride>,
}

/// Acquire a call context. The selected block's own header/root/code are used, even at upgrades.
/// Call authenticate_metadata before encoding JSON arguments; preparation performs no metadata RPC.
pub fn prepare(rpc: &Rpc, selector: &str, method: &str) -> Result<Artifact> {
    let hash = match selector {
        "best" => None,
        "finalized" => Some(rpc.call("chain_getFinalizedHead", json!([]))?),
        hash if hash.starts_with("0x") => Some(json!(
            hash.parse::<H256>()
                .map_err(|e| Error::Invalid(format!("block hash: {e}")))?
        )),
        number => {
            let number: u32 = number
                .parse()
                .map_err(|e| Error::Invalid(format!("block number: {e}")))?;
            let hash = rpc.call("chain_getBlockHash", json!([number]))?;
            if hash.is_null() {
                return Err(Error::Invalid(format!("block {selector} not found")));
            }
            Some(hash)
        }
    };
    let header = rpc.call(
        "chain_getHeader",
        hash.as_ref().map_or_else(|| json!([]), |h| json!([h])),
    )?;
    if header.is_null() {
        return Err(Error::Invalid(format!("block not found: {selector}")));
    }
    let parsed: Header = serde_json::from_value(header.clone())
        .map_err(|e| Error::Rpc(format!("chain_getHeader: {e}")))?;
    let block_hash = parsed.hash();
    if let Some(hash) = hash {
        let requested: H256 = serde_json::from_value(hash)
            .map_err(|e| Error::Rpc(format!("block hash response: {e}")))?;
        if requested != block_hash {
            return Err(Error::Invalid(
                "call header hash does not match request".into(),
            ));
        }
    }
    Ok(Artifact {
        format_version: 1,
        kind: ArtifactKind::Call,
        call: Some(Call {
            method: method.into(),
            data: "0x".into(),
            result: None,
            oracle: None,
        }),
        block_hash,
        block: json!({"header": header, "extrinsics": []}),
        parent_header: header,
        runtime_version: rpc.call("state_getRuntimeVersion", json!([block_hash]))?,
        metadata: String::new(),
        proof_nodes: Vec::new(),
    })
}

/// Execute in fresh overlays for discovery and measurement. Neither pass commits writes.
pub fn replay(
    artifact: Artifact,
    rpc: Option<Rpc>,
    session: &ReplaySession,
    oracle: bool,
) -> Result<CallReplay> {
    with_maintenance(session, || replay_inner(artifact, rpc, session, oracle))
        .map_err(|e| crate::override_hint(session, e))
}

fn replay_inner(
    mut artifact: Artifact,
    rpc: Option<Rpc>,
    session: &ReplaySession,
    oracle: bool,
) -> Result<CallReplay> {
    let (_, state) = artifact.validate()?;
    if artifact.kind != ArtifactKind::Call {
        return Err(Error::Invalid("block artifact: use block --offline".into()));
    }
    if oracle && rpc.is_none() {
        return Err(Error::Invalid(
            "a live state_call check requires RPC".into(),
        ));
    }
    let root = *state.state_root();
    let db = open_db(&artifact, rpc.clone(), session, &state)?;
    let (wasm, heap_pages) = runtime_code(&db, root)?;
    let (wasm, runtime_override) = crate::select_runtime(session, wasm);
    metadata::load(&mut artifact, &db, &wasm, heap_pages, session, &state)?;
    db.mark_bootstrap();
    let call = artifact
        .call
        .as_ref()
        .ok_or_else(|| Error::Invalid("missing call".into()))?;
    let input = unhex(&call.data)?;
    let executor = session.executor_for(&wasm);
    let run = |capture: &Capture| {
        execute_method(
            &db,
            root,
            &wasm,
            heap_pages,
            &executor,
            &call.method,
            &input,
            state.hash(),
            CallContext::Offchain,
            capture,
        )
    };
    eprintln!(
        "call {} at #{}: loading runtime and preparing state",
        call.method,
        state.number()
    );
    run(&Capture::new())?;
    let capture = Capture::new();
    let (proof_bytes, output) = measured_run(&db, rpc.as_ref(), || run(&capture))?;
    if call
        .result
        .as_ref()
        .map(|s| unhex(s))
        .transpose()?
        .is_some_and(|expected| expected != output)
    {
        return Err(Error::Invalid(
            "recorded call result differs from local execution".into(),
        ));
    }
    let oracle_output = if oracle {
        let value = rpc
            .as_ref()
            .ok_or_else(|| Error::Invalid("state_call check needs RPC".into()))?
            .call("state_call", json!([call.method, call.data, state.hash()]))?;
        Some(unhex(value.as_str().ok_or_else(|| {
            Error::Rpc("state_call: non-string result".into())
        })?)?)
    } else {
        call.oracle.as_ref().map(|s| unhex(s)).transpose()?
    };
    let oracle_match = oracle_output.as_ref().map(|expected| expected == &output);
    let capture = captured(&capture, state.hash(), *state.parent_hash());
    let call = artifact
        .call
        .as_mut()
        .ok_or_else(|| Error::Invalid("missing call".into()))?;
    call.result = Some(hex0(&output));
    call.oracle = oracle_output.as_ref().map(|bytes| hex0(bytes));
    persist_export(&db, &mut artifact)?;
    Ok(CallReplay {
        capture,
        block_number: u64::from(*state.number()),
        result: output,
        oracle_match,
        rpc_count: rpc.as_ref().map_or(0, Rpc::count),
        measured_rpc_count: 0,
        proof_round_trips: db.round_trips.load(Ordering::Relaxed),
        proof_bytes,
        artifact,
        runtime_override,
    })
}

/// Load metadata by executing the authenticated runtime before encoding user arguments.
/// Stored/RPC metadata is never used implicitly.
pub fn authenticate_metadata(
    artifact: &mut Artifact,
    rpc: Option<Rpc>,
    session: &ReplaySession,
) -> Result<()> {
    with_maintenance(session, || {
        let (_, state) = artifact.validate()?;
        let db = open_db(artifact, rpc, session, &state)?;
        let (wasm, heap) = runtime_code(&db, *state.state_root())?;
        let (wasm, _) = crate::select_runtime(session, wasm);
        metadata::load(artifact, &db, &wasm, heap, session, &state)?;
        persist_export(&db, artifact)
    })
    .map_err(|e| crate::override_hint(session, e))
}
