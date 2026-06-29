//! Hyge window: `winit` event loop, surface, raw input (Windows), `Window`
//! resource, and `DeviceEvent` translation.
//!
//! The actual `winit::Window` is created from an `ActiveEventLoop` by the
//! application (typically `hyge-app`); this crate provides the
//! [`WindowPlugin`] that registers events and the [`WindowState`]
//! resource, the [`WindowConfig`] type, the [`Window`] wrapper, the event
//! types and the `winit` → Hyge event translation, and the
//! Windows-specific raw input registration.
//!
//! See `docs/architecture.md` §6.9 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-012.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod events;
pub mod plugin;
pub mod raw_input;
pub mod window;

pub mod prelude;
