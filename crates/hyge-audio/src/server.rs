//! Runtime audio server built on Kira.
//!
//! Wires real Kira listener and spatial sub tracks. The server can operate in
//! mock mode (no device) for CI/headless or in full mode with `AudioManager`.

use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::{Entity, Resource};
use hyge_core::prelude::{HygeError, HygeResult, Vec3};
use kira::listener::{ListenerHandle, ListenerId};
use kira::track::{SpatialTrackBuilder, SpatialTrackHandle};
use kira::Tween;
use kira::{AudioManager, AudioManagerSettings, DefaultBackend};

use crate::bus::{AudioBuses, BusKind};
use crate::spatial::SpatialListener;

/// Concrete Kira audio manager type used by Hyge.
pub type KiraAudioManager = AudioManager<DefaultBackend>;

/// Handle to an active spatial emitter in Kira.
#[derive(Debug)]
pub struct SpatialEmitterHandle {
    /// Kira spatial sub track handle.
    pub track: SpatialTrackHandle,
}

/// Runtime audio server.
#[derive(Resource)]
pub struct AudioServer {
    manager: Option<KiraAudioManager>,
    buses: AudioBuses,
    listener_handle: Option<ListenerHandle>,
    spatial_emitters: HashMap<Entity, SpatialEmitterHandle>,
    mock_spatial_emitters: HashSet<Entity>,
    play_requests: Vec<Entity>,
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
            listener_handle: None,
            spatial_emitters: HashMap::new(),
            mock_spatial_emitters: HashSet::new(),
            play_requests: Vec::new(),
        })
    }

    /// Creates a headless/mock audio server that performs routing math but does
    /// not open an audio device.
    #[must_use]
    pub fn mock() -> Self {
        Self {
            manager: None,
            buses: AudioBuses::default(),
            listener_handle: None,
            spatial_emitters: HashMap::new(),
            mock_spatial_emitters: HashSet::new(),
            play_requests: Vec::new(),
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

    /// Creates or updates the Kira spatial listener.
    pub fn set_listener(&mut self, listener: SpatialListener) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let position = glam_vec(listener.position);
        let orientation = glam_quat(listener.forward, listener.up);
        if let Some(handle) = self.listener_handle.as_mut() {
            handle.set_position(position, Tween::default());
            handle.set_orientation(orientation, Tween::default());
        } else {
            match manager.add_listener(position, orientation) {
                Ok(handle) => {
                    self.listener_handle = Some(handle);
                }
                Err(e) => {
                    tracing::warn!("failed to add Kira listener: {e:?}");
                }
            }
        }
    }

    /// Creates a spatial emitter sub track for an entity.
    pub fn add_spatial_emitter(
        &mut self,
        entity: Entity,
        position: Vec3,
        listener_id: Option<ListenerId>,
    ) {
        let lid = listener_id.or(self.listener_id());
        let Some(manager) = self.manager.as_mut() else {
            self.mock_spatial_emitters.insert(entity);
            return;
        };
        let Some(lid) = lid else {
            return;
        };
        let track = match manager.add_spatial_sub_track(
            lid,
            glam_vec(position),
            SpatialTrackBuilder::new(),
        ) {
            Ok(track) => track,
            Err(e) => {
                tracing::warn!("failed to add Kira spatial track: {e:?}");
                return;
            }
        };
        self.spatial_emitters
            .insert(entity, SpatialEmitterHandle { track });
    }

    /// Updates an existing spatial emitter's position.
    pub fn update_spatial_emitter(&mut self, entity: Entity, position: Vec3) {
        let Some(handle) = self.spatial_emitters.get_mut(&entity) else {
            return;
        };
        handle
            .track
            .set_position(glam_vec(position), Tween::default());
    }

    /// Removes a spatial emitter.
    pub fn remove_spatial_emitter(&mut self, entity: Entity) {
        self.spatial_emitters.remove(&entity);
        self.mock_spatial_emitters.remove(&entity);
    }

    /// Returns the number of spatial emitters currently registered.
    #[must_use]
    pub fn spatial_emitter_count(&self) -> usize {
        self.spatial_emitters.len() + self.mock_spatial_emitters.len()
    }

    /// Records a playback request for headless integration and diagnostics.
    pub fn record_play_request(&mut self, source: Entity) {
        self.play_requests.push(source);
    }

    /// Returns the number of playback requests recorded since construction.
    #[must_use]
    pub fn play_request_count(&self) -> usize {
        self.play_requests.len()
    }

    /// Returns the Kira listener id, if one is active.
    #[must_use]
    pub fn listener_id(&self) -> Option<ListenerId> {
        self.listener_handle.as_ref().map(ListenerHandle::id)
    }

    /// Returns a reference to the underlying Kira manager.
    #[must_use]
    pub fn manager(&self) -> Option<&KiraAudioManager> {
        self.manager.as_ref()
    }

    /// Returns a mutable reference to the Kira manager.
    pub fn manager_mut(&mut self) -> Option<&mut KiraAudioManager> {
        self.manager.as_mut()
    }
}

impl Default for AudioServer {
    fn default() -> Self {
        Self::mock()
    }
}

fn glam_vec(v: Vec3) -> glam::Vec3 {
    glam::Vec3::new(v.x, v.y, v.z)
}

fn glam_quat(forward: Vec3, up: Vec3) -> glam::Quat {
    let forward = forward.normalize();
    let up = up.normalize();
    let right = forward.cross(up).normalize();
    let up = right.cross(forward).normalize();
    // Kira's default listener faces -Z with +X right and +Y up. Map local -Z
    // to `forward`, local +X to `right`, and local +Y to `up`.
    let rotation = glam::Mat3::from_cols(right, up, -forward);
    glam::Quat::from_mat3(&rotation)
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

    #[test]
    fn mock_server_spatial_operations_are_noops() {
        let mut server = AudioServer::mock();
        let entity = Entity::from_raw(1);
        server.set_listener(SpatialListener {
            position: Vec3::new(0.0, 1.0, 0.0),
            ..Default::default()
        });
        server.add_spatial_emitter(entity, Vec3::new(10.0, 0.0, 0.0), None);
        server.update_spatial_emitter(entity, Vec3::new(11.0, 0.0, 0.0));
        server.remove_spatial_emitter(entity);
        assert!(server.listener_id().is_none());
    }
}
