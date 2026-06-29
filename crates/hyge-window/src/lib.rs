//! Hyge window: `winit` event loop, surface management, and raw input
//! (Windows: `RegisterRawInputDevices` via `windows-sys`).
//!
//! Translates platform events into ECS-friendly `WindowResized`,
//! `WindowCloseRequested`, `WindowFocused`, and `DeviceEvent` events.
//!
//! See `docs/architecture.md` §6.9 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-012.
