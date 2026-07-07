//! Convenience re-exports for `hyge-audio`.

pub use crate::bus::{AudioBusVolumes, AudioBuses, BusKind};
pub use crate::components::{AudioListener, AudioRolloff, AudioSource};
pub use crate::events::{PlaySound, StopSound};
#[cfg(feature = "audio-hrtf")]
pub use crate::hrtf;
pub use crate::plugin::{audio_event_system, audio_update_system, AudioPlugin};
pub use crate::server::{AudioServer, KiraAudioManager};
pub use crate::spatial::{
    attenuation_gain, listener_emitter_gain, SpatialEmitter, SpatialListener,
};
