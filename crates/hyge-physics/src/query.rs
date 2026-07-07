//! Spatial query traits and a small deterministic static query implementation.

use bevy_ecs::prelude::Entity;
use hyge_core::prelude::{Aabb, Quat, Ray, Vec3};

use crate::components::ColliderShape;

/// Entity-level filter used by spatial queries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryFilter {
    /// Entities to skip during the query.
    pub excluded_entities: Vec<Entity>,
}

impl QueryFilter {
    /// Returns a filter that excludes no entities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when `entity` should be considered by the query.
    #[must_use]
    pub fn allows(&self, entity: Entity) -> bool {
        !self.excluded_entities.contains(&entity)
    }
}

/// Raycast hit information.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// Hit entity.
    pub entity: Entity,
    /// Time of impact along the ray direction.
    pub toi: f32,
    /// Approximate world-space surface normal at the hit point.
    pub normal: Vec3,
}

/// Shape-cast hit information.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeHit {
    /// Hit entity.
    pub entity: Entity,
    /// Time of impact along the cast direction.
    pub toi: f32,
    /// Witness point on the cast shape.
    pub witness1: Vec3,
    /// Witness point on the hit shape.
    pub witness2: Vec3,
    /// Approximate world-space hit normal.
    pub normal: Vec3,
}

/// Trait implemented by physics backends that can answer spatial queries.
pub trait SpatialQuery {
    /// Casts a ray and returns the nearest hit within `max_toi`.
    fn cast_ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_toi: f32,
        filter: QueryFilter,
    ) -> Option<RayHit>;

    /// Casts a collider shape and returns the nearest hit within `max_toi`.
    fn cast_shape(
        &self,
        shape: ColliderShape,
        origin: Vec3,
        dir: Vec3,
        max_toi: f32,
        filter: QueryFilter,
    ) -> Option<ShapeHit>;

    /// Returns all entities intersecting `shape` at `origin` and `rotation`.
    fn intersections_with(
        &self,
        shape: ColliderShape,
        origin: Vec3,
        rotation: Quat,
        filter: QueryFilter,
    ) -> Vec<Entity>;
}

/// Deterministic static AABB query implementation used by tests and tools.
#[derive(Clone, Debug, Default)]
pub struct StaticSpatialQuery {
    colliders: Vec<StaticCollider>,
}

#[derive(Clone, Copy, Debug)]
struct StaticCollider {
    entity: Entity,
    aabb: Aabb,
}

impl StaticSpatialQuery {
    /// Creates an empty static spatial query set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a static box represented by center and half extents.
    pub fn add_static_box(&mut self, entity: Entity, center: Vec3, half_extents: Vec3) {
        self.colliders.push(StaticCollider {
            entity,
            aabb: Aabb::from_center_half_extents(center, half_extents),
        });
    }
}

impl SpatialQuery for StaticSpatialQuery {
    fn cast_ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_toi: f32,
        filter: QueryFilter,
    ) -> Option<RayHit> {
        if dir.length_squared() == 0.0 || max_toi < 0.0 {
            return None;
        }

        let ray = Ray::new(origin, dir);
        self.colliders
            .iter()
            .filter(|collider| filter.allows(collider.entity))
            .filter_map(|collider| {
                ray.intersects_aabb(&collider.aabb).and_then(|(near, far)| {
                    let toi = if near >= 0.0 { near } else { far };
                    (toi <= max_toi).then(|| RayHit {
                        entity: collider.entity,
                        toi,
                        normal: aabb_hit_normal(ray.at(toi), &collider.aabb),
                    })
                })
            })
            .min_by(|a, b| a.toi.total_cmp(&b.toi))
    }

    fn cast_shape(
        &self,
        shape: ColliderShape,
        origin: Vec3,
        dir: Vec3,
        max_toi: f32,
        filter: QueryFilter,
    ) -> Option<ShapeHit> {
        let half_extents = shape.approximate_half_extents();
        let mut expanded = Self::new();
        for collider in &self.colliders {
            let expanded_aabb = Aabb::from_center_half_extents(
                collider.aabb.center(),
                collider.aabb.half_extents() + half_extents,
            );
            expanded.colliders.push(StaticCollider {
                entity: collider.entity,
                aabb: expanded_aabb,
            });
        }

        expanded
            .cast_ray(origin, dir, max_toi, filter)
            .map(|hit| ShapeHit {
                entity: hit.entity,
                toi: hit.toi,
                witness1: origin + dir.normalize() * hit.toi,
                witness2: origin + dir.normalize() * hit.toi - hit.normal * half_extents.length(),
                normal: hit.normal,
            })
    }

    fn intersections_with(
        &self,
        shape: ColliderShape,
        origin: Vec3,
        _rotation: Quat,
        filter: QueryFilter,
    ) -> Vec<Entity> {
        let query_aabb = Aabb::from_center_half_extents(origin, shape.approximate_half_extents());
        self.colliders
            .iter()
            .filter(|collider| filter.allows(collider.entity))
            .filter(|collider| collider.aabb.intersects_aabb(&query_aabb))
            .map(|collider| collider.entity)
            .collect()
    }
}

fn aabb_hit_normal(point: Vec3, aabb: &Aabb) -> Vec3 {
    let distances = [
        (Vec3::NEG_X, (point.x - aabb.min.x).abs()),
        (Vec3::X, (point.x - aabb.max.x).abs()),
        (Vec3::NEG_Y, (point.y - aabb.min.y).abs()),
        (Vec3::Y, (point.y - aabb.max.y).abs()),
        (Vec3::NEG_Z, (point.z - aabb.min.z).abs()),
        (Vec3::Z, (point.z - aabb.max.z).abs()),
    ];

    distances
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(Vec3::ZERO, |(normal, _)| normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raycast_hits_static_box() {
        let mut query = StaticSpatialQuery::new();
        let entity = Entity::from_raw(42);
        query.add_static_box(entity, Vec3::ZERO, Vec3::splat(1.0));

        let hit = query
            .cast_ray(
                Vec3::new(-5.0, 0.0, 0.0),
                Vec3::X,
                10.0,
                QueryFilter::default(),
            )
            .expect("ray should hit the static box");

        assert_eq!(hit.entity, entity);
        assert!((hit.toi - 4.0).abs() < 1.0e-5);
        assert_eq!(hit.normal, Vec3::NEG_X);
    }

    #[test]
    fn raycast_filter_excludes_entity() {
        let mut query = StaticSpatialQuery::new();
        let entity = Entity::from_raw(7);
        query.add_static_box(entity, Vec3::ZERO, Vec3::splat(1.0));

        let hit = query.cast_ray(
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::X,
            10.0,
            QueryFilter {
                excluded_entities: vec![entity],
            },
        );

        assert!(hit.is_none());
    }

    #[test]
    fn intersections_return_overlapping_entities() {
        let mut query = StaticSpatialQuery::new();
        let near = Entity::from_raw(1);
        let far = Entity::from_raw(2);
        query.add_static_box(near, Vec3::ZERO, Vec3::splat(1.0));
        query.add_static_box(far, Vec3::splat(10.0), Vec3::splat(1.0));

        let hits = query.intersections_with(
            ColliderShape::Ball(1.0),
            Vec3::ZERO,
            Quat::IDENTITY,
            QueryFilter::default(),
        );

        assert_eq!(hits, vec![near]);
    }
}
