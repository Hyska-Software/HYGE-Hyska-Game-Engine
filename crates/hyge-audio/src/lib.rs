//! Hyge audio: a `kira` backend with a structured bus mixer and spatial 3D.
//!
//! HRTF is gated by the `audio-hrtf` feature flag. When enabled, the spatial
//! audio backend integrates the `oddio::SpatialScene` for spatial mixing and
//! the `hrtf` crate for real HRTF binaural rendering using an HRIR sphere
//! dataset provided at runtime.
//!
//! See `docs/architecture.md` §6.8 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-072..R-073.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bus;
pub mod components;
pub mod events;
#[cfg(feature = "audio-hrtf")]
pub mod hrtf;
pub mod plugin;
pub mod prelude;
pub mod server;
pub mod spatial;

pub use bus::{AudioBusVolumes, AudioBuses, BusKind};
pub use components::{AudioListener, AudioRolloff, AudioSource};
pub use events::{PlaySound, StopSound};
pub use plugin::AudioPlugin;
pub use server::{AudioServer, KiraAudioManager, SpatialEmitterHandle};
pub use spatial::{attenuation_gain, listener_emitter_gain, SpatialEmitter, SpatialListener};

#[cfg(feature = "audio-hrtf")]
pub use hrtf::{HrtfMode, HrtfRenderer};
#[cfg(feature = "audio-hrtf")]
pub use spatial::OddioSpatialScene;
