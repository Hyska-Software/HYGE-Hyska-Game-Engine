//! Events emitted by the physics backend.

use bevy_ecs::prelude::{Entity, Event};
use hyge_core::prelude::Vec3;

/// Contact point information for a collision pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// World-space contact point.
    pub point: Vec3,
    /// World-space contact normal from `entity_a` toward `entity_b`.
    pub normal: Vec3,
    /// Penetration depth in world units.
    pub penetration: f32,
}

/// Collision start/stop event for a pair of entities.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct CollisionEvent {
    /// First entity in the collision pair.
    pub entity_a: Entity,
    /// Second entity in the collision pair.
    pub entity_b: Entity,
    /// `true` when the collision starts, `false` when it stops.
    pub started: bool,
    /// Optional representative contact from the collision manifold.
    pub contact: Option<Contact>,
}

/// Contact force event for a pair of touching entities.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct ContactForceEvent {
    /// First entity in the contact pair.
    pub entity_a: Entity,
    /// Second entity in the contact pair.
    pub entity_b: Entity,
    /// Total force applied at the contact manifold.
    pub total_force: Vec3,
    /// Total torque applied at the contact manifold.
    pub total_torque: Vec3,
}
