//! Conversion helpers for Rapier rigid bodies.

use rapier3d::prelude::RigidBodyType;

use crate::components::RigidBodyKind;

/// Converts a Hyge rigid body kind to Rapier's body type.
#[must_use]
pub fn rigid_body_type(kind: RigidBodyKind) -> RigidBodyType {
    match kind {
        RigidBodyKind::Dynamic => RigidBodyType::Dynamic,
        RigidBodyKind::Fixed => RigidBodyType::Fixed,
        RigidBodyKind::KinematicPosition => RigidBodyType::KinematicPositionBased,
        RigidBodyKind::KinematicVelocity => RigidBodyType::KinematicVelocityBased,
    }
}
