//! Metadata used for names and JSON arguments must come from authenticated code/state.
use crate::*;
use std::collections::BTreeMap;

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
    let cached = session.metadata.lock().get(&hash).cloned();
    if let Some(entry) = cached {
        use sp_state_machine::Backend;
        let backend = TracedBackend::new(db.clone(), *state.state_root(), Default::default());
        let validation = entry.matches(*state.state_root(), |key, child| {
            match child {
                Some(child) => backend
                    .child_storage_hash(&sp_core::storage::ChildInfo::new_default(child), key),
                None => backend.storage_hash(key),
            }
            .map_err(|e| db.take_error(e))
        });
        let valid = match validation {
            Err(error) if error.is_transient() || matches!(error, Error::Cancelled) => {
                return Err(error);
            }
            other => other.unwrap_or(false),
        };
        db.failure.lock().take();
        if valid {
            artifact.metadata = entry.metadata;
            return Ok(());
        }
    }
    #[cfg(test)]
    session.metadata_runs.fetch_add(1, Ordering::Relaxed);
    let executor = session.executor_for(wasm);
    let capture = Capture::new();
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
            &capture,
        )
        .map(|(_, output)| output)
    });
    // A failed optional metadata export must not poison a subsequent --data execution.
    db.failure.lock().take();
    match result {
        Ok(metadata) => {
            let events = &capture.0.lock().events;
            let mut reads = BTreeMap::new();
            let mut iteration = false;
            for event in events {
                let method = event
                    .fields
                    .get("method")
                    .map(String::as_str)
                    .unwrap_or_default();
                if method == "BackendRead" {
                    let key = unhex(&event.fields["key"])?;
                    let child = event
                        .fields
                        .get("child_info")
                        .filter(|c| !c.is_empty())
                        .map(|c| unhex(c))
                        .transpose()?;
                    let value = event
                        .fields
                        .get("value_hash")
                        .filter(|v| *v != "none")
                        .map(|v| v.parse::<H256>().map_err(|e| Error::Invalid(e.to_string())))
                        .transpose()?;
                    reads.insert((key, child), value);
                } else if method.starts_with("Backend") {
                    iteration = true;
                }
            }
            let mut cache = session.metadata.lock();
            if cache.len() >= 4 {
                cache.clear();
            }
            cache.insert(
                hash,
                session::MetadataEntry {
                    metadata: metadata.clone(),
                    reads: reads
                        .into_iter()
                        .map(|((key, child), value)| (key, child, value))
                        .collect(),
                    iteration_root: iteration.then_some(*state.state_root()),
                },
            );
            artifact.metadata = metadata;
        }
        Err(error) if error.is_transient() || matches!(error, Error::Cancelled) => {
            return Err(error);
        }
        Err(error) if session.rpc_metadata_fallback => {
            eprintln!(
                "warning: local metadata unavailable ({error}); UNAUTHENTICATED METADATA (RPC online; stored artifact offline)"
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

impl session::MetadataEntry {
    fn matches(
        &self,
        root: H256,
        mut read: impl FnMut(&[u8], Option<&[u8]>) -> Result<Option<H256>>,
    ) -> Result<bool> {
        if self.iteration_root.is_some_and(|r| r != root) {
            return Ok(false);
        }
        for (key, child, expected) in &self.reads {
            if read(key, child.as_deref())? != *expected {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn dependency_values_invalidate_metadata_but_state_roots_do_not() {
        let mut entry = session::MetadataEntry {
            metadata: "metadata".into(),
            reads: vec![],
            iteration_root: None,
        };
        assert!(
            entry
                .matches(H256::repeat_byte(2), |_, _| panic!(
                    "pure metadata should not read storage"
                ))
                .unwrap()
        );
        entry
            .reads
            .push((b"key".to_vec(), None, Some(H256::repeat_byte(1))));
        assert!(
            entry
                .matches(H256::repeat_byte(3), |_, _| Ok(Some(H256::repeat_byte(1))))
                .unwrap()
        );
        assert!(
            !entry
                .matches(H256::repeat_byte(3), |_, _| Ok(Some(H256::repeat_byte(2))))
                .unwrap()
        );
        assert!(
            !entry
                .matches(H256::repeat_byte(3), |_, _| Ok(None))
                .unwrap()
        );
        entry.iteration_root = Some(H256::repeat_byte(4));
        assert!(
            !entry
                .matches(H256::repeat_byte(3), |_, _| panic!(
                    "root must invalidate iteration"
                ))
                .unwrap()
        );
    }
    #[test]
    fn people_metadata_storage_reads_are_captured_and_reused_across_blocks() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/replay");
        let session = ReplaySession::new(None).unwrap();
        for name in ["people-transfer", "people-kill-prefix"] {
            let mut artifact = Artifact::load(&base.join(name)).unwrap();
            let state: Header = serde_json::from_value(artifact.parent_header.clone()).unwrap();
            let db = open_db(&artifact, None, &session, &state).unwrap();
            let (wasm, heap) = runtime_code(&db, *state.state_root()).unwrap();
            load(&mut artifact, &db, &wasm, heap, &session, &state).unwrap();
            assert!(!artifact.metadata.is_empty());
            artifact.proof_nodes = db.export();
            let fresh = ReplaySession::new(None).unwrap();
            let db = open_db(&artifact, None, &fresh, &state).unwrap();
            let (wasm, heap) = runtime_code(&db, *state.state_root()).unwrap();
            load(&mut artifact, &db, &wasm, heap, &fresh, &state).unwrap();
            assert!(
                !artifact.metadata.is_empty(),
                "cache-hit save retains metadata proofs"
            );
        }
        assert_eq!(session.metadata_runs.load(Ordering::Relaxed), 1);
        let cache = session.metadata.lock();
        let entry = cache.values().next().unwrap();
        assert!(
            !entry.reads.is_empty(),
            "People metadata constants read storage"
        );
        assert!(entry.iteration_root.is_none());
    }
}
