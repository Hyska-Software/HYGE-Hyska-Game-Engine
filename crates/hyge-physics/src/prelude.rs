//! Convenience re-exports for `hyge-physics`.

pub use crate::components::{
    CharacterController, Collider, ColliderShape, PhysicsPosition, PhysicsVelocity, RigidBody,
    RigidBodyKind,
};
pub use crate::config::{PhysicsConfig, PhysicsTime};
pub use crate::events::{CollisionEvent, Contact, ContactForceEvent};
pub use crate::plugin::PhysicsPlugin;
pub use crate::query::{QueryFilter, RayHit, ShapeHit, SpatialQuery, StaticSpatialQuery};
pub use crate::step::{accumulate_fixed_steps, physics_step_system};

#[cfg(feature = "physics-rapier")]
pub use crate::rapier_impl::{self, RapierPhysicsWorld};
