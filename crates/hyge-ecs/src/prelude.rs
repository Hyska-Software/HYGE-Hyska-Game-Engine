//! Convenience re-exports for `hyge-ecs`.
//!
//! `use hyge_ecs::prelude::*;` brings the full ECS surface (from
//! `bevy_ecs`), the task pools (from `bevy_tasks`), reflection (from
//! `bevy_reflect`), and Hyge's own plugin / schedule / system-set types
//! into scope.

// Hyge-specific types.
pub use crate::plugin::{AppHygeExt, HygePlugin};
pub use crate::schedule::Label;
pub use crate::set::{
    AssetSet, AudioSet, EditorSet, InputSet, PhysicsSet, ScriptSet, TransformSet,
};

// Re-export the bevy ECS surface so downstream crates never need to name
// `bevy_ecs` directly (ADR-0002).
pub use bevy_ecs::prelude::*;
pub use bevy_reflect::Reflect;
pub use bevy_tasks::{AsyncComputeTaskPool, IoTaskPool, Task, TaskPool};
