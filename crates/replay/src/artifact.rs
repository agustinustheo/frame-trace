//! Bounded artifact I/O, including content-addressed fixture-only blob references.
use crate::{Artifact, Error, Result, rpc::hex0};
use std::{fs, io::Read, path::Path};
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PROOF_NODES: usize = 100_000;
pub const MAX_PROOF_BYTES: usize = 24 * 1024 * 1024;
const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn context(path: &Path, error: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{}: {error}", path.display()))
}
fn decompressed(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let read = || -> std::io::Result<Vec<u8>> {
        let decoder = zstd::stream::read::Decoder::new(fs::File::open(path)?)?;
        let mut bytes = Vec::new();
        decoder.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    let bytes = read().map_err(|e| context(path, e))?;
    if bytes.len() > limit {
        return Err(context(
            path,
            format!("decompressed size exceeds {limit} bytes"),
        ));
    }
    Ok(bytes)
}
fn rehydrate(dir: &Path, value: &mut String) -> Result<()> {
    let Some(hash) = value.strip_prefix("blob:") else {
        return Ok(());
    };
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(context(&dir.join("artifact.json.zst"), "invalid blob hash"));
    }
    let path = dir
        .parent()
        .unwrap_or(dir)
        .join("blobs")
        .join(format!("{hash}.zst"));
    let bytes = decompressed(&path, MAX_BLOB_BYTES)?;
    if hex::encode(sp_io::hashing::blake2_256(&bytes)) != hash {
        return Err(context(&path, "blob content hash mismatch"));
    }
    *value = hex0(&bytes);
    Ok(())
}
impl Artifact {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("artifact.json.zst");
        let bytes = decompressed(&path, MAX_ARTIFACT_BYTES)?;
        let mut artifact: Self = serde_json::from_slice(&bytes).map_err(|e| context(&path, e))?;
        if artifact.proof_nodes.len() > MAX_PROOF_NODES {
            return Err(context(&path, "proof count limit exceeded"));
        }
        rehydrate(dir, &mut artifact.metadata)?;
        let mut total = 0usize;
        for node in &mut artifact.proof_nodes {
            rehydrate(dir, node)?;
            total = total.saturating_add(node.len() / 2);
            if total > MAX_PROOF_BYTES {
                return Err(context(&path, "total proof bytes limit exceeded"));
            }
        }
        artifact.validate().map_err(|e| context(&path, e))?;
        Ok(artifact)
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join("artifact.json.zst");
        let save = || -> Result<()> {
            self.validate()?;
            // Saving always emits self-contained hex, never fixture references.
            if self.metadata.starts_with("blob:") {
                return Err(Error::Invalid("unresolved metadata blob reference".into()));
            }
            fs::create_dir_all(dir)?;
            let raw = serde_json::to_vec(self)?;
            if raw.len() > MAX_ARTIFACT_BYTES {
                return Err(Error::Invalid("artifact size limit exceeded".into()));
            }
            let bytes = zstd::stream::encode_all(raw.as_slice(), 9)?;
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let temporary = dir.join(format!(
                ".artifact.{}.{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));

            fs::write(&temporary, bytes).map_err(|e| context(&temporary, e))?;
            if let Err(error) = fs::rename(&temporary, &path) {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
            Ok(())
        };
        save().map_err(|e| context(&path, e))
    }
    pub(crate) fn check_limits(&self) -> Result<()> {
        if self.proof_nodes.len() > MAX_PROOF_NODES {
            return Err(Error::Invalid("proof count limit exceeded".into()));
        }
        let total = self.proof_nodes.iter().try_fold(0usize, |n, s| {
            let bytes = s.strip_prefix("0x").unwrap_or(s);
            if bytes.len() % 2 != 0 || !bytes.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::Invalid("invalid hex proof node".into()));
            }
            Ok(n.saturating_add(bytes.len() / 2))
        })?;
        if total > MAX_PROOF_BYTES {
            return Err(Error::Invalid("total proof bytes limit exceeded".into()));
        }
        Ok(())
    }
}
