//! Errors produced by the editor wire protocol.

use std::io;

/// Errors produced while encoding or decoding the wire format.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolIoError {
    /// Underlying socket or stream failure.
    #[error("protocol I/O failed: {0}")]
    Io(#[from] io::Error),
    /// JSON serialization failure.
    #[error("protocol JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Message exceeds the configured maximum.
    #[error("protocol message is too large: {0} bytes")]
    TooLarge(u32),
    /// Sender and receiver use incompatible versions.
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u32),
    /// Envelope fields do not satisfy the wire contract.
    #[error("invalid protocol envelope: {0}")]
    InvalidEnvelope(&'static str),
    /// A socket operation exceeded the configured request timeout.
    #[error("protocol request timed out")]
    Timeout,
}
