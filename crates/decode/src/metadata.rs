//! Runtime metadata loading and the storage prefix index.

use std::collections::HashMap;

use frame_decode::{
    constants::ConstantTypeInfo,
    extrinsics::decode_extrinsic,
    storage::{StorageEntryInfo, StorageInfo, StorageTypeInfo, decode_storage_key_with_info},
};
use frame_metadata::{RuntimeMetadata, RuntimeMetadataPrefixed};
use parity_scale_codec::Decode;
use scale_info::PortableRegistry;
use sp_crypto_hashing::twox_128;

use crate::{Error, KeyName, key::well_known, ss58};

/// Generic SS58 prefix, used when the runtime has no `System::SS58Prefix` constant.
const DEFAULT_SS58_PREFIX: u16 = 42;

/// Decoded values are cut to this many characters in labels.
const MAX_VALUE_CHARS: usize = 48;

/// Runtime metadata (V14, V15 or V16) indexed for decoding storage keys and extrinsics.
pub struct Metadata {
    runtime: Runtime,
    items: HashMap<[u8; 32], Item>,
    pallets: HashMap<[u8; 16], String>,
    ss58_prefix: u16,
}

enum Runtime {
    V14(frame_metadata::v14::RuntimeMetadataV14),
    V15(frame_metadata::v15::RuntimeMetadataV15),
    V16(frame_metadata::v16::RuntimeMetadataV16),
}

/// Runs `$body` with `$md` bound to the concrete metadata of any supported version.
macro_rules! with_runtime {
    ($runtime:expr, $md:ident => $body:expr) => {
        match $runtime {
            Runtime::V14($md) => $body,
            Runtime::V15($md) => $body,
            Runtime::V16($md) => $body,
        }
    };
}

struct Item {
    pallet: String,
    name: String,
    info: StorageInfo<'static, u32>,
}

/// The pallet and call of an extrinsic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallName {
    /// Pallet name, as in `construct_runtime!`.
    pub pallet: String,
    /// Call name.
    pub call: String,
}

impl std::fmt::Display for CallName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.pallet, self.call)
    }
}

impl Metadata {
    /// Loads SCALE-encoded metadata: `RuntimeMetadataPrefixed` (what `state_getMetadata`
    /// returns), or the `Option<OpaqueMetadata>` that `Metadata_metadata_at_version` returns.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let prefixed = match RuntimeMetadataPrefixed::decode(&mut &*bytes) {
            Ok(prefixed) => prefixed,
            Err(err) => match Option::<Vec<u8>>::decode(&mut &*bytes) {
                Ok(Some(inner)) => RuntimeMetadataPrefixed::decode(&mut &*inner)?,
                _ => return Err(err.into()),
            },
        };
        let runtime = match prefixed.1 {
            RuntimeMetadata::V14(md) => Runtime::V14(md),
            RuntimeMetadata::V15(md) => Runtime::V15(md),
            RuntimeMetadata::V16(md) => Runtime::V16(md),
            other => return Err(Error::UnsupportedVersion(other.version())),
        };
        Self::index(runtime)
    }

    fn index(runtime: Runtime) -> Result<Self, Error> {
        let mut items = HashMap::new();
        let mut pallets = HashMap::new();
        with_runtime!(&runtime, md => {
            for (pallet, name) in md.storage_tuples() {
                let info = md
                    .storage_info(&pallet, &name)
                    .map_err(|e| Error::Metadata(e.to_string()))?
                    .into_owned();
                let pallet_hash = twox_128(pallet.as_bytes());
                pallets.entry(pallet_hash).or_insert_with(|| pallet.to_string());
                let mut prefix = [0u8; 32];
                prefix[..16].copy_from_slice(&pallet_hash);
                prefix[16..].copy_from_slice(&twox_128(name.as_bytes()));
                items.insert(prefix, Item { pallet: pallet.to_string(), name: name.to_string(), info });
            }
        });
        let ss58_prefix = with_runtime!(&runtime, md => md.constant_info("System", "SS58Prefix"))
            .ok()
            .and_then(|c| u16::decode(&mut &*c.bytes).ok())
            .unwrap_or(DEFAULT_SS58_PREFIX);
        Ok(Self {
            runtime,
            items,
            pallets,
            ss58_prefix,
        })
    }

    /// Metadata version (14, 15 or 16).
    pub fn version(&self) -> u32 {
        match self.runtime {
            Runtime::V14(_) => 14,
            Runtime::V15(_) => 15,
            Runtime::V16(_) => 16,
        }
    }

    /// The chain's SS58 address prefix.
    pub fn ss58_prefix(&self) -> u16 {
        self.ss58_prefix
    }

    /// Number of storage items in the metadata.
    pub fn storage_item_count(&self) -> usize {
        self.items.len()
    }

    fn types(&self) -> &PortableRegistry {
        with_runtime!(&self.runtime, md => &md.types)
    }

    /// Names a key of the main trie (`child_trie` is `None`) or of a child trie.
    pub fn key_name(&self, child_trie: Option<&[u8]>, key: &[u8]) -> KeyName {
        if let Some(trie) = child_trie {
            return KeyName::Child {
                trie: trie.to_vec(),
            };
        }
        if let Some(name) = well_known(key) {
            return name;
        }
        let Some(prefix) = key.get(..32).and_then(|p| <[u8; 32]>::try_from(p).ok()) else {
            return KeyName::Unknown {
                pallet: self.pallet_of(key),
            };
        };
        let Some(item) = self.items.get(&prefix) else {
            return KeyName::Unknown {
                pallet: self.pallet_of(key),
            };
        };
        let (keys, complete) = self.map_keys(item, key);
        KeyName::Storage {
            pallet: item.pallet.clone(),
            item: item.name.clone(),
            keys,
            complete,
        }
    }

    fn pallet_of(&self, key: &[u8]) -> Option<String> {
        let hash: [u8; 16] = key.get(..16)?.try_into().ok()?;
        self.pallets.get(&hash).cloned()
    }

    /// Renders the map keys after the 32-byte prefix. Transparent hashers (`Identity`,
    /// `Twox64Concat`, `Blake2_128Concat`) give decoded values; opaque ones give a short hash.
    /// `complete` is false when the key is a strict prefix of a full map key, has trailing bytes,
    /// or does not decode.
    fn map_keys(&self, item: &Item, key: &[u8]) -> (Vec<String>, bool) {
        if key.len() == 32 {
            return (Vec::new(), item.info.keys.is_empty());
        }
        let mut cursor = key;
        let Ok(decoded) = decode_storage_key_with_info(&mut cursor, &item.info, self.types())
        else {
            return (Vec::new(), false);
        };
        let keys = decoded
            .parts()
            .map(|part| match part.value() {
                Some(value) => self.render_value(&key[value.range()], *value.ty()),
                None => format!("#{}", short_hex(&key[part.hash_range()])),
            })
            .collect();
        (keys, cursor.is_empty())
    }

    fn render_value(&self, bytes: &[u8], ty: u32) -> String {
        let is_account = self
            .types()
            .resolve(ty)
            .and_then(|t| t.path.segments.last())
            .is_some_and(|name| name == "AccountId32");
        if let (true, Ok(account)) = (is_account, <[u8; 32]>::try_from(bytes)) {
            return ss58::encode(self.ss58_prefix, &account);
        }
        match scale_value::scale::decode_as_type(&mut &*bytes, ty, self.types()) {
            Ok(value) => truncate(value.to_string()),
            Err(_) => format!("0x{}", short_hex(bytes)),
        }
    }

    /// Decodes the pallet and call name of an encoded extrinsic (with its length prefix, as in
    /// a block body).
    pub fn call_name(&self, extrinsic: &[u8]) -> Result<CallName, Error> {
        with_runtime!(&self.runtime, md => {
            let ext = decode_extrinsic(&mut &*extrinsic, md, &md.types)
                .map_err(|e| Error::Extrinsic(e.to_string()))?;
            Ok(CallName { pallet: ext.pallet_name().to_owned(), call: ext.call_name().to_owned() })
        })
    }
}

fn short_hex(bytes: &[u8]) -> String {
    hex::encode(bytes.get(..8).unwrap_or(bytes))
}

fn truncate(mut s: String) -> String {
    if let Some((cut, _)) = s.char_indices().nth(MAX_VALUE_CHARS) {
        s.truncate(cut);
        s.push('…');
    }
    s
}
