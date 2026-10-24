//! Storage operations parsed from `state` events.
//!
//! The events come from `sp_state_machine::Ext` (`substrate/primitives/state-machine/src/ext.rs`).
//! Each one is a `trace!(target: "state", method = "...", ext_id = ..., ...)`, and `sc-tracing`
//! keeps every field as a string. The fields and their formats per `method`:
//!
//! | method                  | fields                                                       |
//! |-------------------------|--------------------------------------------------------------|
//! | `Get`                   | `key`, `result` (`Some(hex)`/`None`), `result_encoded`       |
//! | `Hash`                  | `key`, `result` (Debug of `Option<Vec<u8>>`)                 |
//! | `Exists`                | `key`, `result` (`true`/`false`)                             |
//! | `ChildGet`              | `child_info`, `key`, `result` (`Some(hex)`/`None`)           |
//! | `ChildHash`             | `child_info`, `key`, `result`                                |
//! | `ChildExists`           | `child_info`, `key`, `result`                                |
//! | `Put`                   | `key`, `value` (`Some(hex)`, or `None` for a delete), `value_encoded` |
//! | `ChildPut`              | `child_info`, `key`, `value`                                 |
//! | `ChildKill`             | `child_info`                                                 |
//! | `ClearPrefix`           | `prefix`                                                     |
//! | `ChildClearPrefix`      | `child_info`, `prefix`                                       |
//! | `Append`                | `key`, `value` (the appended item, hex)                      |
//! | `StorageRoot`           | `storage_root`, `cached`                                     |
//! | `ChildStorageRoot`      | `child_info`, `storage_root`, `cached`                       |
//! | `IndexTransaction`      | `index`, `tx_hash`, `size`                                   |
//! | `RenewTransactionIndex` | `index`, `tx_hash`                                           |
//!
//! Every event also carries `ext_id`. `child_info` is the unprefixed child trie storage key.
//!
//! Two hex formats appear. Fields written with `%HexDisplay` (Display: `key`, `prefix`,
//! `child_info`, `Append`'s `value`) are cut to `<first 512 bytes>...<last 512 bytes>` when the
//! input is 1027 bytes or longer. Fields written with `?Option<HexDisplay>` (Debug: `Get`/`Put`
//! `result`/`value`) are never cut. `next_storage_key` is not traced at all, so iteration is
//! invisible. The `Call` event that `execute_aux` emits on the same target (its `method` is the
//! runtime function, for example `Core_execute_block`) is not a storage op and is skipped.

use crate::{Error, rpc::Event};

/// The kind of a storage operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OpKind {
    /// Read a value (`Get`, `ChildGet`).
    Get,
    /// Check that a key exists (`Exists`, `ChildExists`).
    Exists,
    /// Read the hash of a value (`Hash`, `ChildHash`).
    Hash,
    /// Write a value (`Put`/`ChildPut` with `Some`).
    Put,
    /// Delete a key (`Put`/`ChildPut` with `None`).
    Clear,
    /// Append an item to a value (`Append`).
    Append,
    /// Delete every key under a prefix (`ClearPrefix`, `ChildClearPrefix`).
    ClearPrefix,
    /// Delete a whole child trie (`ChildKill`).
    KillChild,
    /// Compute a storage root (`StorageRoot`, `ChildStorageRoot`).
    StorageRoot,
    /// Index a transaction for storage (`IndexTransaction`).
    IndexTransaction,
    /// Renew an indexed transaction (`RenewTransactionIndex`).
    RenewTransactionIndex,
    /// Backend successor lookup (an attempt; overlays may change its result).
    NextKey,
    /// Backend raw iterator advancement.
    Iteration,
}

impl OpKind {
    /// Short operation name, used as `db.operation.name`.
    pub fn as_str(self) -> &'static str {
        match self {
            OpKind::Get => "get",
            OpKind::Exists => "exists",
            OpKind::Hash => "hash",
            OpKind::Put => "put",
            OpKind::Clear => "clear",
            OpKind::Append => "append",
            OpKind::ClearPrefix => "clear_prefix",
            OpKind::KillChild => "kill_child",
            OpKind::StorageRoot => "storage_root",
            OpKind::IndexTransaction => "index_transaction",
            OpKind::RenewTransactionIndex => "renew_transaction_index",
            OpKind::NextKey => "next_key",
            OpKind::Iteration => "iteration",
        }
    }

    /// Whether the op reads state.
    pub fn is_read(self) -> bool {
        matches!(self, OpKind::Get | OpKind::Exists | OpKind::Hash)
    }

    /// Whether the op changes state.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            OpKind::Put | OpKind::Clear | OpKind::Append | OpKind::ClearPrefix | OpKind::KillChild
        )
    }

    /// Whether the op addresses a key (or prefix) in a trie.
    pub fn has_key(self) -> bool {
        self.is_read() || self.is_write() || self == OpKind::NextKey
    }
}

impl std::fmt::Display for OpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One storage operation, in block execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageOp {
    /// Measured local replay interval, absent for legacy JSON.
    pub timing: Option<crate::MeasuredTiming>,
    /// Parent in the observed span tree.
    pub parent_span: Option<u64>,
    /// Position in the block's op list (0-based).
    pub seq: usize,
    /// Index of the source event in `BlockTrace::events`.
    pub event_index: usize,
    /// `ext_id` of the externalities instance (hex of its little-endian id).
    pub ext_id: Option<String>,
    /// What the op did.
    pub kind: OpKind,
    /// Child trie storage key, `None` for the main trie.
    pub child_trie: Option<PrintedBytes>,
    /// The key, or the prefix for [`OpKind::ClearPrefix`]. Empty when the op has no key.
    pub key: PrintedBytes,
    /// Value read, written or appended, when the event carries it in full.
    pub value: Option<Vec<u8>>,
    /// Length of the value read, written or appended. `None` when absent or unknown.
    pub value_len: Option<usize>,
    /// The value was printed truncated, so its length is unknown.
    pub value_truncated: bool,
    /// Set by phase segmentation: the same key was already read earlier in the same phase.
    pub duplicate_read: bool,
}

impl StorageOp {
    /// The `(child trie, key)` pair when both were printed in full. `None` when the node
    /// truncated either, since two different long keys can then look the same.
    pub fn key_id(&self) -> Option<(Option<&[u8]>, &[u8])> {
        let child = match &self.child_trie {
            Some(trie) => Some(trie.complete()?),
            None => None,
        };
        Some((child, self.key.complete()?))
    }

    fn new(kind: OpKind) -> Self {
        Self {
            timing: None,
            parent_span: None,
            seq: 0,
            event_index: 0,
            ext_id: None,
            kind,
            child_trie: None,
            key: PrintedBytes::default(),
            value: None,
            value_len: None,
            value_truncated: false,
            duplicate_read: false,
        }
    }
}

/// Turns the `state` events of a block trace into ordered storage ops. Other events are skipped;
/// the second value is how many were skipped.
pub fn ops_from_events(events: &[Event]) -> Result<(Vec<StorageOp>, usize), Error> {
    let mut ops = Vec::new();
    let mut skipped = 0;
    for (event_index, event) in events.iter().enumerate() {
        match parse_event(event).map_err(|reason| Error::Event {
            event_index,
            reason,
        })? {
            Some(mut op) => {
                op.seq = ops.len();
                op.event_index = event_index;
                ops.push(op);
            }
            None => skipped += 1,
        }
    }
    Ok((ops, skipped))
}

/// Parses one event. `Ok(None)` means it is not a storage op.
fn parse_event(event: &Event) -> Result<Option<StorageOp>, String> {
    if event.target != "state" {
        return Ok(None);
    }
    let Some(method) = event.field("method") else {
        return Ok(None);
    };
    let (kind, child) = match method {
        "Get" => (OpKind::Get, false),
        "ChildGet" => (OpKind::Get, true),
        "Exists" => (OpKind::Exists, false),
        "ChildExists" => (OpKind::Exists, true),
        "Hash" => (OpKind::Hash, false),
        "ChildHash" => (OpKind::Hash, true),
        "Put" => (OpKind::Put, false),
        "ChildPut" => (OpKind::Put, true),
        "Append" => (OpKind::Append, false),
        "ClearPrefix" => (OpKind::ClearPrefix, false),
        "ChildClearPrefix" => (OpKind::ClearPrefix, true),
        "ChildKill" => (OpKind::KillChild, true),
        "StorageRoot" => (OpKind::StorageRoot, false),
        "ChildStorageRoot" => (OpKind::StorageRoot, true),
        "IndexTransaction" => (OpKind::IndexTransaction, false),
        "RenewTransactionIndex" => (OpKind::RenewTransactionIndex, false),
        "BackendNextKey" => (OpKind::NextKey, false),
        "BackendChildNextKey" => (OpKind::NextKey, true),
        "BackendIterNextKey" | "BackendIterNextPair" => (
            OpKind::Iteration,
            event.field("child_info").is_some_and(|v| !v.is_empty()),
        ),
        // `Call` events (runtime entry points) and anything newer than this parser.
        _ => return Ok(None),
    };

    let mut op = StorageOp::new(kind);
    op.ext_id = event.field("ext_id").map(str::to_owned);
    if child {
        op.child_trie = Some(display_hex(required(event, "child_info")?)?);
    }

    match kind {
        OpKind::Get | OpKind::Exists | OpKind::Hash => {
            op.key = display_hex(required(event, "key")?)?;
            if kind == OpKind::Get {
                // `result` is missing only if the node changed format; treat as unknown.
                if let Some(result) = event.field("result") {
                    op.value = option_hex(result)?;
                    op.value_len = op.value.as_ref().map(Vec::len);
                }
            }
        }
        OpKind::Put => {
            op.key = display_hex(required(event, "key")?)?;
            op.value = option_hex(required(event, "value")?)?;
            match &op.value {
                Some(value) => op.value_len = Some(value.len()),
                None => op.kind = OpKind::Clear,
            }
        }
        OpKind::Append => {
            op.key = display_hex(required(event, "key")?)?;
            let value = display_hex(required(event, "value")?)?;
            op.value_truncated = value.is_truncated();
            if let Some(value) = value.complete() {
                op.value_len = Some(value.len());
                op.value = Some(value.to_vec());
            }
        }
        OpKind::ClearPrefix => {
            op.key = display_hex(required(event, "prefix")?)?;
        }
        OpKind::NextKey => {
            op.key = display_hex(required(event, "key")?)?;
        }
        OpKind::Iteration => {}
        OpKind::Clear
        | OpKind::KillChild
        | OpKind::StorageRoot
        | OpKind::IndexTransaction
        | OpKind::RenewTransactionIndex => {}
    }
    Ok(Some(op))
}

fn required<'a>(event: &'a Event, field: &str) -> Result<&'a str, String> {
    event
        .field(field)
        .ok_or_else(|| format!("missing field `{field}`"))
}

/// Bytes as the node printed them with `%HexDisplay`: in full, or, for inputs of 1027 bytes
/// or more, only the first and last 512 bytes (`<head>...<tail>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PrintedBytes {
    /// All bytes, or the first 512 when truncated.
    pub head: Vec<u8>,
    /// The last 512 bytes when the node truncated the value; the middle is lost.
    pub tail: Option<Vec<u8>>,
}

impl PrintedBytes {
    /// Bytes printed in full.
    pub fn full(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            head: bytes.into(),
            tail: None,
        }
    }

    /// Whether the middle of the bytes is missing.
    pub fn is_truncated(&self) -> bool {
        self.tail.is_some()
    }

    /// The bytes, if printed in full.
    pub fn complete(&self) -> Option<&[u8]> {
        self.tail.is_none().then_some(self.head.as_slice())
    }

    /// Whether nothing was printed.
    pub fn is_empty(&self) -> bool {
        self.head.is_empty() && self.tail.is_none()
    }

    /// `0x`-prefixed hex, with `...` where the node cut the middle out.
    pub fn to_hex(&self) -> String {
        match &self.tail {
            Some(tail) => format!("0x{}...{}", hex::encode(&self.head), hex::encode(tail)),
            None => format!("0x{}", hex::encode(&self.head)),
        }
    }
}

/// Parses `%HexDisplay` output (see [`PrintedBytes`]).
fn display_hex(s: &str) -> Result<PrintedBytes, String> {
    match s.split_once("...") {
        Some((head, tail)) => Ok(PrintedBytes {
            head: decode_hex(head)?,
            tail: Some(decode_hex(tail)?),
        }),
        None => Ok(PrintedBytes::full(decode_hex(s)?)),
    }
}

/// Parses `?Option<HexDisplay>` output: `None` or `Some(<hex>)`.
fn option_hex(s: &str) -> Result<Option<Vec<u8>>, String> {
    if s == "None" {
        return Ok(None);
    }
    let inner = s
        .strip_prefix("Some(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| format!("expected `None` or `Some(<hex>)`, got `{}`", preview(s)))?;
    decode_hex(inner).map(Some)
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_err(|e| format!("bad hex `{}`: {e}", preview(s)))
}

fn preview(s: &str) -> &str {
    s.get(..64).unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::Data;

    fn event(fields: &[(&str, &str)]) -> Event {
        Event {
            target: "state".into(),
            data: Data {
                string_values: fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
            parent_id: None,
        }
    }

    #[test]
    fn parses_get_put_and_delete() {
        let get = parse_event(&event(&[
            ("method", "Get"),
            ("ext_id", "0100"),
            ("key", "aabb"),
            ("result", "Some(0102)"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            (get.kind, get.key.complete(), get.value_len),
            (OpKind::Get, Some(&[0xaa, 0xbb][..]), Some(2))
        );
        assert_eq!(get.ext_id.as_deref(), Some("0100"));

        let missing = parse_event(&event(&[
            ("method", "Get"),
            ("key", "aa"),
            ("result", "None"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(missing.value_len, None);

        let put = parse_event(&event(&[
            ("method", "Put"),
            ("key", "aa"),
            ("value", "Some(010203)"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!((put.kind, put.value_len), (OpKind::Put, Some(3)));

        let delete = parse_event(&event(&[
            ("method", "Put"),
            ("key", "aa"),
            ("value", "None"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(delete.kind, OpKind::Clear);
    }

    #[test]
    fn parses_child_ops_and_prefixes() {
        let op = parse_event(&event(&[
            ("method", "ChildGet"),
            ("child_info", "beef"),
            ("key", "01"),
            ("result", "None"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(op.child_trie, Some(PrintedBytes::full([0xbe, 0xef])));

        let op = parse_event(&event(&[("method", "ClearPrefix"), ("prefix", "abcd")]))
            .unwrap()
            .unwrap();
        assert_eq!(
            (op.kind, op.key.complete()),
            (OpKind::ClearPrefix, Some(&[0xab, 0xcd][..]))
        );

        let op = parse_event(&event(&[("method", "ChildKill"), ("child_info", "01")]))
            .unwrap()
            .unwrap();
        assert!(op.kind.is_write() && op.key.is_empty());
    }

    #[test]
    fn handles_truncated_display_hex() {
        let op = parse_event(&event(&[
            ("method", "Append"),
            ("key", "aa"),
            ("value", "0102...0304"),
        ]))
        .unwrap()
        .unwrap();
        assert!(op.value_truncated);
        assert_eq!(op.value_len, None);

        let op = parse_event(&event(&[
            ("method", "ChildGet"),
            ("child_info", "0a...0b"),
            ("key", "aabb...ccdd"),
            ("result", "None"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(op.key.head, [0xaa, 0xbb]);
        assert_eq!(op.key.tail.as_deref(), Some(&[0xcc, 0xdd][..]));
        assert_eq!(op.key.to_hex(), "0xaabb...ccdd");
        assert!(
            op.child_trie
                .as_ref()
                .is_some_and(PrintedBytes::is_truncated)
        );
        assert_eq!(op.key_id(), None);
    }

    #[test]
    fn skips_non_storage_events_and_rejects_bad_hex() {
        let call = event(&[("method", "Core_execute_block"), ("message", "Call")]);
        assert_eq!(parse_event(&call).unwrap(), None);

        let mut other = event(&[("method", "Get")]);
        other.target = "pallet".into();
        assert_eq!(parse_event(&other).unwrap(), None);

        assert!(parse_event(&event(&[("method", "Get"), ("key", "zz")])).is_err());
        assert!(
            parse_event(&event(&[
                ("method", "Put"),
                ("key", "aa"),
                ("value", "0102")
            ]))
            .is_err()
        );
    }
}
