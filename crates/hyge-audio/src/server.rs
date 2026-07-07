//! Runtime audio server built on Kira.

use bevy_ecs::prelude::Resource;
use hyge_core::prelude::{HygeError, HygeResult};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend};

use crate::bus::{AudioBuses, BusKind};

/// Concrete Kira audio manager type used by Hyge.
pub type KiraAudioManager = AudioManager<DefaultBackend>;

/// Runtime audio server.
#[derive(Resource)]
pub struct AudioServer {
    manager: Option<KiraAudioManager>,
    buses: AudioBuses,
}

impl AudioServer {
    /// Creates an audio server using Kira's default backend.
    ///
    /// # Errors
    ///
    /// Returns an error if no audio backend/device can be initialized.
    pub fn new() -> HygeResult<Self> {
        let manager = KiraAudioManager::new(AudioManagerSettings::default())
            .map_err(|e| HygeError::unsupported(format!("failed to initialize Kira: {e:?}")))?;
        Ok(Self {
            manager: Some(manager),
            buses: AudioBuses::default(),
        })
    }

    /// Creates a headless/mock audio server that performs routing math but does
    /// not open an audio device.
    #[must_use]
    pub fn mock() -> Self {
        Self {
            manager: None,
            buses: AudioBuses::default(),
        }
    }

    /// Returns true when a real Kira manager is available.
    #[must_use]
    pub fn has_backend(&self) -> bool {
        self.manager.is_some()
    }

    /// Returns immutable bus state.
    #[must_use]
    pub fn buses(&self) -> &AudioBuses {
        &self.buses
    }

    /// Returns mutable bus state.
    pub fn buses_mut(&mut self) -> &mut AudioBuses {
        &mut self.buses
    }

    /// Computes effective source gain after bus inheritance.
    #[must_use]
    pub fn effective_gain(&self, bus: BusKind, source_volume: f32) -> f32 {
        self.buses.volumes.inherited_gain(bus) * source_volume
    }
}

impl Default for AudioServer {
    fn default() -> Self {
        Self::mock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_server_does_not_require_device() {
        let server = AudioServer::mock();
        assert!(!server.has_backend());
        assert_eq!(server.effective_gain(BusKind::Music, 0.5), 0.5);
    }
}
