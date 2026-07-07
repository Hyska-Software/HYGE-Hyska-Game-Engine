//! Worker-thread island metadata build for the Rapier integration.

use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::thread;

/// Snapshot sent to the island worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IslandBuildInput {
    /// Monotonic step generation.
    pub generation: u64,
    /// Number of dynamic rigid bodies in the backend.
    pub dynamic_bodies: usize,
    /// Number of colliders in the backend.
    pub colliders: usize,
}

/// Metadata returned from the island worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IslandBuildResult {
    /// Generation this result belongs to.
    pub generation: u64,
    /// Number of active dynamic bodies seen by the worker.
    pub dynamic_bodies: usize,
    /// Number of colliders seen by the worker.
    pub colliders: usize,
    /// Number of broad islands estimated by the worker.
    pub islands: usize,
}

/// Worker state for asynchronous island metadata builds.
#[derive(Debug)]
pub struct RapierIslandBuilder {
    pending: Mutex<Option<Receiver<IslandBuildResult>>>,
    last_result: Option<IslandBuildResult>,
}

impl Default for RapierIslandBuilder {
    fn default() -> Self {
        Self {
            pending: Mutex::new(None),
            last_result: None,
        }
    }
}

impl RapierIslandBuilder {
    /// Starts a worker-thread island build if one is not already pending.
    pub fn submit(&mut self, input: IslandBuildInput) {
        let mut pending = self
            .pending
            .lock()
            .expect("island worker pending lock should not be poisoned");
        if pending.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let islands = usize::from(input.dynamic_bodies > 0);
            let _ = tx.send(IslandBuildResult {
                generation: input.generation,
                dynamic_bodies: input.dynamic_bodies,
                colliders: input.colliders,
                islands,
            });
        });
        *pending = Some(rx);
    }

    /// Polls the worker result without blocking.
    pub fn poll(&mut self) -> Option<IslandBuildResult> {
        let result = self
            .pending
            .lock()
            .expect("island worker pending lock should not be poisoned")
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            *self
                .pending
                .lock()
                .expect("island worker pending lock should not be poisoned") = None;
            self.last_result = Some(result);
            Some(result)
        } else {
            None
        }
    }

    /// Returns the last completed island metadata result.
    #[must_use]
    pub fn last_result(&self) -> Option<IslandBuildResult> {
        self.last_result
    }
}
