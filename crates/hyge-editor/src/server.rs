//! Authenticated, version-negotiated loopback TCP server for the editor.

use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use hyge_core::result::HygeError;
use hyge_editor_protocol::{
    read_frame, write_frame, Envelope, MessageType, ProtocolIoError, PROTOCOL_VERSION,
};

use crate::{
    auth::ConnectionAuth,
    lifecycle::{LifecycleSnapshot, LifecycleState},
    state::{SessionError, SessionRegistry, SessionSnapshot},
};

/// Configuration for the local editor service.
#[derive(Clone, Debug)]
pub struct EditorServerConfig {
    /// Address to bind. Only IPv4 loopback addresses are accepted.
    pub bind_address: String,
    /// Session token expected by the handshake.
    pub session_token: String,
    /// Maximum idle time for a request socket.
    pub request_timeout: Duration,
    /// How long disconnected sessions remain resumable.
    pub session_ttl: Duration,
}

impl Default for EditorServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:0".into(),
            session_token: "hyge-local-dev".into(),
            request_timeout: Duration::from_secs(5),
            session_ttl: Duration::from_secs(300),
        }
    }
}

/// Authenticated TCP editor service.
pub struct EditorServer {
    listener: TcpListener,
    config: EditorServerConfig,
    sessions: Arc<Mutex<SessionRegistry>>,
    shutdown: Arc<AtomicBool>,
    frontend: Arc<Mutex<Option<std::process::Child>>>,
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
        if config.request_timeout.is_zero() || config.session_ttl.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "editor timeouts must be greater than zero",
            ));
        }
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            config,
            sessions: Arc::new(Mutex::new(SessionRegistry::default())),
            shutdown: Arc::new(AtomicBool::new(false)),
            frontend: Arc::new(Mutex::new(None)),
        })
    }

    /// Transfers ownership of an optional frontend child to the server.
    pub fn attach_frontend(&self, child: std::process::Child) -> io::Result<()> {
        self.frontend
            .lock()
            .map_err(|_| io::Error::other("frontend ownership lock poisoned"))?
            .replace(child);
        Ok(())
    }

    /// Returns the actual bound address, useful when port zero was requested.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Requests the accept loop to stop after active connections finish.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Returns a retained session snapshot.
    pub fn session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.sessions.lock().ok()?.snapshot(session_id)
    }

    /// Serves connections until shutdown is requested.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for an accept failure other than the temporary
    /// non-blocking state.
    pub fn run(&self) -> io::Result<()> {
        let result = loop {
            if self.shutdown.load(Ordering::Acquire) {
                break Ok(());
            }
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.expire(self.config.session_ttl);
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    let config = self.config.clone();
                    let sessions = Arc::clone(&self.sessions);
                    let shutdown = Arc::clone(&self.shutdown);
                    thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, &config, sessions, shutdown) {
                            if !is_expected_disconnect(&error) {
                                tracing::warn!(%error, "editor client disconnected with error");
                            }
                        }
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => break Err(error),
            }
        };
        self.cleanup_frontend();
        result
    }

    fn cleanup_frontend(&self) {
        let child = self.frontend.lock().ok().and_then(|mut child| child.take());
        let Some(mut child) = child else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }

    /// Handles one envelope as an unauthenticated connection.
    ///
    /// This helper is intended for deterministic unit tests. Network clients
    /// use the per-connection authentication state in [`Self::run`].
    pub fn handle(&self, envelope: &Envelope) -> Envelope {
        let mut auth = ConnectionAuth::default();
        if envelope.message_type == MessageType::Hello {
            return process_hello(envelope, &self.config, &self.sessions, &mut auth);
        }
        Envelope::error(
            &envelope.message_id,
            "unauthorized",
            "editor handshake is required before requests",
        )
    }
}

impl Drop for EditorServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.cleanup_frontend();
    }
}

fn handle_connection(
    mut stream: TcpStream,
    config: &EditorServerConfig,
    sessions: Arc<Mutex<SessionRegistry>>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), ProtocolIoError> {
    stream.set_read_timeout(Some(config.request_timeout))?;
    stream.set_write_timeout(Some(config.request_timeout))?;
    let mut auth = ConnectionAuth::default();
    let hello = read_frame(&mut stream).map_err(map_timeout)?;
    let response = process_hello(&hello, config, &sessions, &mut auth);
    write_frame(&mut stream, &response)?;
    if response.message_type == MessageType::EngineError {
        return Ok(());
    }

    loop {
        let request = match read_frame(&mut stream) {
            Ok(request) => request,
            Err(error) if is_timeout(&error) => {
                disconnect_auth(&sessions, &auth);
                return Err(ProtocolIoError::Timeout);
            }
            Err(error) => {
                disconnect_auth(&sessions, &auth);
                return Err(error);
            }
        };
        if request.protocol_version != PROTOCOL_VERSION {
            let response = Envelope::error(
                &request.message_id,
                "incompatible_version",
                "protocol version is not the negotiated version",
            );
            write_frame(&mut stream, &response)?;
            disconnect_auth(&sessions, &auth);
            return Ok(());
        }
        if !auth.mark_message_id(&request.message_id) {
            let response = Envelope::error(
                &request.message_id,
                "request_id_conflict",
                "message_id was already used on this connection",
            );
            write_frame(&mut stream, &response)?;
            continue;
        }
        let responses = handle_authenticated(&request, config, &sessions, &shutdown, &auth);
        for response in &responses {
            write_frame(&mut stream, response)?;
        }
        if responses
            .iter()
            .any(|response| response.message_type == MessageType::ServerShutdown)
            || shutdown.load(Ordering::Acquire)
        {
            disconnect_auth(&sessions, &auth);
            return Ok(());
        }
        if response
            .error
            .as_ref()
            .is_some_and(|error| error.code == "session_replaced")
        {
            disconnect_auth(&sessions, &auth);
            return Ok(());
        }
    }
}

fn disconnect_auth(sessions: &Arc<Mutex<SessionRegistry>>, auth: &ConnectionAuth) {
    if let Some(binding) = auth.binding.as_ref() {
        if let Ok(mut registry) = sessions.lock() {
            registry.disconnect(binding);
        }
    }
}

fn process_hello(
    envelope: &Envelope,
    config: &EditorServerConfig,
    sessions: &Arc<Mutex<SessionRegistry>>,
    auth: &mut ConnectionAuth,
) -> Envelope {
    if envelope.message_type != MessageType::Hello {
        return Envelope::error(
            &envelope.message_id,
            "unauthorized",
            "hello must be the first message",
        );
    }
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Envelope::error(
            &envelope.message_id,
            "incompatible_version",
            "protocol version is not supported by this server",
        );
    }
    if !auth.mark_message_id(&envelope.message_id) {
        return Envelope::error(
            &envelope.message_id,
            "request_id_conflict",
            "message_id was already used on this connection",
        );
    }
    if !auth.authenticate(envelope, &config.session_token) {
        return Envelope::error(
            &envelope.message_id,
            "unauthorized",
            "invalid editor session token",
        );
    }
    let Some(versions) = envelope
        .payload
        .get("supported_protocol_versions")
        .and_then(serde_json::Value::as_array)
    else {
        return Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "hello requires supported_protocol_versions",
        );
    };
    if versions.is_empty()
        || !versions.iter().all(serde_json::Value::is_number)
        || envelope
            .payload
            .get("client_name")
            .and_then(serde_json::Value::as_str)
            .map_or(true, str::is_empty)
    {
        return Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "hello contains an invalid client identity or version list",
        );
    }
    let compatible = versions
        .iter()
        .any(|version| version.as_u64() == Some(u64::from(PROTOCOL_VERSION)));
    if !compatible {
        return Envelope::error(
            &envelope.message_id,
            "incompatible_version",
            "client and server have no compatible protocol version",
        );
    }
    let requested_session = match envelope.payload.get("session_id") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => match value.as_str() {
            Some(value) => Some(value),
            None => {
                return Envelope::error(
                    &envelope.message_id,
                    "invalid_request",
                    "session_id must be a string or null",
                )
            }
        },
    };
    let result = sessions
        .lock()
        .map_err(|_| SessionError::NotFound)
        .and_then(|mut registry| registry.bind(requested_session, config.session_ttl));
    let (binding, resumed) = match result {
        Ok(result) => result,
        Err(error) => {
            return Envelope::error(
                &envelope.message_id,
                session_error_code(error),
                session_error_message(error),
            )
        }
    };
    auth.binding = Some(binding.clone());
    Envelope::hello_ack(
        &envelope.message_id,
        binding.session_id,
        resumed,
        config
            .request_timeout
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
    )
}

fn handle_authenticated(
    envelope: &Envelope,
    config: &EditorServerConfig,
    sessions: &Arc<Mutex<SessionRegistry>>,
    shutdown: &Arc<AtomicBool>,
    auth: &ConnectionAuth,
) -> Vec<Envelope> {
    let Some(binding) = auth.binding.as_ref() else {
        return vec![Envelope::error(
            &envelope.message_id,
            "unauthorized",
            "editor handshake is required before requests",
        )];
    };
    let runtime = {
        let Ok(mut registry) = sessions.lock() else {
            return vec![Envelope::error(
                &envelope.message_id,
                "session_unavailable",
                "session registry is unavailable",
            )];
        };
        if let Err(error) = registry.touch(binding) {
            return vec![Envelope::error(
                &envelope.message_id,
                session_error_code(error),
                session_error_message(error),
            )];
        }
        match registry.runtime(binding) {
            Ok(runtime) => runtime,
            Err(error) => {
                return vec![Envelope::error(
                    &envelope.message_id,
                    session_error_code(error),
                    session_error_message(error),
                )]
            }
        }
    };
    match envelope.message_type {
        MessageType::OpenProject => lifecycle_open_project(envelope, sessions, binding, runtime),
        MessageType::OpenScene => lifecycle_open_scene(envelope, sessions, binding, runtime),
        MessageType::SaveScene => lifecycle_save_scene(envelope, runtime, &binding.session_id),
        MessageType::ServerShutdown => {
            shutdown.store(true, Ordering::Release);
            vec![Envelope::new(
                &envelope.message_id,
                MessageType::ServerShutdown,
                serde_json::json!({"session_id": binding.session_id, "released": true}),
            )]
        }
        _ => {
            let _ = config;
            vec![Envelope::error(
                &envelope.message_id,
                "unsupported_request",
                "editor command is reserved for a later editor milestone",
            )]
        }
    }
}

fn lifecycle_open_project(
    envelope: &Envelope,
    sessions: &Arc<Mutex<SessionRegistry>>,
    binding: &crate::state::SessionBinding,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let Some(path) = envelope
        .payload
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "open_project requires path",
        )];
    };
    let mut responses = vec![lifecycle_status(
        &envelope.message_id,
        &binding.session_id,
        LifecycleState::Loading,
        None,
    )];
    let result = runtime
        .lock()
        .map_err(|_| HygeError::invalid_argument("runtime lock poisoned"))
        .and_then(|mut runtime| runtime.open_project(Path::new(path)));
    match result {
        Ok(snapshot) => {
            if let Ok(mut registry) = sessions.lock() {
                if let Some(canonical) = snapshot.project.as_ref() {
                    let _ = registry.update_project(binding, canonical.display().to_string());
                }
            }
            responses.push(lifecycle_status(
                &envelope.message_id,
                &binding.session_id,
                snapshot.state.clone(),
                Some(&snapshot),
            ));
            responses.push(command_completed(envelope, "open_project", &snapshot));
        }
        Err(error) => {
            if let Ok(mut runtime) = runtime.lock() {
                runtime.fail(error.to_string());
            }
            responses.push(lifecycle_status(
                &envelope.message_id,
                &binding.session_id,
                LifecycleState::Failed,
                None,
            ));
            responses.push(Envelope::error(
                &envelope.message_id,
                "project_open_failed",
                error.to_string(),
            ));
        }
    }
    responses
}

fn lifecycle_open_scene(
    envelope: &Envelope,
    sessions: &Arc<Mutex<SessionRegistry>>,
    binding: &crate::state::SessionBinding,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let Some(path) = envelope
        .payload
        .get("path")
        .and_then(serde_json::Value::as_str)
    else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "open_scene requires path",
        )];
    };
    let mut responses = vec![lifecycle_status(
        &envelope.message_id,
        &binding.session_id,
        LifecycleState::Loading,
        None,
    )];
    let result = runtime
        .lock()
        .map_err(|_| HygeError::invalid_argument("runtime lock poisoned"))
        .and_then(|mut runtime| runtime.open_scene(Path::new(path)));
    match result {
        Ok(snapshot) => {
            if let Ok(mut registry) = sessions.lock() {
                if let Some(canonical) = snapshot.scene.as_ref() {
                    let _ = registry.update_scene(binding, canonical.display().to_string());
                }
            }
            responses.push(lifecycle_status(
                &envelope.message_id,
                &binding.session_id,
                snapshot.state.clone(),
                Some(&snapshot),
            ));
            responses.push(command_completed(envelope, "open_scene", &snapshot));
        }
        Err(error) => {
            if let Ok(mut runtime) = runtime.lock() {
                runtime.fail(error.to_string());
            }
            responses.push(lifecycle_status(
                &envelope.message_id,
                &binding.session_id,
                LifecycleState::Failed,
                None,
            ));
            responses.push(Envelope::error(
                &envelope.message_id,
                "scene_open_failed",
                error.to_string(),
            ));
        }
    }
    responses
}

fn lifecycle_save_scene(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
    session_id: &str,
) -> Vec<Envelope> {
    let result = runtime
        .lock()
        .map_err(|_| HygeError::invalid_argument("runtime lock poisoned"))
        .and_then(|mut runtime| runtime.save_scene());
    match result {
        Ok(snapshot) => vec![
            lifecycle_status(
                &envelope.message_id,
                session_id,
                snapshot.state.clone(),
                Some(&snapshot),
            ),
            command_completed(envelope, "save_scene", &snapshot),
        ],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "scene_save_failed",
            error.to_string(),
        )],
    }
}

fn lifecycle_status(
    message_id: &str,
    session_id: &str,
    state: LifecycleState,
    snapshot: Option<&LifecycleSnapshot>,
) -> Envelope {
    let snapshot = snapshot
        .map(|snapshot| {
            serde_json::json!({
                "project_path": snapshot.project.as_ref().map(|path| path.display().to_string()),
                "scene_path": snapshot.scene.as_ref().map(|path| path.display().to_string()),
                "revision": snapshot.revision,
                "diagnostics": snapshot.diagnostics,
            })
        })
        .unwrap_or_else(|| serde_json::json!({}));
    let mut envelope = Envelope::new(
        message_id,
        MessageType::LifecycleStatus,
        serde_json::json!({
            "session_id": session_id,
            "state": state.as_str(),
            "details": snapshot,
        }),
    );
    envelope.correlation_id = Some(message_id.to_owned());
    envelope
}

fn command_completed(envelope: &Envelope, command: &str, snapshot: &LifecycleSnapshot) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({
            "command": command,
            "state": snapshot.state.as_str(),
            "project_path": snapshot.project.as_ref().map(|path| path.display().to_string()),
            "scene_path": snapshot.scene.as_ref().map(|path| path.display().to_string()),
            "revision": snapshot.revision,
            "diagnostics": snapshot.diagnostics,
        }),
    );
    response.correlation_id = Some(envelope.message_id.clone());
    response
}

/*
fn old_handler_placeholder(
    envelope: &Envelope,
    config: &EditorServerConfig,
    sessions: &Arc<Mutex<SessionRegistry>>,
    shutdown: &Arc<AtomicBool>,
    auth: &ConnectionAuth,
) -> Envelope {
    let _ = (envelope, config, sessions, shutdown, auth);
    Envelope::error(
            &envelope.message_id,
            "session_unavailable",
            "session registry is unavailable",
        );
}
*/

fn session_error_code(error: SessionError) -> &'static str {
    match error {
        SessionError::InvalidId => "invalid_request",
        SessionError::NotFound => "session_not_found",
        SessionError::Replaced => "session_replaced",
    }
}

fn session_error_message(error: SessionError) -> &'static str {
    match error {
        SessionError::InvalidId => "session_id is invalid",
        SessionError::NotFound => "editor session was not found",
        SessionError::Replaced => "editor session was replaced by a newer connection",
    }
}

fn map_timeout(error: ProtocolIoError) -> ProtocolIoError {
    if is_timeout(&error) {
        ProtocolIoError::Timeout
    } else {
        error
    }
}

fn is_timeout(error: &ProtocolIoError) -> bool {
    matches!(error, ProtocolIoError::Io(error) if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock))
}

fn is_expected_disconnect(error: &ProtocolIoError) -> bool {
    matches!(error, ProtocolIoError::Io(error) if matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe))
        || matches!(error, ProtocolIoError::Timeout)
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use hyge_editor_protocol::{read_envelope, write_envelope};

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
    fn hello_acknowledges_valid_client_and_assigns_session() {
        let server = server();
        let response = server.handle(&Envelope::hello("1", "hyge-local-dev"));
        assert_eq!(response.message_type, MessageType::HelloAck);
        assert!(response.payload["session_id"].as_str().is_some());
        assert_eq!(response.payload["resumed"], false);
    }

    #[test]
    fn rejects_incompatible_versions_and_duplicate_ids() {
        let server = server();
        let mut hello = Envelope::hello("1", "hyge-local-dev");
        hello.payload["supported_protocol_versions"] = serde_json::json!([99]);
        assert_eq!(
            server.handle(&hello).error.expect("error").code,
            "incompatible_version"
        );
        let first = server.handle(&Envelope::hello("2", "hyge-local-dev"));
        assert_eq!(first.message_type, MessageType::HelloAck);
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
    fn tcp_connection_round_trip_preserves_identity_and_reconnects() {
        let server = server();
        let address = server.local_addr().expect("address");
        let thread_server = server;
        let thread = thread::spawn(move || thread_server.run());

        let mut stream = TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let hello = Envelope::hello("hello-id", "hyge-local-dev");
        write_envelope(&mut stream, &hello).expect("write hello");
        let response = read_envelope(&mut stream).expect("read hello");
        let session_id = response.payload["session_id"]
            .as_str()
            .expect("session id")
            .to_owned();
        assert!(!response.payload["resumed"].as_bool().expect("resumed"));

        let open = Envelope::new(
            "project-id",
            MessageType::OpenProject,
            serde_json::json!({"path": "project"}),
        );
        write_envelope(&mut stream, &open).expect("write project");
        let _ = read_envelope(&mut stream).expect("read project");
        drop(stream);

        let mut reconnect = TcpStream::connect(address).expect("reconnect");
        reconnect
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let mut hello = Envelope::hello("resume-id", "hyge-local-dev");
        hello.payload["session_id"] = serde_json::json!(session_id);
        write_envelope(&mut reconnect, &hello).expect("write resume");
        let response = read_envelope(&mut reconnect).expect("read resume");
        assert_eq!(response.payload["resumed"], true);
        let shutdown = Envelope::new(
            "shutdown",
            MessageType::ServerShutdown,
            serde_json::json!({}),
        );
        write_envelope(&mut reconnect, &shutdown).expect("shutdown");
        let _ = read_envelope(&mut reconnect).expect("shutdown response");
        thread.join().expect("server thread").expect("run");
    }
}
