//! Hyge ECS: the only crate that imports `bevy_ecs` directly.
//!
//! Re-exports `bevy_ecs::prelude`, `bevy_tasks`, and `bevy_reflect` so that
//! every other Hyge crate can depend on `hyge-ecs` instead of naming
//! `bevy_ecs` itself. Defines the [`HygePlugin`](plugin::HygePlugin) trait,
//! the [`Label`](schedule::Label) schedule enum, and the
//! [`SystemSet`](set) enums for cross-crate system ordering.
//!
//! See `docs/architecture.md` §6.2 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-011.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Re-export bevy_ecs, bevy_tasks, and bevy_reflect at the crate root so
// downstream crates can `use hyge_ecs::*;` and get the full ECS surface
// without ever naming `bevy_ecs` directly. This is the single chokepoint
// that lets us swap the ECS implementation later (ADR-0002).
pub use bevy_ecs::prelude::*;
pub use bevy_reflect::Reflect;
pub use bevy_tasks::{AsyncComputeTaskPool, IoTaskPool, Task, TaskPool};

pub mod plugin;
pub mod prelude;
pub mod schedule;
pub mod set;
