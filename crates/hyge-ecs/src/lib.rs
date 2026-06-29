//! Hyge ECS: the only crate that imports `bevy_ecs` directly.
//!
//! Re-exports `bevy_ecs::prelude`, defines the `HygePlugin` trait, the
//! `schedule::Label` enum (First, PreUpdate, FixedUpdate, Update,
//! RenderExtract, Render, Last), and the `set::*` enums for cross-crate
//! system ordering.
//!
//! See `docs/architecture.md` §6.2 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-011.
