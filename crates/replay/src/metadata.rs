//! Metadata used for names and JSON arguments must come from authenticated code/state.
use crate::*;

fn negotiate(mut run: impl FnMut(&str, &[u8]) -> Result<Vec<u8>>) -> Result<String> {
    for version in [16u32, 15] {
        let bytes = match run("Metadata_metadata_at_version", &version.encode()) {
            Ok(bytes) => bytes,
            Err(Error::MethodNotFound(_)) => break,
            Err(error) => return Err(error),
        };
        let mut cursor = bytes.as_slice();
        let decoded = Option::<Vec<u8>>::decode(&mut cursor)
            .map_err(|e| Error::Invalid(format!("local metadata response: {e}")))?;
        if !cursor.is_empty() {
            return Err(Error::Invalid("local metadata: trailing bytes".into()));
        }
        if let Some(bytes) = decoded {
            return Ok(rpc::hex0(&bytes));
        }
    }
    let bytes = run("Metadata_metadata", &[])?;
    let mut cursor = bytes.as_slice();
    let decoded = Vec::<u8>::decode(&mut cursor)
        .map_err(|e| Error::Invalid(format!("local V14 metadata response: {e}")))?;
    if !cursor.is_empty() {
        return Err(Error::Invalid("local V14 metadata: trailing bytes".into()));
    }
    Ok(rpc::hex0(&decoded))
}

pub(crate) fn load(
    artifact: &mut Artifact,
    db: &ProofDb,
    wasm: &[u8],
    heap: Option<u64>,
    session: &ReplaySession,
    state: &Header,
) -> Result<()> {
    let hash = H256::from(sp_io::hashing::blake2_256(wasm));
    if let Some((metadata, nodes)) = session
        .metadata
        .lock()
        .get(&hash)
        .and_then(|roots| roots.get(state.state_root()))
        .cloned()
    {
        db.retain_nodes(&nodes);
        artifact.metadata = metadata;
        return Ok(());
    }
    let executor = session.executor_for(wasm);
    let result = negotiate(|method, data| {
        execute_method(
            db,
            *state.state_root(),
            wasm,
            heap,
            &executor,
            method,
            data,
            state.hash(),
            CallContext::Offchain,
            &Capture::new(),
        )
        .map(|(_, output)| output)
    });
    // A failed optional metadata export must not poison a subsequent --data execution.
    db.failure.lock().take();
    match result {
        Ok(metadata) => {
            let mut cache = session.metadata.lock();
            if cache.len() >= 4 {
                cache.clear();
            }
            let roots = cache.entry(hash).or_default();
            if roots.len() >= 3 {
                roots.clear();
            }
            roots.insert(*state.state_root(), (metadata.clone(), db.used_nodes()));
            artifact.metadata = metadata;
        }
        Err(error) if error.is_transient() || matches!(error, Error::Cancelled) => {
            return Err(error);
        }
        Err(error) if session.rpc_metadata_fallback => {
            eprintln!(
                "warning: local metadata unavailable ({error}); RPC METADATA FALLBACK (UNAUTHENTICATED)"
            );
            artifact.metadata = match db.rpc() {
                Some(rpc) => metadata_result(rpc.call("state_getMetadata", json!([state.hash()])))?,
                None => artifact.metadata.clone(),
            };
        }
        Err(error) => {
            eprintln!(
                "notice: local metadata unavailable ({error}); using unnamed storage and hex results"
            );
            artifact.metadata.clear();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negotiates_v16_v15_and_legacy_v14_without_rpc_metadata() {
        for supported in [16u32, 15, 14] {
            let mut versions = Vec::new();
            let result = negotiate(|method, data| {
                if method == "Metadata_metadata" {
                    return Ok(b"v14".to_vec().encode());
                }
                let version = u32::decode(&mut &*data).expect("version");
                versions.push(version);
                Ok((version == supported)
                    .then(|| format!("v{version}").into_bytes())
                    .encode())
            })
            .expect("metadata");
            assert_eq!(result, rpc::hex0(format!("v{supported}").as_bytes()));
            assert_eq!(
                versions,
                if supported == 16 {
                    vec![16]
                } else {
                    vec![16, 15]
                }
            );
        }
        assert_eq!(
            negotiate(|method, _| if method.ends_with("at_version") {
                Err(Error::MethodNotFound(method.into()))
            } else {
                Ok(b"v14".to_vec().encode())
            })
            .expect("legacy"),
            rpc::hex0(b"v14")
        );
    }
}
