//! Human-readable names for storage keys.

use std::fmt;

/// Child trie keys longer than this many bytes are shown cut, with `…`.
const CHILD_KEY_SHOWN: usize = 32;

/// What a storage key refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyName {
    /// A FRAME storage item: `twox128(pallet) ++ twox128(item)` followed by map keys.
    Storage {
        /// Pallet name.
        pallet: String,
        /// Storage item name.
        item: String,
        /// Rendered map keys: decoded values for transparent hashers, `#<hash>` otherwise.
        keys: Vec<String>,
        /// False when the key is only a prefix of a full key (for example `clear_prefix` on a map),
        /// was printed truncated, or could not be decoded. Shown as `…`.
        complete: bool,
    },
    /// A well-known key outside of any pallet, such as `:code`.
    WellKnown(String),
    /// A key inside a child trie. Child keys are not described by metadata, so the key is
    /// shown as hex.
    Child {
        /// Child trie storage key (unprefixed).
        trie: Vec<u8>,
        /// The key inside the child trie; empty for ops on the whole trie (`kill_child`).
        key: Vec<u8>,
    },
    /// No storage item with this prefix. `pallet` is set when the first 16 bytes match one.
    Unknown {
        /// Pallet whose `twox128(name)` the key starts with, if any.
        pallet: Option<String>,
    },
}

impl KeyName {
    /// The pallet, used as `db.collection.name`.
    pub fn pallet(&self) -> Option<&str> {
        match self {
            KeyName::Storage { pallet, .. } => Some(pallet),
            KeyName::Unknown { pallet } => pallet.as_deref(),
            _ => None,
        }
    }

    /// Whether the name identifies the key: every map key decoded, or a child key short enough
    /// to show in full.
    pub fn is_complete(&self) -> bool {
        match self {
            KeyName::Storage { complete, .. } => *complete,
            KeyName::WellKnown(_) => true,
            KeyName::Child { key, .. } => key.len() <= CHILD_KEY_SHOWN,
            KeyName::Unknown { .. } => false,
        }
    }

    /// The item: `Pallet::Item`, the well-known key, `child:<id>`, or `unknown`.
    pub fn item(&self) -> String {
        match self {
            KeyName::Storage { pallet, item, .. } => format!("{pallet}::{item}"),
            KeyName::WellKnown(name) => name.clone(),
            KeyName::Child { trie, .. } => {
                format!("child:0x{}", hex::encode(trie.get(..8).unwrap_or(trie)))
            }
            KeyName::Unknown {
                pallet: Some(pallet),
            } => format!("{pallet}::<unknown>"),
            KeyName::Unknown { pallet: None } => "<unknown>".to_owned(),
        }
    }
}

impl fmt::Display for KeyName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.item())?;
        let (keys, complete) = match self {
            KeyName::Storage { keys, complete, .. } => (keys, complete),
            KeyName::Child { key, .. } if key.is_empty() => return Ok(()),
            KeyName::Child { key, .. } => {
                let shown = hex::encode(key.get(..CHILD_KEY_SHOWN).unwrap_or(key));
                let more = if key.len() > CHILD_KEY_SHOWN {
                    "…"
                } else {
                    ""
                };
                return write!(f, "[0x{shown}{more}]");
            }
            _ => return Ok(()),
        };
        match (keys.is_empty(), complete) {
            (true, true) => Ok(()),
            (false, true) => write!(f, "({})", keys.join(", ")),
            // A prefix of a map key, or a key that did not decode in full.
            (true, false) => f.write_str("(…)"),
            (false, false) => write!(f, "({}, …)", keys.join(", ")),
        }
    }
}

/// Names keys defined by `sp_storage::well_known_keys` and FRAME support, which do not start
/// with a pallet prefix.
pub(crate) fn well_known(key: &[u8]) -> Option<KeyName> {
    const EXACT: &[&[u8]] = &[
        b":code",
        b":pending_code",
        b":heappages",
        b":extrinsic_index",
        b":intrablock_entropy",
        b":transaction_level:",
    ];
    if let Some(name) = EXACT.iter().find(|k| **k == key) {
        return Some(KeyName::WellKnown(
            String::from_utf8_lossy(name).into_owned(),
        ));
    }
    // A child trie root as stored in the main trie.
    if let Some(id) = key.strip_prefix(b":child_storage:default:") {
        return Some(KeyName::WellKnown(format!(
            ":child_storage:default:0x{}",
            hex::encode(id)
        )));
    }
    key.starts_with(b":child_storage:")
        .then(|| KeyName::WellKnown(":child_storage:".to_owned()))
}
