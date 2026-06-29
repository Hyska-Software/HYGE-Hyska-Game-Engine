//! Hyge input: `Action<T>` abstraction (Button, Axis, Vec2) with TOML-defined
//! bindings and hot-reload via `notify`.
//!
//! Reads `DeviceEvent` from `hyge-window` and gamepad events from `gilrs`,
//! aggregates per binding, and exposes the resulting `ActionMap` to game code.
//!
//! See `docs/architecture.md` §6.10 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-074..R-075.
