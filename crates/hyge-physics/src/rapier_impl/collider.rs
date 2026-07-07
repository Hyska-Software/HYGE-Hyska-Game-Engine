//! Conversion helpers for Rapier colliders.

use rapier3d::prelude::{ColliderBuilder, SharedShape};

use crate::components::{Collider, ColliderShape};

/// Builds a Rapier collider from a Hyge collider component.
pub fn collider_builder(collider: &Collider) -> ColliderBuilder {
    let mut builder = ColliderBuilder::new(shared_shape(&collider.shape))
        .density(collider.density)
        .friction(collider.friction)
        .restitution(collider.restitution);

    if collider.is_sensor {
        builder = builder.sensor(true);
    }

    builder
}

fn shared_shape(shape: &ColliderShape) -> SharedShape {
    match shape {
        ColliderShape::Ball(radius) => SharedShape::ball(*radius),
        ColliderShape::Cuboid(half_extents) => {
            SharedShape::cuboid(half_extents[0], half_extents[1], half_extents[2])
        }
        ColliderShape::Capsule {
            half_height,
            radius,
        } => SharedShape::capsule_y(*half_height, *radius),
        ColliderShape::Cylinder {
            half_height,
            radius,
        } => SharedShape::cylinder(*half_height, *radius),
        ColliderShape::Cone {
            half_height,
            radius,
        } => SharedShape::cone(*half_height, *radius),
        ColliderShape::ConvexHull(points) => {
            let points = points
                .iter()
                .map(|p| rapier3d::na::Point3::new(p[0], p[1], p[2]))
                .collect::<Vec<_>>();
            SharedShape::convex_hull(&points).unwrap_or_else(|| SharedShape::ball(0.0))
        }
        ColliderShape::Trimesh { vertices, indices } => {
            let vertices = vertices
                .iter()
                .map(|p| rapier3d::na::Point3::new(p[0], p[1], p[2]))
                .collect::<Vec<_>>();
            let indices = indices
                .chunks_exact(3)
                .map(|tri| [tri[0], tri[1], tri[2]])
                .collect::<Vec<_>>();
            SharedShape::trimesh(vertices, indices)
        }
        ColliderShape::Heightfield { heights, scale } => {
            let rows = heights.len().max(1);
            let data = rapier3d::na::DMatrix::from_row_slice(rows, 1, heights);
            SharedShape::heightfield(data, rapier3d::na::Vector3::new(scale[0], 1.0, scale[1]))
        }
    }
}
