//! Hyge physics: ECS physics components, events, queries, and optional
//! `rapier3d` integration behind the `physics-rapier` feature flag.
//!
//! Exposes `RigidBody`, `Collider`, `CharacterController`, `Joint` components;
//! `CollisionEvent` / `ContactForceEvent` events; and a `SpatialQuery` trait
//! for raycasts and shape casts.
//!
//! See `docs/architecture.md` §6.7 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-070..R-071.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod components;
pub mod config;
pub mod events;
pub mod plugin;
pub mod prelude;
pub mod query;
pub mod step;

#[cfg(feature = "physics-rapier")]
pub mod rapier_impl;

pub use components::{CharacterController, Collider, ColliderShape, RigidBody, RigidBodyKind};
pub use config::{PhysicsConfig, PhysicsTime};
pub use events::{CollisionEvent, Contact, ContactForceEvent};
pub use plugin::PhysicsPlugin;
pub use query::{QueryFilter, RayHit, ShapeHit, SpatialQuery, StaticSpatialQuery};
