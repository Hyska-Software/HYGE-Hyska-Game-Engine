//! Audio command events.

use bevy_ecs::prelude::{Entity, Event};

/// Request to play an audio source.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct PlaySound {
    /// Entity carrying an [`AudioSource`](crate::components::AudioSource).
    pub source: Entity,
    /// Event-local volume multiplier.
    pub volume: f32,
    /// Event-local pitch multiplier.
    pub pitch: f32,
}

/// Request to stop an audio source.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopSound {
    /// Entity carrying an [`AudioSource`](crate::components::AudioSource).
    pub source: Entity,
}
