//! Serializable envelope and message names.

use serde::{Deserialize, Serialize};

use crate::{error::ProtocolIoError, PROTOCOL_VERSION};

/// Message direction-independent envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol version used by the sender.
    pub protocol_version: u32,
    /// Correlates a request and its response.
    pub message_id: String,
    /// Semantic message name.
    pub message_type: MessageType,
    /// Optional request identifier for asynchronous responses or events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Message-specific JSON object.
    pub payload: serde_json::Value,
    /// Optional structured error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

/// Supported requests and events in the editor contract.
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
    /// Requests the current asset tree and dependency graph.
    RequestAssetSnapshot,
    /// Requests retained console lines.
    RequestConsoleSnapshot,
    /// Requests retained profiler samples.
    RequestProfilerSnapshot,
    /// Cancels an asset preview job.
    CancelAssetPreview,
    /// Publishes the current world hierarchy.
    WorldSnapshot,
    /// Publishes the current selection.
    SelectionChanged,
    /// Publishes a component change.
    ComponentChanged,
    /// Publishes an asset change.
    AssetChanged,
    /// Returns an asset tree snapshot.
    AssetSnapshot,
    /// Returns retained console lines.
    ConsoleSnapshot,
    /// Returns retained profiler samples.
    ProfilerSnapshot,
    /// Announces a completed preview.
    AssetPreviewReady,
    /// Announces a cancelled preview.
    AssetPreviewCancelled,
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
    /// Announces a project/session lifecycle transition.
    LifecycleStatus,
}

/// Structured protocol error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable diagnostic.
    pub message: String,
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
            correlation_id: None,
            payload,
            error: None,
        }
    }

    /// Creates a handshake request.
    #[must_use]
    pub fn hello(message_id: impl Into<String>, session_token: impl Into<String>) -> Self {
        Self::new(
            message_id,
            MessageType::Hello,
            serde_json::json!({
                "client_name": "hyge-editor-client",
                "supported_protocol_versions": [PROTOCOL_VERSION],
                "session_id": null,
                "session_token": session_token.into()
            }),
        )
    }

    /// Creates a successful handshake response.
    #[must_use]
    pub fn hello_ack(
        message_id: impl Into<String>,
        session_id: impl Into<String>,
        resumed: bool,
        request_timeout_ms: u64,
    ) -> Self {
        Self::new(
            message_id,
            MessageType::HelloAck,
            serde_json::json!({
                "selected_protocol_version": PROTOCOL_VERSION,
                "session_id": session_id.into(),
                "resumed": resumed,
                "server": "hyge-editor",
                "request_timeout_ms": request_timeout_ms
            }),
        )
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
            correlation_id: None,
            payload: serde_json::json!({}),
            error: Some(ProtocolError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ProtocolIoError> {
        self.validate_structure()?;
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolIoError::UnsupportedVersion(self.protocol_version));
        }
        Ok(())
    }

    pub(crate) fn validate_structure(&self) -> Result<(), ProtocolIoError> {
        if self.message_id.is_empty() {
            return Err(ProtocolIoError::InvalidEnvelope(
                "message_id must not be empty",
            ));
        }
        if !self.payload.is_object() {
            return Err(ProtocolIoError::InvalidEnvelope(
                "payload must be an object",
            ));
        }
        if self.correlation_id.as_ref().is_some_and(String::is_empty) {
            return Err(ProtocolIoError::InvalidEnvelope(
                "correlation_id must not be empty",
            ));
        }
        match (&self.message_type, &self.error) {
            (MessageType::EngineError, None) => {
                return Err(ProtocolIoError::InvalidEnvelope(
                    "engine_error requires error",
                ));
            }
            (MessageType::EngineError, Some(error))
                if error.code.is_empty() || error.message.is_empty() =>
            {
                return Err(ProtocolIoError::InvalidEnvelope(
                    "error code and message must not be empty",
                ));
            }
            (_, Some(_)) => {
                return Err(ProtocolIoError::InvalidEnvelope(
                    "only engine_error may contain error",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}
