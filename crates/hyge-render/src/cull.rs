//! CPU frustum culling helpers for render extraction.

use hyge_core::prelude::{Aabb, Frustum, Mat4, Vec3};

/// A frustum wrapper used by phase-5 culling tests and extraction.
#[derive(Copy, Clone, Debug)]
pub struct SimpleFrustum {
    frustum: Frustum,
}

impl SimpleFrustum {
    /// Builds a frustum from a view-projection matrix.
    #[must_use]
    pub fn from_view_proj(view_proj: Mat4) -> Self {
        Self {
            frustum: Frustum::from_view_proj(view_proj),
        }
    }

    /// Builds an orthographic frustum in world space.
    #[must_use]
    pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        Self::from_view_proj(Mat4::orthographic_rh(left, right, bottom, top, near, far))
    }

    /// Returns true when `aabb` intersects this frustum.
    #[must_use]
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.frustum.intersects_aabb(aabb)
    }
}

/// One cullable instance with local bounds and a world transform.
#[derive(Copy, Clone, Debug)]
pub struct CullInstance {
    /// Local-space bounds.
    pub local_bounds: Aabb,
    /// World transform.
    pub transform: Mat4,
}

impl CullInstance {
    /// Creates a cullable instance.
    #[must_use]
    pub const fn new(local_bounds: Aabb, transform: Mat4) -> Self {
        Self {
            local_bounds,
            transform,
        }
    }

    /// Computes world-space bounds by transforming all eight corners.
    #[must_use]
    pub fn world_bounds(&self) -> Aabb {
        transform_aabb(self.local_bounds, self.transform)
    }
}

/// Transforms an AABB by an arbitrary affine transform.
#[must_use]
pub fn transform_aabb(aabb: Aabb, transform: Mat4) -> Aabb {
    let corners = [
        Vec3::new(aabb.min.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.max.z),
    ];
    let mut out = Aabb::EMPTY;
    for corner in corners {
        let p = transform.transform_point3(corner);
        out.merge(&Aabb::from_point(p));
    }
    out
}

/// Returns the visible instance indices after frustum culling.
#[must_use]
pub fn cull_instances(frustum: &SimpleFrustum, instances: &[CullInstance]) -> Vec<usize> {
    instances
        .iter()
        .enumerate()
        .filter_map(|(idx, instance)| {
            frustum
                .intersects_aabb(&instance.world_bounds())
                .then_some(idx)
        })
        .collect()
}
