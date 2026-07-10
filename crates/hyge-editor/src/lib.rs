//! Rust editor service for the Hyge Qt frontend.
//!
//! The service owns editor mutations and emits stable protocol snapshots.
//! The PySide6 process is a presentation client; it never receives a direct
//! pointer or ABI handle to the ECS world.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use hyge_editor_protocol::{
    read_envelope, write_envelope, Envelope, MessageType, ProtocolIoError, PROTOCOL_VERSION,
};

/// Configuration for the local editor service.
#[derive(Clone, Debug)]
pub struct EditorServerConfig {
    /// Address to bind. Loopback is the safe default.
    pub bind_address: String,
    /// Session token expected by the first hello payload.
    pub session_token: String,
}

impl Default for EditorServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:0".into(),
            session_token: "hyge-local-dev".into(),
        }
    }
}

/// Mutable state owned by the service between requests.
#[derive(Clone, Debug, Default)]
pub struct EditorState {
    /// Currently selected entity ids represented as strings at the wire boundary.
    pub selected_entities: Vec<String>,
    /// Last opened project path.
    pub project: Option<String>,
    /// Last opened scene path.
    pub scene: Option<String>,
}

/// TCP editor service.
pub struct EditorServer {
    listener: TcpListener,
    config: EditorServerConfig,
    state: Arc<Mutex<EditorState>>,
}

impl EditorServer {
    /// Binds a local editor service.
    pub fn bind(config: EditorServerConfig) -> io::Result<Self> {
        let listener = TcpListener::bind(&config.bind_address)?;
        Ok(Self {
            listener,
            config,
            state: Arc::new(Mutex::new(EditorState::default())),
        })
    }

    /// Returns the actual bound address, useful when port zero was requested.
    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// Serves connections until the listener is dropped.
    pub fn run(&self) -> io::Result<()> {
        for stream in self.listener.incoming() {
            let stream = stream?;
            let config = self.config.clone();
            let state = Arc::clone(&self.state);
            thread::spawn(move || {
                if let Err(error) = handle_connection(stream, &config, state) {
                    tracing::warn!(%error, "editor client disconnected with error");
                }
            });
        }
        Ok(())
    }

    /// Handles one request without requiring a network connection.
    pub fn handle(&self, envelope: &Envelope) -> Envelope {
        handle_envelope(envelope, &self.config, &self.state)
    }
}

fn handle_connection(
    mut stream: TcpStream,
    config: &EditorServerConfig,
    state: Arc<Mutex<EditorState>>,
) -> Result<(), ProtocolIoError> {
    loop {
        let request = read_envelope(&mut stream)?;
        let response = handle_envelope(&request, config, &state);
        write_envelope(&mut stream, &response)?;
        if response.message_type == MessageType::ServerShutdown {
            return Ok(());
        }
    }
}

fn handle_envelope(
    envelope: &Envelope,
    config: &EditorServerConfig,
    state: &Arc<Mutex<EditorState>>,
) -> Envelope {
    if envelope.message_type == MessageType::Hello {
        let token = envelope
            .payload
            .get("session_token")
            .and_then(serde_json::Value::as_str);
        if token != Some(config.session_token.as_str()) {
            return Envelope::error(
                &envelope.message_id,
                "unauthorized",
                "invalid editor session token",
            );
        }
        return Envelope::new(
            &envelope.message_id,
            MessageType::HelloAck,
            serde_json::json!({"protocol_version": PROTOCOL_VERSION, "server": "hyge-editor"}),
        );
    }

    let mut state = match state.lock() {
        Ok(state) => state,
        Err(_) => {
            return Envelope::error(
                &envelope.message_id,
                "state_poisoned",
                "editor state lock is unavailable",
            )
        }
    };
    match envelope.message_type {
        MessageType::OpenProject => {
            state.project = envelope
                .payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            Envelope::new(
                &envelope.message_id,
                MessageType::CommandCompleted,
                serde_json::json!({"command": "open_project"}),
            )
        }
        MessageType::OpenScene => {
            state.scene = envelope
                .payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            Envelope::new(
                &envelope.message_id,
                MessageType::WorldSnapshot,
                serde_json::json!({"entities": [], "scene": state.scene}),
            )
        }
        MessageType::SelectEntities => {
            state.selected_entities = envelope
                .payload
                .get("entities")
                .and_then(serde_json::Value::as_array)
                .map(|entities| {
                    entities
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Envelope::new(
                &envelope.message_id,
                MessageType::SelectionChanged,
                serde_json::json!({"entities": state.selected_entities}),
            )
        }
        MessageType::SaveScene
        | MessageType::EditComponent
        | MessageType::AddComponent
        | MessageType::RemoveComponent
        | MessageType::ReparentEntity
        | MessageType::DuplicateEntity
        | MessageType::DestroyEntity
        | MessageType::InstantiatePrefab
        | MessageType::Undo
        | MessageType::Redo
        | MessageType::SetEditorCamera
        | MessageType::SetViewportSize
        | MessageType::RequestAssetPreview => Envelope::new(
            &envelope.message_id,
            MessageType::CommandCompleted,
            serde_json::json!({"command": envelope.message_type, "accepted": true}),
        ),
        _ => Envelope::error(
            &envelope.message_id,
            "unsupported_request",
            "message is not a client request",
        ),
    }
}

/// Common editor exports.
pub mod prelude {
    pub use crate::{EditorServer, EditorServerConfig, EditorState};
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyge_editor_protocol::MessageType;

    #[test]
    fn hello_requires_session_token() {
        let server = EditorServer::bind(EditorServerConfig::default()).expect("bind");
        let request = Envelope::new(
            "1",
            MessageType::Hello,
            serde_json::json!({"session_token": "wrong"}),
        );
        let response = server.handle(&request);
        assert_eq!(response.error.expect("error").code, "unauthorized");
    }

    #[test]
    fn hello_acknowledges_valid_client() {
        let server = EditorServer::bind(EditorServerConfig::default()).expect("bind");
        let request = Envelope::new(
            "1",
            MessageType::Hello,
            serde_json::json!({"session_token": "hyge-local-dev"}),
        );
        let response = server.handle(&request);
        assert_eq!(response.message_type, MessageType::HelloAck);
    }

    #[test]
    fn selection_updates_service_state() {
        let server = EditorServer::bind(EditorServerConfig::default()).expect("bind");
        let request = Envelope::new(
            "2",
            MessageType::SelectEntities,
            serde_json::json!({"entities": ["1", "2"]}),
        );
        let response = server.handle(&request);
        assert_eq!(response.message_type, MessageType::SelectionChanged);
        assert_eq!(response.payload["entities"], serde_json::json!(["1", "2"]));
    }
}
