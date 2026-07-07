//! ECS components that describe physics bodies, colliders, and controllers.
//!
//! The public methods accept and return `glam` vectors from `hyge-core`, while
//! reflected fields store plain arrays so the editor and scripting layer can
//! inspect them without requiring optional `glam` reflection support.

use bevy_ecs::prelude::Component;
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::Reflect;
use hyge_core::prelude::{Vec2, Vec3};

/// How a [`RigidBody`] participates in the physics simulation.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RigidBodyKind {
    /// Simulated by forces, gravity, and contacts.
    #[default]
    Dynamic,
    /// Immovable body used for static world geometry.
    Fixed,
    /// Kinematic body moved by setting its next position.
    KinematicPosition,
    /// Kinematic body moved by setting its velocity.
    KinematicVelocity,
}

/// Physics body component.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct RigidBody {
    /// Body simulation mode.
    pub kind: RigidBodyKind,
    /// Whether continuous collision detection is enabled.
    pub ccd: bool,
    /// Per-body gravity multiplier.
    pub gravity_scale: f32,
    /// Linear velocity damping coefficient.
    pub linear_damping: f32,
    /// Angular velocity damping coefficient.
    pub angular_damping: f32,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            kind: RigidBodyKind::Dynamic,
            ccd: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
        }
    }
}

/// World-space physics translation used by the Rapier backend.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct PhysicsPosition {
    /// Translation in world space.
    pub translation: [f32; 3],
}

impl PhysicsPosition {
    /// Creates a position from a `glam` vector.
    #[must_use]
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation: translation.into(),
        }
    }

    /// Returns the translation as a `glam` vector.
    #[must_use]
    pub fn as_vec3(&self) -> Vec3 {
        Vec3::from(self.translation)
    }
}

impl Default for PhysicsPosition {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
        }
    }
}

/// Linear and angular velocity written by the physics backend.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct PhysicsVelocity {
    /// Linear velocity in world units per second.
    pub linear: [f32; 3],
    /// Angular velocity in radians per second.
    pub angular: [f32; 3],
}

impl Default for PhysicsVelocity {
    fn default() -> Self {
        Self {
            linear: [0.0; 3],
            angular: [0.0; 3],
        }
    }
}

/// Collision geometry used by a [`Collider`].
#[derive(Reflect, Clone, Debug, PartialEq)]
pub enum ColliderShape {
    /// Sphere with the given radius.
    Ball(f32),
    /// Box with per-axis half extents.
    Cuboid([f32; 3]),
    /// Capsule aligned with the local Y axis.
    Capsule {
        /// Half of the cylindrical section height.
        half_height: f32,
        /// Radius of the capsule caps and cylinder.
        radius: f32,
    },
    /// Cylinder aligned with the local Y axis.
    Cylinder {
        /// Half height of the cylinder.
        half_height: f32,
        /// Radius of the cylinder.
        radius: f32,
    },
    /// Cone aligned with the local Y axis.
    Cone {
        /// Half height of the cone.
        half_height: f32,
        /// Base radius of the cone.
        radius: f32,
    },
    /// Convex hull points in local space.
    ConvexHull(Vec<[f32; 3]>),
    /// Triangle mesh in local space.
    Trimesh {
        /// Mesh vertices in local space.
        vertices: Vec<[f32; 3]>,
        /// Triangle indices, three indices per triangle.
        indices: Vec<u32>,
    },
    /// Heightfield samples with X/Z scale.
    Heightfield {
        /// Row-major height samples.
        heights: Vec<f32>,
        /// X/Z scale applied to the heightfield grid.
        scale: [f32; 2],
    },
}

impl ColliderShape {
    /// Creates a cuboid shape from `glam` half extents.
    #[must_use]
    pub fn cuboid(half_extents: Vec3) -> Self {
        Self::Cuboid(half_extents.into())
    }

    /// Creates a heightfield shape from `glam` scale.
    #[must_use]
    pub fn heightfield(heights: Vec<f32>, scale: Vec2) -> Self {
        Self::Heightfield {
            heights,
            scale: scale.into(),
        }
    }

    /// Returns an approximate local-space AABB half extent for broad-phase tests.
    #[must_use]
    pub fn approximate_half_extents(&self) -> Vec3 {
        match self {
            Self::Ball(radius) => Vec3::splat(*radius),
            Self::Cuboid(half_extents) => Vec3::from(*half_extents),
            Self::Capsule {
                half_height,
                radius,
            } => Vec3::new(*radius, half_height + radius, *radius),
            Self::Cylinder {
                half_height,
                radius,
            }
            | Self::Cone {
                half_height,
                radius,
            } => Vec3::new(*radius, *half_height, *radius),
            Self::ConvexHull(points) => points_half_extents(points),
            Self::Trimesh { vertices, .. } => points_half_extents(vertices),
            Self::Heightfield { heights, scale } => {
                let max_height = heights
                    .iter()
                    .copied()
                    .fold(0.0_f32, |acc, h| acc.max(h.abs()));
                Vec3::new(scale[0], max_height, scale[1])
            }
        }
    }
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Ball(0.5)
    }
}

fn points_half_extents(points: &[[f32; 3]]) -> Vec3 {
    if points.is_empty() {
        return Vec3::ZERO;
    }

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for point in points {
        let point = Vec3::from(*point);
        min = min.min(point);
        max = max.max(point);
    }
    (max - min) * 0.5
}

/// Collider component attached to an entity with or without a [`RigidBody`].
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component)]
pub struct Collider {
    /// Collision shape in local space.
    pub shape: ColliderShape,
    /// Mass density used for dynamic bodies.
    pub density: f32,
    /// Coulomb friction coefficient.
    pub friction: f32,
    /// Restitution coefficient.
    pub restitution: f32,
    /// Sensor colliders emit events but do not generate contact response.
    pub is_sensor: bool,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::default(),
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            is_sensor: false,
        }
    }
}

/// Character controller tuning values.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct CharacterController {
    /// Maximum walkable slope in radians.
    pub max_slope: f32,
    /// Maximum height that can be stepped over.
    pub step_height: f32,
    /// Upward jump impulse or speed, depending on the active backend.
    pub jump: f32,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            max_slope: 45.0_f32.to_radians(),
            step_height: 0.3,
            jump: 5.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::Reflect;

    use super::*;

    fn assert_reflect_round_trip<T>(value: T)
    where
        T: Reflect + Clone + PartialEq + std::fmt::Debug + 'static,
    {
        let reflected = (&value as &dyn Reflect).clone_value();
        let mut restored = value.clone();
        restored.apply(&*reflected);
        assert_eq!(value, restored);
    }

    #[test]
    fn rigid_body_reflect_round_trip() {
        let body = RigidBody {
            kind: RigidBodyKind::KinematicVelocity,
            ccd: true,
            gravity_scale: 0.25,
            linear_damping: 0.5,
            angular_damping: 0.75,
        };

        assert_reflect_round_trip(body);
    }

    #[test]
    fn collider_reflect_round_trip() {
        let collider = Collider {
            shape: ColliderShape::Capsule {
                half_height: 1.25,
                radius: 0.4,
            },
            density: 2.0,
            friction: 0.8,
            restitution: 0.15,
            is_sensor: true,
        };

        assert_reflect_round_trip(collider);
    }

    #[test]
    fn character_controller_reflect_round_trip() {
        let controller = CharacterController {
            max_slope: 0.6,
            step_height: 0.45,
            jump: 7.0,
        };

        assert_reflect_round_trip(controller);
    }

    #[test]
    fn physics_position_reflect_round_trip() {
        assert_reflect_round_trip(PhysicsPosition::from_translation(Vec3::new(1.0, 2.0, 3.0)));
    }

    #[test]
    fn physics_velocity_reflect_round_trip() {
        assert_reflect_round_trip(PhysicsVelocity {
            linear: [1.0, 2.0, 3.0],
            angular: [4.0, 5.0, 6.0],
        });
    }

    #[test]
    fn collider_shape_has_required_variants() {
        let shapes = [
            ColliderShape::Ball(1.0),
            ColliderShape::Cuboid([1.0, 2.0, 3.0]),
            ColliderShape::Capsule {
                half_height: 1.0,
                radius: 0.5,
            },
            ColliderShape::Cylinder {
                half_height: 1.0,
                radius: 0.5,
            },
            ColliderShape::Cone {
                half_height: 1.0,
                radius: 0.5,
            },
            ColliderShape::ConvexHull(vec![[0.0, 0.0, 0.0]]),
            ColliderShape::Trimesh {
                vertices: vec![[0.0, 0.0, 0.0]],
                indices: vec![0],
            },
            ColliderShape::Heightfield {
                heights: vec![0.0],
                scale: [1.0, 1.0],
            },
        ];
        assert_eq!(shapes.len(), 8);
    }
}
