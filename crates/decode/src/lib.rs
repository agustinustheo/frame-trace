//! Metadata-based decoding for frame-trace.
//!
//! [`Metadata`] loads V14/V15/V16 runtime metadata (through `frame-metadata`) and names raw
//! storage keys as `Pallet::Item(map keys…)` using `frame-decode`. It also names extrinsics.
//! [`annotate`] applies it to a whole [`frame_trace_model::TracedBlock`].

mod annotate;
mod key;
mod metadata;
pub mod ss58;

pub use annotate::{
    AnnotatedBlock, AnnotatedCall, AnnotatedOp, AnnotatedPhase, ExecutionInfo, PhaseTiming,
    annotate, annotate_at_block,
};
pub use key::KeyName;
pub use metadata::{CallName, Metadata};

/// Errors from loading metadata or decoding.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bytes are not SCALE-encoded runtime metadata.
    #[error("invalid metadata: {0}")]
    Codec(#[from] parity_scale_codec::Error),
    /// The metadata text is not valid hex.
    #[error("metadata is not valid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    /// The metadata version is not V14, V15 or V16.
    #[error("unsupported metadata version V{0} (need V14, V15 or V16)")]
    UnsupportedVersion(u32),
    /// The metadata is inconsistent.
    #[error("invalid metadata: {0}")]
    Metadata(String),
    /// Runtime API lookup, user arguments or result decoding.
    #[error("{0}")]
    RuntimeApi(String),
    /// The extrinsic could not be decoded.
    #[error("cannot decode extrinsic: {0}")]
    Extrinsic(String),
}
