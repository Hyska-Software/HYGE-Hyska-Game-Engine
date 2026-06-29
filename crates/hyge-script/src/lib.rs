//! Hyge script: a Lua runtime via `mlua` integrated with the ECS through
//! `bevy_reflect`.
//!
//! Provides a sandbox (strips `os`, `io`, `debug`, `package`, raw `require`),
//! a typed `hyge.*` API surface generated from `Reflect` metadata, and
//! hot-reload with an `on_reload(old_state)` hook for state preservation.
//!
//! WASM via `wasmtime` is deferred to v0.2 (see `docs/architecture.md` §17).
//!
//! See `docs/architecture.md` §6.11 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-076..R-077.
