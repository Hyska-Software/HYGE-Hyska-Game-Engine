//! Convenience re-exports for `hyge-audio`.

pub use crate::bus::{AudioBusVolumes, AudioBuses, BusKind};
pub use crate::components::{AudioListener, AudioRolloff, AudioSource};
pub use crate::events::{PlaySound, StopSound};
#[cfg(feature = "audio-hrtf")]
pub use crate::hrtf::{HrtfMode, HrtfRenderer};
pub use crate::plugin::{
    audio_event_system, spatial_emitter_sync_system, spatial_listener_sync_system, AudioPlugin,
};
pub use crate::server::{AudioServer, KiraAudioManager, SpatialEmitterHandle};
pub use crate::spatial::{
    attenuation_gain, listener_emitter_gain, SpatialEmitter, SpatialListener,
};

#[cfg(feature = "audio-hrtf")]
pub use crate::spatial::OddioSpatialScene;
