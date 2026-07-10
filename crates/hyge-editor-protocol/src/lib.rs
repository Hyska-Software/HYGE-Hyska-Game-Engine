//! Versioned wire contract for the Hyge editor.
//!
//! The protocol deliberately contains no ECS or renderer types. The Rust
//! service translates engine state into these stable JSON messages, while a
//! Qt/PySide frontend consumes them without linking to the engine ABI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// Current wire protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum accepted JSON message size (16 MiB).
pub const MAX_MESSAGE_BYTES: u32 = 16 * 1024 * 1024;

/// Message direction-independent envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol version used by the sender.
    pub protocol_version: u32,
    /// Correlates a request and its response.
    pub message_id: String,
    /// Semantic message name.
    pub message_type: MessageType,
    /// Message-specific JSON payload.
    #[serde(default)]
    pub payload: serde_json::Value,
    /// Optional structured error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

/// Supported requests and events in the initial editor contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// Establishes a session and negotiates the protocol.
    Hello,
    /// A successful handshake response.
    HelloAck,
    /// Opens a project directory.
    OpenProject,
    /// Opens a cooked world scene.
    OpenScene,
    /// Saves the active scene.
    SaveScene,
    /// Updates the editor selection.
    SelectEntities,
    /// Changes a reflected component field.
    EditComponent,
    /// Adds a component.
    AddComponent,
    /// Removes a component.
    RemoveComponent,
    /// Changes an entity parent.
    ReparentEntity,
    /// Duplicates an entity.
    DuplicateEntity,
    /// Destroys an entity.
    DestroyEntity,
    /// Instantiates a prefab.
    InstantiatePrefab,
    /// Undoes the latest command.
    Undo,
    /// Redoes the latest command.
    Redo,
    /// Updates editor camera state.
    SetEditorCamera,
    /// Changes viewport dimensions.
    SetViewportSize,
    /// Requests an asset preview.
    RequestAssetPreview,
    /// Publishes the current world hierarchy.
    WorldSnapshot,
    /// Publishes the current selection.
    SelectionChanged,
    /// Publishes a component change.
    ComponentChanged,
    /// Publishes an asset change.
    AssetChanged,
    /// Publishes a scene reload.
    SceneReloaded,
    /// Publishes a console line.
    ConsoleLine,
    /// Publishes profiler data.
    ProfilerSample,
    /// Announces a new viewport frame in shared memory.
    ViewportFrameAvailable,
    /// Completes a command.
    CommandCompleted,
    /// Publishes a service error.
    EngineError,
    /// Announces service shutdown.
    ServerShutdown,
}

/// Structured protocol error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable diagnostic.
    pub message: String,
}

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
}

impl Envelope {
    /// Creates an envelope for the current protocol version.
    #[must_use]
    pub fn new(
        message_id: impl Into<String>,
        message_type: MessageType,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message_id: message_id.into(),
            message_type,
            payload,
            error: None,
        }
    }

    /// Creates a protocol error response while preserving the request id.
    #[must_use]
    pub fn error(
        message_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message_id: message_id.into(),
            message_type: MessageType::EngineError,
            payload: serde_json::Value::Null,
            error: Some(ProtocolError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/// Writes one length-prefixed envelope to a stream.
pub fn write_envelope<W: Write>(
    writer: &mut W,
    envelope: &Envelope,
) -> Result<(), ProtocolIoError> {
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolIoError::UnsupportedVersion(
            envelope.protocol_version,
        ));
    }
    let body = serde_json::to_vec(envelope)?;
    let length = u32::try_from(body.len()).map_err(|_| ProtocolIoError::TooLarge(u32::MAX))?;
    if length > MAX_MESSAGE_BYTES {
        return Err(ProtocolIoError::TooLarge(length));
    }
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Reads one length-prefixed envelope from a stream.
pub fn read_envelope<R: Read>(reader: &mut R) -> Result<Envelope, ProtocolIoError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header);
    if length > MAX_MESSAGE_BYTES {
        return Err(ProtocolIoError::TooLarge(length));
    }
    let mut body = vec![0_u8; length as usize];
    reader.read_exact(&mut body)?;
    let envelope: Envelope = serde_json::from_slice(&body)?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolIoError::UnsupportedVersion(
            envelope.protocol_version,
        ));
    }
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn round_trips_length_prefixed_envelope() {
        let envelope = Envelope::new(
            "42",
            MessageType::Hello,
            serde_json::json!({"client": "pytest"}),
        );
        let mut bytes = Vec::new();
        write_envelope(&mut bytes, &envelope).expect("write must succeed");
        let decoded = read_envelope(&mut Cursor::new(bytes)).expect("read must succeed");
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn rejects_wrong_protocol_version() {
        let mut envelope = Envelope::new("1", MessageType::Hello, serde_json::Value::Null);
        envelope.protocol_version = PROTOCOL_VERSION + 1;
        let error = write_envelope(&mut Vec::new(), &envelope).expect_err("version must fail");
        assert!(matches!(error, ProtocolIoError::UnsupportedVersion(2)));
    }

    #[test]
    fn error_response_is_machine_readable() {
        let envelope = Envelope::error("7", "invalid_request", "bad payload");
        assert_eq!(
            envelope.error.as_ref().expect("error exists").code,
            "invalid_request"
        );
        assert_eq!(envelope.message_type, MessageType::EngineError);
    }
}
