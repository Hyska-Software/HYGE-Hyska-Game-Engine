//! Canonical editor session registry.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyge_render::prelude::RendererConfig;

use crate::lifecycle::{EditorSessionRuntime, RuntimeHandle};
use crate::transport::{SharedViewportTransport, MAX_VIEWPORT_DIMENSION};
use crate::viewport::EditorRenderBridge;

/// Mutable metadata owned by one editor session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorState {
    /// Last opened project path.
    pub project: Option<String>,
    /// Last opened scene path.
    pub scene: Option<String>,
}

/// Opaque identity and generation for one authenticated connection.
#[derive(Clone, Debug)]
pub(crate) struct SessionBinding {
    pub(crate) session_id: String,
    generation: u64,
}

impl PartialEq for SessionBinding {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id && self.generation == other.generation
    }
}

impl Eq for SessionBinding {}

/// Publicly observable session metadata for diagnostics and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Stable session identity.
    pub session_id: String,
    /// Whether a current TCP connection owns the session.
    pub connected: bool,
    /// Session project metadata.
    pub state: EditorState,
}

struct SessionRecord {
    state: EditorState,
    runtime: RuntimeHandle,
    last_seen: Instant,
    generation: u64,
    connected: bool,
    transport: Option<SharedViewportTransport>,
    render_bridge: Option<EditorRenderBridge>,
    render_in_flight: bool,
    render_revisions: Option<(u64, u64)>,
    mutation_gate: Arc<Mutex<()>>,
}

/// In-process source of truth for reconnectable editor sessions.
#[derive(Default)]
pub(crate) struct SessionRegistry {
    sessions: HashMap<String, SessionRecord>,
    next_generation: u64,
}

impl SessionRegistry {
    pub(crate) fn bind(
        &mut self,
        requested_id: Option<&str>,
        ttl: Duration,
    ) -> Result<(SessionBinding, bool), SessionError> {
        self.expire(ttl);
        let now = Instant::now();
        let (session_id, resumed) = if let Some(session_id) = requested_id {
            if session_id.is_empty() {
                return Err(SessionError::InvalidId);
            }
            if !self.sessions.contains_key(session_id) {
                return Err(SessionError::NotFound);
            }
            (session_id.to_owned(), true)
        } else {
            (new_session_id(), false)
        };

        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let generation = self.next_generation;
        let record = self
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| SessionRecord {
                state: EditorState::default(),
                runtime: std::sync::Arc::new(std::sync::Mutex::new(EditorSessionRuntime::new())),
                last_seen: now,
                generation,
                connected: false,
                transport: None,
                render_bridge: None,
                render_in_flight: false,
                render_revisions: None,
                mutation_gate: Arc::new(Mutex::new(())),
            });
        // A reconnect must not invalidate a generation while a mutation from
        // the previous connection is still executing.  The old connection
        // therefore completes atomically before the new generation becomes
        // authoritative.
        let mutation_gate = Arc::clone(&record.mutation_gate);
        {
            let _guard = mutation_gate
                .lock()
                .map_err(|_| SessionError::Unavailable)?;
            record.last_seen = now;
            record.generation = generation;
            record.connected = true;
            record.transport = None;
            record.render_bridge = None;
            record.render_in_flight = false;
            record.render_revisions = None;
        }
        Ok((
            SessionBinding {
                session_id,
                generation,
            },
            resumed,
        ))
    }

    pub(crate) fn touch(&mut self, binding: &SessionBinding) -> Result<(), SessionError> {
        let record = self
            .sessions
            .get_mut(&binding.session_id)
            .ok_or(SessionError::NotFound)?;
        if record.generation != binding.generation || !record.connected {
            return Err(SessionError::Replaced);
        }
        record.last_seen = Instant::now();
        Ok(())
    }

    pub(crate) fn disconnect(&mut self, binding: &SessionBinding) {
        if let Some(record) = self.sessions.get_mut(&binding.session_id) {
            if record.generation == binding.generation {
                record.connected = false;
                record.last_seen = Instant::now();
            }
        }
    }

    pub(crate) fn update_project(
        &mut self,
        binding: &SessionBinding,
        project: String,
    ) -> Result<(), SessionError> {
        self.touch(binding)?;
        let record = self
            .sessions
            .get_mut(&binding.session_id)
            .ok_or(SessionError::NotFound)?;
        record.state.project = Some(project);
        Ok(())
    }

    pub(crate) fn update_scene(
        &mut self,
        binding: &SessionBinding,
        scene: String,
    ) -> Result<(), SessionError> {
        self.touch(binding)?;
        let record = self
            .sessions
            .get_mut(&binding.session_id)
            .ok_or(SessionError::NotFound)?;
        record.state.scene = Some(scene);
        Ok(())
    }

    /// Returns a session snapshot if the identity is still retained.
    pub fn snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.sessions.get(session_id).map(|record| SessionSnapshot {
            session_id: session_id.to_owned(),
            connected: record.connected,
            state: record.state.clone(),
        })
    }

    /// Returns the runtime handle for an authenticated session.
    pub fn runtime_handle(&self, session_id: &str) -> Option<crate::lifecycle::RuntimeHandle> {
        self.sessions
            .get(session_id)
            .map(|session| Arc::clone(&session.runtime))
    }

    pub(crate) fn mutation_guard(
        &self,
        binding: &SessionBinding,
    ) -> Result<(RuntimeHandle, Arc<Mutex<()>>), SessionError> {
        let record = self
            .sessions
            .get(&binding.session_id)
            .ok_or(SessionError::NotFound)?;
        if record.generation != binding.generation || !record.connected {
            return Err(SessionError::Replaced);
        }
        Ok((record.runtime.clone(), Arc::clone(&record.mutation_gate)))
    }

    pub(crate) fn is_current(&self, binding: &SessionBinding) -> bool {
        self.sessions
            .get(&binding.session_id)
            .is_some_and(|record| record.generation == binding.generation && record.connected)
    }

    pub(crate) fn open_transport(
        &mut self,
        binding: &SessionBinding,
        width: u32,
        height: u32,
    ) -> Result<(String, u64), SessionError> {
        let record = self
            .sessions
            .get_mut(&binding.session_id)
            .ok_or(SessionError::NotFound)?;
        if record.generation != binding.generation || !record.connected {
            return Err(SessionError::Replaced);
        }
        let name = format!(
            "Local\\hyge-editor-{}-{}",
            binding.session_id, binding.generation
        );
        if width == 0
            || height == 0
            || width > MAX_VIEWPORT_DIMENSION
            || height > MAX_VIEWPORT_DIMENSION
        {
            return Err(SessionError::Unavailable);
        }
        let geometry = record
            .runtime
            .lock()
            .map_err(|_| SessionError::Unavailable)?
            .viewport_geometry()
            .map_err(|_| SessionError::RendererUnavailable)?;
        let bridge = if let Some(bridge) = record.render_bridge.take() {
            bridge
        } else {
            EditorRenderBridge::new(RendererConfig::default(), geometry)
                .map_err(|_| SessionError::RendererUnavailable)?
        };
        let bytes = 64 + 3 * (64 + width as usize * height as usize * 4);
        record.transport = Some(
            SharedViewportTransport::create(name.clone(), binding.generation, bytes)
                .map_err(|_| SessionError::NotFound)?,
        );
        record.render_bridge = Some(bridge);
        record.render_in_flight = false;
        record.render_revisions = None;
        Ok((name, binding.generation))
    }

    pub(crate) fn close_transport(&mut self, binding: &SessionBinding) -> Result<(), SessionError> {
        let record = self
            .sessions
            .get_mut(&binding.session_id)
            .ok_or(SessionError::NotFound)?;
        if record.generation != binding.generation || !record.connected {
            return Err(SessionError::Replaced);
        }
        record.transport = None;
        record.render_in_flight = false;
        record.render_revisions = None;
        Ok(())
    }

    pub(crate) fn reset_transport(
        &mut self,
        binding: &SessionBinding,
        width: u32,
        height: u32,
    ) -> Result<(String, u64), SessionError> {
        if width == 0
            || height == 0
            || width > MAX_VIEWPORT_DIMENSION
            || height > MAX_VIEWPORT_DIMENSION
        {
            return Err(SessionError::NotFound);
        }
        self.close_transport(binding)?;
        self.open_transport(binding, width, height)
    }

    /// Pumps real extracted ECS frames through each session transport.
    pub(crate) fn pump_viewports(&mut self) {
        for record in self.sessions.values_mut() {
            if !record.connected {
                continue;
            }
            let (Some(transport), Some(bridge)) =
                (record.transport.as_mut(), record.render_bridge.as_ref())
            else {
                continue;
            };
            if let Some(result) = bridge.try_receive() {
                record.render_in_flight = false;
                let revisions = record.render_revisions.take();
                match result {
                    Ok(frame) => {
                        let Ok(mut runtime) = record.runtime.lock() else {
                            continue;
                        };
                        let (scene_revision, camera_revision) = revisions
                            .unwrap_or((frame.revision, runtime.viewport_state().camera_revision));
                        let current = runtime.viewport_state();
                        if current.scene_revision != scene_revision
                            || current.camera_revision != camera_revision
                        {
                            continue;
                        }
                        if let Err(error) = transport.publish(
                            frame.width,
                            frame.height,
                            scene_revision,
                            camera_revision,
                            &frame.pixels,
                        ) {
                            runtime
                                .degrade_viewport(format!("viewport transport publish: {error}"));
                        } else {
                            runtime.complete_viewport_render(frame.revision, camera_revision);
                        }
                    }
                    Err(error) => {
                        if let Ok(mut runtime) = record.runtime.lock() {
                            runtime.degrade_viewport(format!("viewport render: {error}"));
                        }
                    }
                }
            }
            if record.render_in_flight {
                continue;
            }
            let request = record
                .runtime
                .lock()
                .ok()
                .and_then(|mut runtime| runtime.next_viewport_render());
            if let Some(request) = request {
                match bridge.submit(request.revision, request.view, &request.snapshot) {
                    Ok(()) => {
                        record.render_in_flight = true;
                        record.render_revisions = Some((request.revision, request.camera_revision));
                    }
                    Err(error) => {
                        if let Ok(mut runtime) = record.runtime.lock() {
                            runtime.degrade_viewport(format!("viewport submit: {error}"));
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn expire(&mut self, ttl: Duration) {
        let now = Instant::now();
        self.sessions
            .retain(|_, record| now.duration_since(record.last_seen) <= ttl);
    }

    pub(crate) fn shutdown(&mut self) {
        let sessions = std::mem::take(&mut self.sessions);
        for record in sessions.into_values() {
            let Ok(_mutation_guard) = record.mutation_gate.lock() else {
                continue;
            };
            if let Ok(mut runtime) = record.runtime.lock() {
                runtime.shutdown();
            }
        }
    }
}

/// Session lifecycle errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionError {
    InvalidId,
    NotFound,
    Replaced,
    Unavailable,
    RendererUnavailable,
}

fn new_session_id() -> String {
    let mut bytes = [0_u8; 32];
    if getrandom::getrandom(&mut bytes).is_ok() {
        return blake3::hash(&bytes).to_hex().to_string();
    }
    static FALLBACK: AtomicU64 = AtomicU64::new(1);
    format!("local-{}", FALLBACK.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderable_project() -> std::path::PathBuf {
        use hyge_asset::importer::material::MaterialData;
        use hyge_asset::importer::mesh::{MeshData, Vertex};

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/r103-editor-project");
        let root = std::env::temp_dir().join(format!(
            "hyge-r103-pump-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("assets/cook")).expect("create project");
        std::fs::copy(
            fixture.join("main.hyge-world"),
            root.join("main.hyge-world"),
        )
        .expect("copy world");
        std::fs::copy(
            fixture.join("assets/persistent-cube.hyge-prefab"),
            root.join("assets/persistent-cube.hyge-prefab"),
        )
        .expect("copy prefab");
        let vertices = vec![
            Vertex {
                position: [-1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            },
            Vertex {
                position: [1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
            },
            Vertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.5, 1.0],
            },
        ];
        hyge_asset::importer::mesh::write(
            &root.join("assets/cook/000.hyge-mesh"),
            &MeshData::from_triangle_list(vertices, vec![0, 1, 2]),
        )
        .expect("cook mesh");
        hyge_asset::importer::material::write(
            &root.join("assets/cook/000.hyge-mat"),
            &MaterialData::default(),
        )
        .expect("cook material");
        root
    }

    #[test]
    fn reconnect_reuses_state_and_replaces_old_generation() {
        let mut registry = SessionRegistry::default();
        let (first, resumed) = registry.bind(None, Duration::from_secs(300)).expect("bind");
        assert!(!resumed);
        registry
            .update_project(&first, "project".into())
            .expect("project");
        let (second, resumed) = registry
            .bind(Some(&first.session_id), Duration::from_secs(300))
            .expect("resume");
        assert!(resumed);
        assert_ne!(first.generation, second.generation);
        assert_eq!(registry.touch(&first), Err(SessionError::Replaced));
        assert_eq!(
            registry
                .snapshot(&second.session_id)
                .expect("snapshot")
                .state
                .project
                .as_deref(),
            Some("project")
        );
    }

    #[test]
    fn unknown_and_empty_session_ids_are_distinct_errors() {
        let mut registry = SessionRegistry::default();
        assert_eq!(
            registry.bind(Some(""), Duration::from_secs(1)),
            Err(SessionError::InvalidId)
        );
        assert_eq!(
            registry.bind(Some("missing"), Duration::from_secs(1)),
            Err(SessionError::NotFound)
        );
    }

    #[test]
    fn viewport_transport_is_session_owned_and_replaced_on_reconnect() {
        let mut registry = SessionRegistry::default();
        let (first, _) = registry.bind(None, Duration::from_secs(300)).expect("bind");
        let (name, generation) = match registry.open_transport(&first, 640, 360) {
            Ok(value) => value,
            Err(SessionError::RendererUnavailable) => return,
            Err(error) => panic!("open transport: {error:?}"),
        };
        assert!(name.contains(&first.session_id));
        assert_eq!(generation, first.generation);
        let (second, resumed) = registry
            .bind(Some(&first.session_id), Duration::from_secs(300))
            .expect("reconnect");
        assert!(resumed);
        assert_eq!(
            registry.open_transport(&first, 640, 360),
            Err(SessionError::Replaced)
        );
        let (_, next_generation) = match registry.open_transport(&second, 640, 360) {
            Ok(value) => value,
            Err(SessionError::RendererUnavailable) => return,
            Err(error) => panic!("reopen transport: {error:?}"),
        };
        assert_ne!(generation, next_generation);
        registry.close_transport(&second).expect("close transport");
    }

    #[test]
    fn real_session_pump_publishes_consumable_rgba_frame() {
        let root = renderable_project();
        let mut registry = SessionRegistry::default();
        let (binding, _) = registry.bind(None, Duration::from_secs(300)).expect("bind");
        let runtime = registry
            .runtime_handle(&binding.session_id)
            .expect("runtime");
        {
            let mut runtime = runtime.lock().expect("runtime lock");
            runtime.open_project(&root).expect("open project");
            runtime
                .open_scene(&root.join("main.hyge-world"))
                .expect("open scene");
            runtime.set_viewport_size(32, 24);
        }
        match registry.open_transport(&binding, 32, 24) {
            Ok(_) => {}
            Err(SessionError::RendererUnavailable) => return,
            Err(error) => panic!("open transport: {error:?}"),
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let (header, pixels) = loop {
            registry.pump_viewports();
            let record = registry.sessions.get(&binding.session_id).expect("session");
            if let Some(transport) = record.transport.as_ref() {
                if let Some(header) = transport.ring.newest_header() {
                    let pixels = transport.ring.consume(&header).expect("consume frame");
                    break (header, pixels);
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "viewport frame timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!((header.width, header.height), (32, 24));
        assert_eq!(pixels.len(), 32 * 24 * 4);
        assert!(pixels.iter().any(|pixel| *pixel != 0));
        assert!(header.scene_revision > 0);
        assert!(header.camera_revision > 0);

        {
            use crate::commands::{EditComponentsCommand, EditorCommand};
            let mut runtime = runtime.lock().expect("runtime lock");
            let snapshot = runtime.editor_snapshot().expect("snapshot");
            let node = snapshot
                .hierarchy
                .iter()
                .find(|node| node.name == "Persistent Cube")
                .expect("persistent node");
            let transform = snapshot
                .component_catalog
                .iter()
                .find(|component| component.short_name == "Transform")
                .expect("Transform descriptor");
            runtime
                .apply_command(
                    snapshot.revision,
                    EditorCommand::EditComponents(EditComponentsCommand::new(
                        vec![node.entity],
                        transform.type_path.clone(),
                        Some("translation".into()),
                        serde_json::json!([2.0, 0.0, 0.0]),
                    )),
                )
                .expect("edit Transform");
        }
        let second = loop {
            registry.pump_viewports();
            let record = registry.sessions.get(&binding.session_id).expect("session");
            if let Some(transport) = record.transport.as_ref() {
                if let Some(candidate) = transport.ring.newest_header() {
                    if candidate.frame_id > header.frame_id {
                        let pixels = transport
                            .ring
                            .consume(&candidate)
                            .expect("consume edited frame");
                        break (candidate, pixels);
                    }
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "edited frame timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(second.0.scene_revision > header.scene_revision);
        assert_eq!(second.0.camera_revision, header.camera_revision);
        assert_ne!(blake3::hash(&pixels), blake3::hash(&second.1));
        let _ = std::fs::remove_dir_all(root);
    }
}
