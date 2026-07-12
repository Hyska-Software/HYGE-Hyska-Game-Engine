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
    commands::EditorCommand,
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
        if !self.shutdown.swap(true, Ordering::AcqRel) {
            // Wake the non-blocking accept loop immediately.  Active client
            // sockets still converge through their bounded read timeout and
            // the same session teardown path below.
            if let Ok(address) = self.listener.local_addr() {
                let _ = TcpStream::connect_timeout(&address, Duration::from_millis(50));
            }
            self.shutdown_resources();
        }
    }

    /// Returns a retained session snapshot.
    pub fn session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.sessions.lock().ok()?.snapshot(session_id)
    }

    /// Returns the data services for a retained session, for runtime sinks.
    pub fn session_data_services(
        &self,
        session_id: &str,
    ) -> Option<crate::data::EditorDataServices> {
        let runtime = self.sessions.lock().ok()?.runtime_handle(session_id)?;
        runtime.lock().ok().map(|runtime| runtime.data_services())
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
        self.shutdown_resources();
        result
    }

    fn shutdown_resources(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.shutdown();
        }
        self.cleanup_frontend();
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
        if !self.shutdown.swap(true, Ordering::AcqRel) {
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.shutdown();
            }
            self.cleanup_frontend();
        }
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
    let (runtime, mutation_gate) = {
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
        match registry.mutation_guard(binding) {
            Ok(access) => access,
            Err(error) => {
                return vec![Envelope::error(
                    &envelope.message_id,
                    session_error_code(error),
                    session_error_message(error),
                )]
            }
        }
    };
    let Ok(_mutation_guard) = mutation_gate.lock() else {
        return vec![Envelope::error(
            &envelope.message_id,
            "session_unavailable",
            "editor session mutation gate is unavailable",
        )];
    };
    let Ok(registry) = sessions.lock() else {
        return vec![Envelope::error(
            &envelope.message_id,
            "session_unavailable",
            "session registry is unavailable",
        )];
    };
    if !registry.is_current(binding) {
        return vec![Envelope::error(
            &envelope.message_id,
            "session_replaced",
            "editor session was replaced by a newer connection",
        )];
    }
    drop(registry);
    let mut responses = poll_scene_events(&runtime, &binding.session_id);
    responses.extend(match envelope.message_type.clone() {
        MessageType::OpenProject => lifecycle_open_project(envelope, sessions, binding, runtime),
        MessageType::OpenScene => lifecycle_open_scene(envelope, sessions, binding, runtime),
        MessageType::ActivateAsset => asset_activate(envelope, sessions, binding, runtime),
        MessageType::RequestWorldSnapshot => world_snapshot_request(envelope, runtime),
        MessageType::SaveScene => lifecycle_save_scene(envelope, runtime, &binding.session_id),
        MessageType::ResolveSceneReload => resolve_scene_reload(envelope, runtime),
        MessageType::SelectEntities => {
            lifecycle_select_entities(envelope, runtime, &binding.session_id)
        }
        MessageType::SetEditorCamera => viewport_set_camera(envelope, runtime),
        MessageType::SetViewportSize => viewport_set_size(envelope, runtime),
        MessageType::ViewportInput => viewport_input(envelope, runtime),
        MessageType::OpenViewportTransport => viewport_open_transport(envelope, sessions, binding),
        MessageType::CloseViewportTransport => {
            viewport_close_transport(envelope, sessions, binding)
        }
        MessageType::ViewportTransportReset => {
            viewport_reset_transport(envelope, sessions, binding)
        }
        MessageType::EditComponent
        | MessageType::EditComponents
        | MessageType::AddComponent
        | MessageType::RemoveComponent
        | MessageType::ReparentEntity
        | MessageType::DuplicateEntity
        | MessageType::DestroyEntity
        | MessageType::InstantiatePrefab
        | MessageType::Undo
        | MessageType::Redo => editor_command(envelope, runtime),
        MessageType::RequestAssetSnapshot => data_asset_snapshot(envelope, runtime),
        MessageType::RequestConsoleSnapshot => data_console_snapshot(envelope, runtime),
        MessageType::RequestProfilerSnapshot => data_profiler_snapshot(envelope, runtime),
        MessageType::RequestAssetPreview => data_asset_preview(envelope, runtime),
        MessageType::CancelAssetPreview => data_cancel_preview(envelope, runtime),
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
    });
    responses
}

fn poll_scene_events(runtime: &crate::lifecycle::RuntimeHandle, session_id: &str) -> Vec<Envelope> {
    let result = runtime
        .lock()
        .ok()
        .and_then(|mut runtime| runtime.poll_scene_reload().ok().flatten());
    match result {
        Some(crate::lifecycle::SceneReloadEvent::Conflict(conflict)) => vec![Envelope::new(
            format!("scene-reload-conflict-{session_id}"),
            MessageType::SceneReloadConflict,
            serde_json::json!({
                "path": conflict.path.display().to_string(),
                "external_asset_id": asset_id_string(conflict.external_asset_id),
                "dirty": true,
                "actions": ["reload_discard", "keep_editor", "save_then_reload"]
            }),
        )],
        Some(crate::lifecycle::SceneReloadEvent::Reloaded(report)) => vec![Envelope::new(
            format!("scene-reloaded-{session_id}"),
            MessageType::SceneReloaded,
            serde_json::json!({
                "session_id": session_id,
                "diff": {
                    "added_instances": report.diff.added_instances,
                    "removed_instances": report.diff.removed_instances,
                    "changed_instances": report.diff.changed_instances,
                    "environment_changed": report.diff.environment_changed,
                    "post_process_changed": report.diff.post_process_changed
                },
                "preserved_scene_ids": report.preserved_scene_ids,
                "restored_scene_ids": report.restored_scene_ids,
                "reattached_scene_ids": report.reattached_scene_ids
            }),
        )],
        None => Vec::new(),
    }
}

fn asset_id_string(id: hyge_asset::AssetId) -> String {
    id.0.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn viewport_open_transport(
    envelope: &Envelope,
    sessions: &Arc<Mutex<SessionRegistry>>,
    binding: &crate::state::SessionBinding,
) -> Vec<Envelope> {
    match sessions
        .lock()
        .ok()
        .and_then(|mut registry| registry.open_transport(binding).ok())
    {
        Some((name, generation)) => vec![Envelope::new(
            &envelope.message_id,
            MessageType::ViewportTransportReady,
            serde_json::json!({"mapping_name":name,"generation":generation,"width":640,"height":360,"pixel_format":"rgba8_srgb","ring_slots":3}),
        )],
        None => vec![Envelope::error(
            &envelope.message_id,
            "viewport_transport_unavailable",
            "could not open session viewport mapping",
        )],
    }
}

fn viewport_close_transport(
    envelope: &Envelope,
    sessions: &Arc<Mutex<SessionRegistry>>,
    binding: &crate::state::SessionBinding,
) -> Vec<Envelope> {
    match sessions
        .lock()
        .ok()
        .and_then(|mut registry| registry.close_transport(binding).ok())
    {
        Some(()) => vec![Envelope::new(
            &envelope.message_id,
            MessageType::CommandCompleted,
            serde_json::json!({"command":"close_viewport_transport","released":true}),
        )],
        None => vec![Envelope::error(
            &envelope.message_id,
            "viewport_transport_unavailable",
            "could not close session viewport mapping",
        )],
    }
}

fn viewport_reset_transport(
    envelope: &Envelope,
    sessions: &Arc<Mutex<SessionRegistry>>,
    binding: &crate::state::SessionBinding,
) -> Vec<Envelope> {
    let width = envelope
        .payload
        .get("width")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(640) as u32;
    let height = envelope
        .payload
        .get("height")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(360) as u32;
    match sessions
        .lock()
        .ok()
        .and_then(|mut registry| registry.reset_transport(binding, width, height).ok())
    {
        Some((name, generation)) => vec![Envelope::new(
            &envelope.message_id,
            MessageType::ViewportTransportReset,
            serde_json::json!({"mapping_name":name,"generation":generation,"width":width,"height":height}),
        )],
        None => vec![Envelope::error(
            &envelope.message_id,
            "viewport_transport_unavailable",
            "could not reset session viewport mapping",
        )],
    }
}

fn viewport_input(envelope: &Envelope, runtime: crate::lifecycle::RuntimeHandle) -> Vec<Envelope> {
    let generation = envelope
        .payload
        .get("generation")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let batch = serde_json::from_value::<crate::ViewportInputBatch>(envelope.payload.clone());
    match batch.and_then(|batch| {
        runtime
            .lock()
            .map_err(|_| serde_json::Error::io(std::io::Error::other("runtime lock poisoned")))
            .and_then(|mut runtime| {
                runtime
                    .apply_viewport_input(&batch, generation)
                    .map_err(|error| serde_json::Error::io(std::io::Error::other(error)))
                    .map(|revision| (batch, revision))
            })
    }) {
        Ok((_batch, revision)) => vec![Envelope::new(
            &envelope.message_id,
            MessageType::CommandCompleted,
            serde_json::json!({"command":"viewport_input","input_revision":revision}),
        )],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "invalid_viewport_input",
            error.to_string(),
        )],
    }
}

fn data_asset_snapshot(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let result = runtime
        .lock()
        .map_err(|_| "runtime lock poisoned".to_owned())
        .and_then(|runtime| runtime.asset_snapshot());
    match result {
        Ok(snapshot) => vec![data_envelope(
            envelope,
            MessageType::AssetSnapshot,
            serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})),
        )],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "asset_db_unavailable",
            error,
        )],
    }
}

fn asset_activate(
    envelope: &Envelope,
    sessions: &Arc<Mutex<SessionRegistry>>,
    binding: &crate::state::SessionBinding,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let Some(asset_id) = envelope
        .payload
        .get("asset_id")
        .and_then(serde_json::Value::as_str)
    else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "activate_asset requires asset_id",
        )];
    };
    let path = match runtime
        .lock()
        .map_err(|_| "runtime lock poisoned".to_owned())
        .and_then(|runtime| runtime.asset_path(asset_id))
    {
        Ok(path) => path,
        Err(error) => {
            return vec![Envelope::error(
                &envelope.message_id,
                "invalid_asset",
                error,
            )]
        }
    };
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("hyge-world") => {
            let mut forwarded = envelope.clone();
            forwarded.message_type = MessageType::OpenScene;
            forwarded.payload = serde_json::json!({"path": path});
            lifecycle_open_scene(&forwarded, sessions, binding, runtime)
        }
        Some("hyge-prefab") => {
            let Some(expected_revision) = envelope
                .payload
                .get("expected_revision")
                .and_then(serde_json::Value::as_u64)
            else {
                return vec![Envelope::error(
                    &envelope.message_id,
                    "invalid_request",
                    "prefab activation requires expected_revision",
                )];
            };
            match runtime
                .lock()
                .map_err(|_| "runtime lock poisoned".to_owned())
                .and_then(|mut runtime| {
                    runtime
                        .instantiate_asset_prefab(asset_id, expected_revision)
                        .map_err(|error| error.message)
                }) {
                Ok((_effect, snapshot)) => vec![
                    world_snapshot(envelope, &snapshot),
                    selection_changed(envelope, &snapshot),
                    data_envelope(
                        envelope,
                        MessageType::CommandCompleted,
                        serde_json::json!({"command":"activate_asset","revision":snapshot.revision}),
                    ),
                ],
                Err(error) => vec![Envelope::error(
                    &envelope.message_id,
                    "asset_activation_failed",
                    error,
                )],
            }
        }
        Some("hyge-mesh") => data_asset_preview(envelope, runtime),
        _ => vec![Envelope::error(
            &envelope.message_id,
            "unsupported_asset",
            "only .hyge-world, .hyge-prefab and .hyge-mesh can be activated",
        )],
    }
}

fn data_console_snapshot(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let filter = crate::data::ConsoleFilter {
        min_level: envelope
            .payload
            .get("min_level")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        target_prefix: envelope
            .payload
            .get("target_prefix")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    };
    let result = runtime
        .lock()
        .map(|runtime| runtime.console_snapshot(filter))
        .map_err(|_| "runtime lock poisoned");
    match result {
        Ok(snapshot) => vec![data_envelope(
            envelope,
            MessageType::ConsoleSnapshot,
            serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})),
        )],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "console_unavailable",
            error,
        )],
    }
}

fn data_profiler_snapshot(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let result = runtime
        .lock()
        .map(|runtime| runtime.profiler_snapshot())
        .map_err(|_| "runtime lock poisoned");
    match result {
        Ok(snapshot) => vec![data_envelope(
            envelope,
            MessageType::ProfilerSnapshot,
            serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})),
        )],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "profiler_unavailable",
            error,
        )],
    }
}

fn data_asset_preview(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let Some(asset_id) = envelope
        .payload
        .get("asset_id")
        .and_then(serde_json::Value::as_str)
    else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "request_asset_preview requires asset_id",
        )];
    };
    let job_id = envelope
        .payload
        .get("job_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&envelope.message_id);
    let result = runtime
        .lock()
        .map_err(|_| "runtime lock poisoned".to_owned())
        .and_then(|runtime| runtime.request_asset_preview(asset_id, job_id));
    match result {
        Ok(result) => vec![data_envelope(
            envelope,
            MessageType::AssetPreviewReady,
            serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})),
        )],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "preview_failed",
            error,
        )],
    }
}

fn data_cancel_preview(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let Some(job_id) = envelope
        .payload
        .get("job_id")
        .and_then(serde_json::Value::as_str)
    else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "cancel_asset_preview requires job_id",
        )];
    };
    let cancelled = runtime
        .lock()
        .map(|runtime| runtime.cancel_asset_preview(job_id))
        .unwrap_or(false);
    if cancelled {
        vec![data_envelope(
            envelope,
            MessageType::AssetPreviewCancelled,
            serde_json::json!({"job_id": job_id, "state": "cancelled"}),
        )]
    } else {
        vec![Envelope::error(
            &envelope.message_id,
            "preview_not_found",
            "preview job was not found",
        )]
    }
}

fn data_envelope(
    envelope: &Envelope,
    message_type: MessageType,
    payload: serde_json::Value,
) -> Envelope {
    let mut response = Envelope::new(&envelope.message_id, message_type, payload);
    response.correlation_id = Some(envelope.message_id.clone());
    response
}

fn editor_command(envelope: &Envelope, runtime: crate::lifecycle::RuntimeHandle) -> Vec<Envelope> {
    let expected_revision = match envelope
        .payload
        .get("expected_revision")
        .and_then(serde_json::Value::as_u64)
    {
        Some(revision) => revision,
        None => {
            return vec![Envelope::error(
                &envelope.message_id,
                "invalid_request",
                "mutating editor requests require expected_revision",
            )]
        }
    };
    let result = runtime
        .lock()
        .map_err(|_| {
            crate::commands::CommandFailure::new("command_failed", "runtime lock poisoned")
        })
        .and_then(|mut runtime| match envelope.message_type {
            MessageType::Undo => runtime.undo_command(expected_revision),
            MessageType::Redo => runtime.redo_command(expected_revision),
            _ => {
                let command = decode_editor_command(envelope)?;
                runtime.apply_command(expected_revision, command)
            }
        });
    match result {
        Ok((effect, snapshot)) => {
            let command = command_name(envelope.message_type.clone());
            vec![
                world_snapshot(envelope, &snapshot),
                selection_changed(envelope, &snapshot),
                command_completed_editor(envelope, command, &effect, &snapshot),
            ]
        }
        Err(error) => vec![command_error(envelope, error)],
    }
}

fn command_name(message_type: MessageType) -> &'static str {
    match message_type {
        MessageType::EditComponent => "edit_component",
        MessageType::EditComponents => "edit_components",
        MessageType::AddComponent => "add_component",
        MessageType::RemoveComponent => "remove_component",
        MessageType::ReparentEntity => "reparent_entity",
        MessageType::DuplicateEntity => "duplicate_entity",
        MessageType::DestroyEntity => "destroy_entity",
        MessageType::InstantiatePrefab => "instantiate_prefab",
        MessageType::Undo => "undo",
        MessageType::Redo => "redo",
        _ => "editor_command",
    }
}

fn decode_editor_command(
    envelope: &Envelope,
) -> Result<EditorCommand, crate::commands::CommandFailure> {
    let payload = envelope.payload.clone();
    fn decode<T: serde::de::DeserializeOwned>(
        value: serde_json::Value,
    ) -> Result<T, crate::commands::CommandFailure> {
        serde_json::from_value(value).map_err(|error| {
            crate::commands::CommandFailure::new("invalid_request", error.to_string())
        })
    }
    match envelope.message_type.clone() {
        MessageType::EditComponent => decode::<crate::commands::EditComponentCommand>(payload)
            .map(EditorCommand::EditComponent),
        MessageType::EditComponents => decode::<crate::commands::EditComponentsCommand>(payload)
            .map(EditorCommand::EditComponents),
        MessageType::AddComponent => {
            decode::<crate::commands::AddComponentCommand>(payload).map(EditorCommand::AddComponent)
        }
        MessageType::RemoveComponent => decode::<crate::commands::RemoveComponentCommand>(payload)
            .map(EditorCommand::RemoveComponent),
        MessageType::ReparentEntity => {
            decode::<crate::commands::ReparentCommand>(payload).map(EditorCommand::Reparent)
        }
        MessageType::DuplicateEntity => {
            decode::<crate::commands::DuplicateCommand>(payload).map(EditorCommand::Duplicate)
        }
        MessageType::DestroyEntity => {
            decode::<crate::commands::DestroyCommand>(payload).map(EditorCommand::Destroy)
        }
        MessageType::InstantiatePrefab => {
            decode::<crate::commands::InstantiateCommand>(payload).map(EditorCommand::Instantiate)
        }
        _ => Err(crate::commands::CommandFailure::new(
            "invalid_request",
            "message is not an editor command",
        )),
    }
}

fn world_snapshot_request(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    match runtime
        .lock()
        .map_err(|_| "runtime lock poisoned".to_owned())
        .and_then(|runtime| runtime.editor_snapshot().map_err(|error| error.to_string()))
    {
        Ok(snapshot) => vec![
            world_snapshot(envelope, &snapshot),
            selection_changed(envelope, &snapshot),
        ],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "snapshot_unavailable",
            error,
        )],
    }
}

fn command_error(envelope: &Envelope, error: crate::commands::CommandFailure) -> Envelope {
    Envelope::error(&envelope.message_id, &error.code, &error.message)
}

fn viewport_set_camera(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let camera = match serde_json::from_value::<crate::EditorCameraState>(envelope.payload.clone())
    {
        Ok(camera) => camera,
        Err(error) => {
            return vec![Envelope::error(
                &envelope.message_id,
                "invalid_request",
                error.to_string(),
            )]
        }
    };
    let result = runtime
        .lock()
        .map_err(|_| "runtime lock poisoned".to_owned())
        .and_then(|mut runtime| {
            runtime
                .set_editor_camera(camera)
                .map(|_| runtime.viewport_state())
        });
    match result {
        Ok(viewport) => vec![viewport_completed(envelope, "set_editor_camera", &viewport)],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            error,
        )],
    }
}

fn viewport_set_size(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let width = envelope
        .payload
        .get("width")
        .and_then(serde_json::Value::as_u64);
    let height = envelope
        .payload
        .get("height")
        .and_then(serde_json::Value::as_u64);
    let (Some(width), Some(height)) = (width, height) else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "viewport size requires numeric width and height",
        )];
    };
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "viewport dimensions must be between 1 and 16384",
        )];
    }
    let result = runtime
        .lock()
        .map_err(|_| "runtime lock poisoned".to_owned())
        .map(|mut runtime| runtime.set_viewport_size(width as u32, height as u32));
    match result {
        Ok(viewport) => vec![viewport_completed(envelope, "set_viewport_size", &viewport)],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "viewport_unavailable",
            error,
        )],
    }
}

fn viewport_completed(
    envelope: &Envelope,
    command: &str,
    viewport: &crate::ViewportState,
) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({
            "command": command,
            "width": viewport.width,
            "height": viewport.height,
            "camera_revision": viewport.camera_revision,
            "scene_revision": viewport.scene_revision,
            "last_frame_revision": viewport.last_frame_revision,
            "state": format!("{:?}", viewport.state).to_lowercase(),
        }),
    );
    response.correlation_id = Some(envelope.message_id.clone());
    response
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
            responses.push(Envelope::diagnostic_error(
                &envelope.message_id,
                "project_open_failed",
                error.to_string(),
                true,
                Some(path.to_owned()),
                Some("open_project".to_owned()),
                Some("check the project path and lock, then retry".to_owned()),
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
            if let Ok(runtime) = runtime.lock() {
                if let Ok(editor_snapshot) = runtime.editor_snapshot() {
                    responses.push(world_snapshot(envelope, &editor_snapshot));
                    responses.push(selection_changed(envelope, &editor_snapshot));
                }
            }
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
            responses.push(Envelope::diagnostic_error(
                &envelope.message_id,
                "scene_open_failed",
                error.to_string(),
                true,
                Some(path.to_owned()),
                Some("open_scene".to_owned()),
                Some("fix the scene or prefab diagnostic, then retry".to_owned()),
            ));
        }
    }
    responses
}

fn lifecycle_select_entities(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
    session_id: &str,
) -> Vec<Envelope> {
    let values = envelope
        .payload
        .get("entities")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let scene_ids: Vec<String> = envelope
        .payload
        .get("scene_ids")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if values.is_empty()
        && scene_ids.is_empty()
        && envelope.payload.get("entities").is_none()
        && envelope.payload.get("scene_ids").is_none()
    {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "select_entities requires an entities array",
        )];
    };
    let entities = values
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .collect();
    let shift = envelope
        .payload
        .get("shift")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let result = runtime
        .lock()
        .map_err(|_| HygeError::invalid_argument("runtime lock poisoned"))
        .and_then(|mut runtime| {
            if !scene_ids.is_empty() {
                runtime.select_scene_ids(scene_ids, shift)
            } else {
                runtime.select_entities_with_shift(entities, shift)
            }
        });
    match result {
        Ok(snapshot) => vec![
            selection_changed(envelope, &snapshot),
            command_completed_snapshot(envelope, "select_entities", session_id, &snapshot),
        ],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "selection_failed",
            error.to_string(),
        )],
    }
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

fn resolve_scene_reload(
    envelope: &Envelope,
    runtime: crate::lifecycle::RuntimeHandle,
) -> Vec<Envelope> {
    let Some(action) = envelope
        .payload
        .get("action")
        .and_then(serde_json::Value::as_str)
    else {
        return vec![Envelope::error(
            &envelope.message_id,
            "invalid_request",
            "resolve_scene_reload requires action",
        )];
    };
    let result = runtime
        .lock()
        .map_err(|_| HygeError::invalid_argument("runtime lock poisoned"))
        .and_then(|mut runtime| runtime.resolve_scene_reload(action));
    match result {
        Ok(Some(report)) => vec![
            Envelope::new(
                format!("scene-reloaded-{}", envelope.message_id),
                MessageType::SceneReloaded,
                serde_json::json!({
                    "diff": {
                        "added_instances": report.diff.added_instances,
                        "removed_instances": report.diff.removed_instances,
                        "changed_instances": report.diff.changed_instances,
                        "environment_changed": report.diff.environment_changed,
                        "post_process_changed": report.diff.post_process_changed
                    },
                    "preserved_scene_ids": report.preserved_scene_ids,
                    "restored_scene_ids": report.restored_scene_ids,
                    "reattached_scene_ids": report.reattached_scene_ids
                }),
            ),
            command_completed_simple(envelope, "resolve_scene_reload"),
        ],
        Ok(None) => vec![command_completed_simple(envelope, "resolve_scene_reload")],
        Err(error) => vec![Envelope::error(
            &envelope.message_id,
            "scene_reload_failed",
            error.to_string(),
        )],
    }
}

fn command_completed_simple(envelope: &Envelope, command: &str) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({"command": command}),
    );
    response.correlation_id = Some(envelope.message_id.clone());
    response
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

fn world_snapshot(envelope: &Envelope, snapshot: &crate::snapshots::EditorSnapshot) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::WorldSnapshot,
        serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})),
    );
    response.correlation_id = Some(envelope.message_id.clone());
    response
}

fn selection_changed(envelope: &Envelope, snapshot: &crate::snapshots::EditorSnapshot) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::SelectionChanged,
        serde_json::json!({
            "revision": snapshot.revision,
            "scene_revision": snapshot.scene_revision,
            "entities": snapshot.selection,
            "scene_ids": snapshot.selection_scene_ids,
        }),
    );
    response.correlation_id = Some(envelope.message_id.clone());
    response
}

fn command_completed_snapshot(
    envelope: &Envelope,
    command: &str,
    session_id: &str,
    snapshot: &crate::snapshots::EditorSnapshot,
) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({
            "command": command,
            "session_id": session_id,
            "revision": snapshot.revision,
            "scene_revision": snapshot.scene_revision,
        }),
    );
    response.correlation_id = Some(envelope.message_id.clone());
    response
}

fn command_completed_editor(
    envelope: &Envelope,
    command: &str,
    effect: &crate::commands::CommandEffect,
    snapshot: &crate::snapshots::EditorSnapshot,
) -> Envelope {
    let mut response = Envelope::new(
        &envelope.message_id,
        MessageType::CommandCompleted,
        serde_json::json!({
            "command": command,
            "operation": if matches!(envelope.message_type.clone(), MessageType::Undo) { "undo" } else if matches!(envelope.message_type.clone(), MessageType::Redo) { "redo" } else { "apply" },
            "revision": snapshot.revision,
            "scene_revision": snapshot.scene_revision,
            "affected_entities": effect.affected_entities,
            "entity_remappings": effect.entity_remappings,
            "selection": snapshot.selection,
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
        SessionError::Unavailable => "session_unavailable",
    }
}

fn session_error_message(error: SessionError) -> &'static str {
    match error {
        SessionError::InvalidId => "session_id is invalid",
        SessionError::NotFound => "editor session was not found",
        SessionError::Replaced => "editor session was replaced by a newer connection",
        SessionError::Unavailable => "editor session is temporarily unavailable",
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
