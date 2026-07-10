//! Authenticated loopback TCP server for the editor protocol.

use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use hyge_editor_protocol::{read_envelope, write_envelope, Envelope, MessageType, ProtocolIoError};

use crate::{auth::ConnectionAuth, state::EditorState};

/// Configuration for the local editor service.
#[derive(Clone, Debug)]
pub struct EditorServerConfig {
    /// Address to bind. Only IPv4 loopback addresses are accepted.
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

/// Authenticated TCP editor service.
pub struct EditorServer {
    listener: TcpListener,
    config: EditorServerConfig,
    state: Arc<Mutex<EditorState>>,
    shutdown: Arc<AtomicBool>,
}

impl EditorServer {
    /// Binds a local editor service.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the address is invalid, non-loopback, or
    /// cannot be bound.
    pub fn bind(config: EditorServerConfig) -> io::Result<Self> {
        let address = config.bind_address.parse::<SocketAddr>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid bind address: {error}"),
            )
        })?;
        if address.ip() != IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "editor service must bind to IPv4 loopback 127.0.0.1",
            ));
        }
        if config.session_token.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "editor session token must not be empty",
            ));
        }
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            config,
            state: Arc::new(Mutex::new(EditorState::default())),
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns the actual bound address, useful when port zero was requested.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Requests the accept loop to stop after active connections finish.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Serves connections until shutdown is requested.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for an accept failure other than the temporary
    /// non-blocking state.
    pub fn run(&self) -> io::Result<()> {
        while !self.shutdown.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    let config = self.config.clone();
                    let state = Arc::clone(&self.state);
                    let shutdown = Arc::clone(&self.shutdown);
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, &config, state, shutdown) {
                            tracing::warn!(%error, "editor client disconnected with error");
                        }
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Handles one envelope as an unauthenticated connection.
    ///
    /// This helper is intended for deterministic unit tests. Network clients
    /// use the per-connection authentication state in [`Self::run`].
    pub fn handle(&self, envelope: &Envelope) -> Envelope {
        let mut auth = ConnectionAuth::default();
        handle_envelope(
            envelope,
            &self.config,
            &self.state,
            &self.shutdown,
            &mut auth,
        )
    }
}

fn handle_connection(
    mut stream: TcpStream,
    config: &EditorServerConfig,
    state: Arc<Mutex<EditorState>>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), ProtocolIoError> {
    let mut auth = ConnectionAuth::default();
    loop {
        let request = read_envelope(&mut stream)?;
        let response = handle_envelope(&request, config, &state, &shutdown, &mut auth);
        write_envelope(&mut stream, &response)?;
        if response.message_type == MessageType::ServerShutdown || shutdown.load(Ordering::Acquire)
        {
            return Ok(());
        }
    }
}

fn handle_envelope(
    envelope: &Envelope,
    config: &EditorServerConfig,
    state: &Arc<Mutex<EditorState>>,
    shutdown: &Arc<AtomicBool>,
    auth: &mut ConnectionAuth,
) -> Envelope {
    if envelope.message_type == MessageType::Hello {
        if !auth.authenticate(envelope, &config.session_token) {
            return Envelope::error(
                &envelope.message_id,
                "unauthorized",
                "invalid editor session token",
            );
        }
        return Envelope::hello_ack(&envelope.message_id);
    }

    if !auth.is_authenticated() {
        return Envelope::error(
            &envelope.message_id,
            "unauthorized",
            "editor handshake is required before requests",
        );
    }

    match envelope.message_type {
        MessageType::OpenProject => update_project(envelope, state),
        MessageType::OpenScene => update_scene(envelope, state),
        MessageType::ServerShutdown => {
            shutdown.store(true, Ordering::Release);
            Envelope::new(
                &envelope.message_id,
                MessageType::ServerShutdown,
                serde_json::json!({}),
            )
        }
        MessageType::Hello
        | MessageType::HelloAck
        | MessageType::WorldSnapshot
        | MessageType::SelectionChanged
        | MessageType::ComponentChanged
        | MessageType::AssetChanged
        | MessageType::SceneReloaded
        | MessageType::ConsoleLine
        | MessageType::ProfilerSample
        | MessageType::ViewportFrameAvailable
        | MessageType::CommandCompleted
        | MessageType::EngineError => Envelope::error(
            &envelope.message_id,
            "unsupported_request",
            "message is not an implemented client request",
        ),
        _ => Envelope::error(
            &envelope.message_id,
            "unsupported_request",
            "editor command is reserved for a later editor milestone",
        ),
    }
}

fn update_project(envelope: &Envelope, state: &Arc<Mutex<EditorState>>) -> Envelope {
    let Some(path) = envelope
        .payload
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "open_project requires path",
        );
    };
    let Ok(mut state) = state.lock() else {
        return Envelope::error(
            &envelope.message_id,
            "state_poisoned",
            "editor state is unavailable",
        );
    };
    state.project = Some(path.to_owned());
    Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({"command": "open_project", "recorded": true}),
    )
}

fn update_scene(envelope: &Envelope, state: &Arc<Mutex<EditorState>>) -> Envelope {
    let Some(path) = envelope
        .payload
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "open_scene requires path",
        );
    };
    let Ok(mut state) = state.lock() else {
        return Envelope::error(
            &envelope.message_id,
            "state_poisoned",
            "editor state is unavailable",
        );
    };
    state.scene = Some(path.to_owned());
    Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({"command": "open_scene", "recorded": true}),
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    use super::*;

    fn server() -> EditorServer {
        EditorServer::bind(EditorServerConfig::default()).expect("bind")
    }

    #[test]
    fn hello_requires_session_token() {
        let server = server();
        let response = server.handle(&Envelope::hello("1", "wrong"));
        assert_eq!(response.error.expect("error").code, "unauthorized");
    }

    #[test]
    fn hello_acknowledges_valid_client() {
        let server = server();
        let response = server.handle(&Envelope::hello("1", "hyge-local-dev"));
        assert_eq!(response.message_type, MessageType::HelloAck);
    }

    #[test]
    fn direct_requests_require_handshake() {
        let server = server();
        let response = server.handle(&Envelope::new(
            "2",
            MessageType::OpenProject,
            serde_json::json!({"path": "."}),
        ));
        assert_eq!(response.error.expect("error").code, "unauthorized");
    }

    #[test]
    fn reserved_commands_are_not_reported_as_accepted() {
        let server = server();
        let _ = server.handle(&Envelope::hello("1", "hyge-local-dev"));
        let response = server.handle(&Envelope::new(
            "2",
            MessageType::Undo,
            serde_json::json!({}),
        ));
        assert_eq!(response.error.expect("error").code, "unauthorized");
    }

    #[test]
    fn rejects_non_loopback_bind() {
        let result = EditorServer::bind(EditorServerConfig {
            bind_address: "0.0.0.0:0".into(),
            ..EditorServerConfig::default()
        });
        assert!(result.is_err());
    }

    #[test]
    fn tcp_connection_round_trip_preserves_message_id_and_authenticates() {
        let server = server();
        let address = server.local_addr().expect("address");
        let shutdown = Arc::clone(&server.shutdown);
        let thread = thread::spawn(move || server.run());

        let mut stream = TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let hello = Envelope::hello("hello-id", "hyge-local-dev");
        write_envelope(&mut stream, &hello).expect("write hello");
        let response = read_envelope(&mut stream).expect("read hello");
        assert_eq!(response.message_type, MessageType::HelloAck);
        assert_eq!(response.message_id, "hello-id");

        let open = Envelope::new(
            "project-id",
            MessageType::OpenProject,
            serde_json::json!({"path": "."}),
        );
        write_envelope(&mut stream, &open).expect("write project");
        let response = read_envelope(&mut stream).expect("read project");
        assert_eq!(response.message_id, "project-id");

        let shutdown_request = Envelope::new(
            "shutdown-id",
            MessageType::ServerShutdown,
            serde_json::json!({}),
        );
        write_envelope(&mut stream, &shutdown_request).expect("write shutdown");
        let _ = read_envelope(&mut stream).expect("read shutdown");
        shutdown.store(true, Ordering::Release);
        thread.join().expect("server thread").expect("server run");
    }

    #[allow(dead_code)]
    fn _stream_traits_are_available(stream: &mut TcpStream) {
        let _ = stream.write(&[]);
        let _ = stream.read(&mut []);
    }
}
