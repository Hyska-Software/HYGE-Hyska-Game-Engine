//! Hyge window: `winit` event loop, surface, raw input (Windows), `Window`
//! resource, and `DeviceEvent` translation.
//!
//! The actual `winit::Window` is created from an `ActiveEventLoop` by the
//! application (typically `hyge-app`); this crate provides the
//! [`plugin::WindowPlugin`] that registers events and the
//! [`plugin::WindowState`] resource, the [`config::WindowConfig`] type,
//! the [`window::Window`] wrapper, the event types and the `winit` →
//! Hyge event translation, and the Windows-specific raw input
//! registration.
//!
//! See `docs/architecture.md` §6.9 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-012.
//!
//! ## Unsafe code policy
//!
//! This crate downgrades the workspace's `unsafe_code = "deny"` to a
//! `deny` here (the workspace uses `forbid`). The deviation is necessary
//! because the Windows raw-input registration uses FFI via
//! `windows-sys::Win32::UI::Input::RegisterRawInputDevices`, which is an
//! `extern "system"` function. The single `unsafe` block is confined to
//! `src/raw_input.rs` under `#[cfg(windows)]` and is gated by a
//! `// SAFETY:` comment. All other code in this crate is safe Rust. This
//! is consistent with AGENTS.md §7.7's intent (audit every `unsafe`) —
//! the FFI surface is exactly one FFI call, with a documented invariant.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod events;
pub mod plugin;
pub mod raw_input;
pub mod window;

pub mod prelude;
