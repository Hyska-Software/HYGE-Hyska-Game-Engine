//! Canonical editor session registry.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::lifecycle::{EditorSessionRuntime, RuntimeHandle};
use crate::transport::{SharedViewportTransport, MAX_VIEWPORT_DIMENSION};

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
        let bytes = 64 + 3 * (64 + 640 * 360 * 4);
        record.transport = Some(
            SharedViewportTransport::create(name.clone(), binding.generation, bytes)
                .map_err(|_| SessionError::NotFound)?,
        );
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
        self.open_transport(binding)
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
        let (name, generation) = registry.open_transport(&first).expect("open transport");
        assert!(name.contains(&first.session_id));
        assert_eq!(generation, first.generation);
        let (second, resumed) = registry
            .bind(Some(&first.session_id), Duration::from_secs(300))
            .expect("reconnect");
        assert!(resumed);
        assert_eq!(registry.open_transport(&first), Err(SessionError::Replaced));
        let (_, next_generation) = registry.open_transport(&second).expect("reopen transport");
        assert_ne!(generation, next_generation);
        registry.close_transport(&second).expect("close transport");
    }
}
