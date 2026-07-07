//! Spatial audio math, attenuation, and backend integration.
//!
//! Two backends are supported:
//! - Kira's built-in spatial sub tracks (always available).
//! - `oddio` spatial scene (behind `audio-hrtf` feature, provides an
//!   alternative spatial mixer that can be paired with the `hrtf` crate for
//!   HRTF binaural rendering).

use hyge_core::prelude::Vec3;

use crate::components::AudioRolloff;

/// A listener in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialListener {
    /// Listener position in world space.
    pub position: Vec3,
    /// Listener orientation, represented as a Kira-compatible orientation in
    /// higher-level backend setup.
    pub forward: Vec3,
    /// Listener up vector.
    pub up: Vec3,
}

impl Default for SpatialListener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            up: Vec3::Y,
        }
    }
}

/// Spatial emitter descriptor used by the audio backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialEmitter {
    /// Emitter position in world space.
    pub position: Vec3,
    /// Maximum audible range.
    pub range: f32,
}

impl Default for SpatialEmitter {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            range: 1.0,
        }
    }
}

/// Computes attenuation for `distance`, `range`, and rolloff model.
#[must_use]
pub fn attenuation_gain(distance: f32, range: f32, rolloff: AudioRolloff) -> f32 {
    let range = range.max(f32::EPSILON);
    let distance = distance.max(0.0);
    if distance >= range {
        return 0.0;
    }

    match rolloff {
        AudioRolloff::Linear => 1.0 - (distance / range),
        AudioRolloff::Inverse => 1.0 / (1.0 + distance),
        AudioRolloff::Logarithmic => 1.0 - (distance + 1.0).ln() / (range + 1.0).ln(),
    }
    .clamp(0.0, 1.0)
}

/// Computes attenuation from listener/emitter positions.
#[must_use]
pub fn listener_emitter_gain(
    listener: SpatialListener,
    emitter: SpatialEmitter,
    rolloff: AudioRolloff,
) -> f32 {
    attenuation_gain(
        listener.position.distance(emitter.position),
        emitter.range,
        rolloff,
    )
}

/// Oddio-based spatial scene, available behind `audio-hrtf`.
///
/// Wraps `oddio::SpatialScene` to provide a real spatial audio mixer that
/// can be paired with the `hrtf` crate for binaural HRTF rendering. The scene
/// uses oddio's `SpatialOptions` for per-source position/velocity updates.
#[cfg(feature = "audio-hrtf")]
pub struct OddioSpatialScene {
    handle: oddio::SpatialSceneControl,
}

#[cfg(feature = "audio-hrtf")]
impl OddioSpatialScene {
    /// Creates a new oddio spatial scene.
    #[must_use]
    pub fn new() -> Self {
        let (handle, _scene) = oddio::SpatialScene::new();
        Self { handle }
    }

    /// Returns the underlying oddio scene control handle.
    #[must_use]
    pub fn handle(&self) -> &oddio::SpatialSceneControl {
        &self.handle
    }

    /// Plays a mono signal at the given position.
    ///
    /// `signal` must be a single-channel (`oddio::Sample`, i.e. `f32`) signal.
    pub fn play(
        &mut self,
        signal: oddio::FramesSignal<oddio::Sample>,
        position: Vec3,
        velocity: Vec3,
    ) -> oddio::Spatial {
        let options = oddio::SpatialOptions {
            position: mint::Point3 {
                x: position.x,
                y: position.y,
                z: position.z,
            },
            velocity: mint::Vector3 {
                x: velocity.x,
                y: velocity.y,
                z: velocity.z,
            },
            ..Default::default()
        };
        self.handle.play(signal, options)
    }
}

#[cfg(feature = "audio-hrtf")]
impl Default for OddioSpatialScene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_attenuation_fades_to_zero_at_range() {
        assert_eq!(attenuation_gain(0.0, 10.0, AudioRolloff::Linear), 1.0);
        assert_eq!(attenuation_gain(5.0, 10.0, AudioRolloff::Linear), 0.5);
        assert_eq!(attenuation_gain(10.0, 10.0, AudioRolloff::Linear), 0.0);
    }

    #[test]
    fn inverse_attenuation_decreases_with_distance() {
        let near = attenuation_gain(1.0, 10.0, AudioRolloff::Inverse);
        let far = attenuation_gain(5.0, 10.0, AudioRolloff::Inverse);
        assert!(near > far);
        assert!((near - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn logarithmic_attenuation_is_clamped() {
        assert_eq!(attenuation_gain(-1.0, 10.0, AudioRolloff::Logarithmic), 1.0);
        assert_eq!(attenuation_gain(20.0, 10.0, AudioRolloff::Logarithmic), 0.0);
    }

    #[test]
    fn distance_attenuation_matches_expected() {
        let listener = SpatialListener {
            position: Vec3::ZERO,
            ..Default::default()
        };
        let emitter = SpatialEmitter {
            position: Vec3::new(5.0, 0.0, 0.0),
            range: 10.0,
        };
        let gain = listener_emitter_gain(listener, emitter, AudioRolloff::Linear);
        assert!((gain - 0.5).abs() < 1.0e-6);
    }

    #[cfg(feature = "audio-hrtf")]
    #[test]
    fn oddio_spatial_scene_creates() {
        let scene = OddioSpatialScene::new();
        let _ = scene.handle();
    }
}
