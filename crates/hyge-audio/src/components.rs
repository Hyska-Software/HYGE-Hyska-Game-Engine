//! ECS components for audio emitters and listeners.

use bevy_ecs::prelude::Component;
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::Reflect;

use crate::bus::BusKind;

/// Distance attenuation model for spatial sources.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AudioRolloff {
    /// Linear fade from 1 to 0 across source range.
    #[default]
    Linear,
    /// Inverse distance attenuation.
    Inverse,
    /// Logarithmic attenuation curve.
    Logarithmic,
}

/// Audio source component.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component)]
pub struct AudioSource {
    /// Asset path or clip identifier.
    pub clip: String,
    /// Target bus.
    pub bus: BusKind,
    /// Whether this source is spatialized.
    pub spatial: bool,
    /// Source local volume.
    pub volume: f32,
    /// Pitch multiplier.
    pub pitch: f32,
    /// Maximum audible range for spatial attenuation.
    pub range: f32,
    /// Distance attenuation curve.
    pub rolloff: AudioRolloff,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self {
            clip: String::new(),
            bus: BusKind::Sfx,
            spatial: false,
            volume: 1.0,
            pitch: 1.0,
            range: 25.0,
            rolloff: AudioRolloff::Linear,
        }
    }
}

/// Audio listener component.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct AudioListener {
    /// Listener position in world space.
    pub position: [f32; 3],
    /// Listener forward vector.
    pub forward: [f32; 3],
    /// Listener up vector.
    pub up: [f32; 3],
}

impl Default for AudioListener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::Reflect;

    use super::*;

    fn assert_reflect_round_trip<T>(value: T)
    where
        T: Reflect + Clone + PartialEq + std::fmt::Debug + 'static,
    {
        let reflected = (&value as &dyn Reflect).clone_value();
        let mut restored = value.clone();
        restored.apply(&*reflected);
        assert_eq!(value, restored);
    }

    #[test]
    fn audio_source_reflect_round_trip() {
        assert_reflect_round_trip(AudioSource {
            clip: "sfx/hit.ogg".to_string(),
            spatial: true,
            rolloff: AudioRolloff::Inverse,
            ..AudioSource::default()
        });
    }

    #[test]
    fn audio_listener_reflect_round_trip() {
        assert_reflect_round_trip(AudioListener::default());
    }
}
