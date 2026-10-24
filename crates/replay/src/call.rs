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
}

/// Acquire a call context. The selected block's own header/root/code are used, even at upgrades.
/// Encoding JSON arguments can use the returned metadata before proof acquisition starts.
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
        metadata: call_metadata(rpc, block_hash)?,
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
    let result = replay_inner(artifact, rpc, session, oracle);
    let maintenance = session.prune_disk_cache();
    match result {
        Ok(replay) => {
            maintenance?;
            Ok(replay)
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
    oracle: bool,
) -> Result<CallReplay> {
    let (_, state) = artifact.validate()?;
    if artifact.kind != ArtifactKind::Call {
        return Err(Error::Invalid("block artifact: use block --offline".into()));
    }
    if oracle && rpc.is_none() {
        return Err(Error::Invalid("a live oracle requires RPC".into()));
    }
    let call = artifact
        .call
        .as_ref()
        .ok_or_else(|| Error::Invalid("missing call".into()))?;
    let input = unhex(&call.data)?;
    let root = *state.state_root();
    let db = ProofDb::with_session(rpc.clone(), state.hash(), session.clone());
    db.set_root(root);
    db.insert_nodes(
        artifact
            .proof_nodes
            .iter()
            .map(|s| unhex(s))
            .collect::<Result<Vec<_>>>()?,
    );
    if rpc.is_some() && session.nodes.lock().nodes.is_empty() {
        db.fetch(None, &[b":code".to_vec(), b":heappages".to_vec()])?;
    }
    let (wasm, heap_pages) = runtime_code(&db, root)?;
    let run = |capture: &Capture| {
        execute_method(
            &db,
            root,
            &wasm,
            heap_pages,
            &session.executor,
            &call.method,
            &input,
            state.hash(),
            CallContext::Offchain,
            capture,
        )
    };
    eprintln!(
        "call {} at #{}: compiling/loading runtime and discovering state",
        call.method,
        state.number()
    );
    run(&Capture::new())?;
    db.online.store(false, Ordering::SeqCst);
    let capture = Capture::new();
    let (proof_bytes, output) = without_rpc(rpc.as_ref(), || run(&capture))?;
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
            .ok_or_else(|| Error::Invalid("oracle needs RPC".into()))?
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
    db.persist()?;
    artifact.proof_nodes = db.export();
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
    })
}

// Most nodes default state_getMetadata to V14, which has no runtime API types.
fn call_metadata(rpc: &Rpc, hash: H256) -> Result<String> {
    let fallback = metadata_result(rpc.call("state_getMetadata", json!([hash])))?;
    for version in [16u32, 15] {
        let result = rpc.call(
            "state_call",
            json!([
                "Metadata_metadata_at_version",
                hex0(&version.encode()),
                hash
            ]),
        );
        let value = match result {
            Ok(value) => value,
            Err(Error::Rpc(message))
                if message.contains("not found") || message.contains("-32601") =>
            {
                break;
            }
            Err(Error::Unavailable(_)) => break,
            Err(error) => return Err(error),
        };
        let text = value
            .as_str()
            .ok_or_else(|| Error::Rpc("metadata state_call: non-string result".into()))?;
        let bytes = unhex(text)?;
        let mut cursor = bytes.as_slice();
        let decoded = Option::<Vec<u8>>::decode(&mut cursor)
            .map_err(|e| Error::Rpc(format!("metadata state_call: {e}")))?;
        if !cursor.is_empty() {
            return Err(Error::Rpc("metadata state_call: trailing bytes".into()));
        }
        if let Some(bytes) = decoded {
            return Ok(hex0(&bytes));
        }
    }
    Ok(fallback)
}
