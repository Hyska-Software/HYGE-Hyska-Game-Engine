//! Math types built on top of [`glam`].
//!
//! Re-exports the `glam` types we use everywhere (`Vec2/3/4`, `Mat3/4`,
//! `Quat`, integer and unsigned variants) and adds engine-specific
//! geometry types: [`Aabb`], [`Frustum`], [`Plane`], and [`Ray`].

pub use glam::{
    Mat2, Mat3, Mat3A, Mat4, Quat, Vec2, Vec3, Vec3A, Vec4, Vec4A, IVec2, IVec3, IVec4,
    UVec2, UVec3, UVec4,
};

use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

/// Axis-aligned bounding box.
///
/// `min` and `max` are the two opposite corners; the box is the set of all
/// points `p` with `min ≤ p ≤ max` (componentwise). An "empty" box has
/// `min = +∞` and `max = -∞`; [`Aabb::merge`] against an empty box is a
/// no-op against the receiver.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    /// The corner with the smallest coordinates.
    pub min: Vec3,
    /// The corner with the largest coordinates.
    pub max: Vec3,
}

impl Aabb {
    /// The empty AABB. Merging with `EMPTY` is the identity on the receiver.
    pub const EMPTY: Aabb = Aabb {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    /// Constructs an AABB from explicit min and max corners.
    #[inline]
    pub const fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Constructs a degenerate (zero-extent) AABB at a single point.
    #[inline]
    pub const fn from_point(p: Vec3) -> Self {
        Self { min: p, max: p }
    }

    /// Constructs an AABB from a center point and per-axis half-extents.
    #[inline]
    pub fn from_center_half_extents(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            min: center - half_extents,
            max: center + half_extents,
        }
    }

    /// Returns the geometric center of the AABB.
    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Returns the per-axis size (width, height, depth).
    #[inline]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// Returns the per-axis half-extents.
    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        self.size() * 0.5
    }

    /// In-place merge: expands `self` to also contain `other`.
    pub fn merge(&mut self, other: &Aabb) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    /// Returns a new AABB that contains both `self` and `other`.
    #[inline]
    pub fn merged(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Returns true if the AABB contains the given point (inclusive).
    #[inline]
    pub fn contains_point(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    /// Returns true if the AABB contains `other` entirely.
    #[inline]
    pub fn contains_aabb(&self, other: &Aabb) -> bool {
        self.contains_point(other.min) && self.contains_point(other.max)
    }

    /// Returns true if `self` and `other` share at least one point.
    #[inline]
    pub fn intersects_aabb(&self, other: &Aabb) -> bool {
        self.min.cmple(other.max).all() && self.max.cmpge(other.min).all()
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// A plane defined by `dot(normal, p) + d = 0`.
///
/// A point is on the "inside" of the plane (in front of it) when
/// `distance(p) >= 0`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Plane {
    /// The unit (or pre-normalization) normal of the plane.
    pub normal: Vec3,
    /// The signed distance from the origin to the plane along `normal`.
    pub d: f32,
}

impl Plane {
    /// Constructs a plane from a 4-component homogeneous coefficient vector
    /// `(a, b, c, d)` representing `a*x + b*y + c*z + d = 0`. The plane is
    /// normalized in place so that `normal` has unit length.
    pub fn from_vec4(coeff: Vec4) -> Self {
        let mut p = Self {
            normal: Vec3::new(coeff.x, coeff.y, coeff.z),
            d: coeff.w,
        };
        p.normalize();
        p
    }

    /// Signed distance from `p` to the plane. Positive = on the normal side.
    #[inline]
    pub fn distance(&self, p: Vec3) -> f32 {
        self.normal.dot(p) + self.d
    }

    /// Normalizes the plane equation so that `normal` is unit length.
    pub fn normalize(&mut self) {
        let len = self.normal.length();
        if len > 0.0 {
            self.normal /= len;
            self.d /= len;
        }
    }
}

/// A view frustum described by six planes: left, right, bottom, top,
/// near, far (in that order).
#[derive(Copy, Clone, Debug)]
pub struct Frustum {
    /// The six bounding planes. Indices: 0=left, 1=right, 2=bottom, 3=top,
    /// 4=near, 5=far.
    pub planes: [Plane; 6],
}

impl Frustum {
    /// Constructs a frustum from a view-projection matrix using the
    /// Gribb-Hartmann method. Assumes the standard OpenGL/wgpu NDC range
    /// of `[-1, 1]` on all three axes; for Vulkan's `[0, 1]` depth, the
    /// caller must remap `z` before constructing the matrix.
    pub fn from_view_proj(view_proj: Mat4) -> Self {
        let left   = Plane::from_vec4(view_proj.row(3) + view_proj.row(0));
        let right  = Plane::from_vec4(view_proj.row(3) - view_proj.row(0));
        let bottom = Plane::from_vec4(view_proj.row(3) + view_proj.row(1));
        let top    = Plane::from_vec4(view_proj.row(3) - view_proj.row(1));
        let near   = Plane::from_vec4(view_proj.row(3) + view_proj.row(2));
        let far    = Plane::from_vec4(view_proj.row(3) - view_proj.row(2));
        Frustum { planes: [left, right, bottom, top, near, far] }
    }

    /// Returns true if the frustum contains the given point.
    pub fn contains_point(&self, p: Vec3) -> bool {
        self.planes.iter().all(|pl| pl.distance(p) >= 0.0)
    }

    /// Returns true if the AABB intersects (or is fully inside) the frustum.
    ///
    /// Uses the "positive vertex" optimization (Christer Ericson,
    /// *Real-Time Collision Detection* §6.5): for each plane, take the
    /// corner of the AABB farthest in the direction of the plane normal.
    /// If that corner is outside the plane, the AABB is fully outside the
    /// frustum. If every plane's positive vertex is inside, the AABB
    /// intersects (or is fully inside) the frustum.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.planes.iter().all(|pl| {
            let pv = Vec3::select(pl.normal.cmpgt(Vec3::ZERO), aabb.max, aabb.min);
            pl.distance(pv) >= 0.0
        })
    }
}

/// A ray defined by an origin and a unit direction.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Ray {
    /// The starting point of the ray.
    pub origin: Vec3,
    /// The unit-length direction of travel.
    pub direction: Vec3,
}

impl Ray {
    /// Constructs a ray and normalizes `direction` to unit length.
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self { origin, direction: direction.normalize() }
    }

    /// Returns the point at parameter `t` along the ray.
    #[inline]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }

    /// Returns the entry and exit `t` values for the ray against the AABB,
    /// or `None` if the ray misses. The slab-test algorithm is used; if
    /// `t_far < 0`, the AABB is behind the ray and the result is `None`.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> Option<(f32, f32)> {
        let inv_dir = Vec3::new(
            1.0 / self.direction.x,
            1.0 / self.direction.y,
            1.0 / self.direction.z,
        );
        let t1 = (aabb.min - self.origin) * inv_dir;
        let t2 = (aabb.max - self.origin) * inv_dir;
        let tmin = t1.min(t2);
        let tmax = t1.max(t2);
        let t_near = tmin.max_element();
        let t_far = tmax.min_element();
        if t_near <= t_far && t_far >= 0.0 {
            Some((t_near, t_far))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_empty_contains_nothing() {
        let e = Aabb::EMPTY;
        assert!(!e.contains_point(Vec3::ZERO));
        assert!(!e.contains_point(Vec3::splat(1.0e10)));
    }

    #[test]
    fn aabb_from_point_is_degenerate() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        let a = Aabb::from_point(p);
        assert_eq!(a.min, p);
        assert_eq!(a.max, p);
        assert!(a.contains_point(p));
    }

    #[test]
    fn aabb_center_and_size() {
        let a = Aabb::new(Vec3::splat(-2.0), Vec3::splat(4.0));
        assert_eq!(a.center(), Vec3::splat(1.0));
        assert_eq!(a.size(), Vec3::splat(6.0));
        assert_eq!(a.half_extents(), Vec3::splat(3.0));
    }

    #[test]
    fn aabb_merge_in_place() {
        let mut a = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        let b = Aabb::new(Vec3::new(0.0, 0.0, 2.0), Vec3::splat(3.0));
        a.merge(&b);
        assert_eq!(a.min, Vec3::splat(-1.0));
        assert_eq!(a.max, Vec3::splat(3.0));
    }

    #[test]
    fn aabb_merged_returns_new() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        let b = Aabb::new(Vec3::splat(2.0), Vec3::splat(3.0));
        let m = a.merged(&b);
        assert_eq!(m.min, Vec3::ZERO);
        assert_eq!(m.max, Vec3::splat(3.0));
        // Originals unchanged.
        assert_eq!(a, Aabb::new(Vec3::ZERO, Vec3::splat(1.0)));
    }

    #[test]
    fn aabb_merge_with_empty() {
        let mut a = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        let before = a;
        a.merge(&Aabb::EMPTY);
        assert_eq!(a, before);
    }

    #[test]
    fn aabb_contains_and_intersects() {
        let outer = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        let inner = Aabb::new(Vec3::splat(2.0), Vec3::splat(5.0));
        let disjoint = Aabb::new(Vec3::splat(20.0), Vec3::splat(30.0));
        let touching = Aabb::new(Vec3::splat(10.0), Vec3::splat(15.0));
        assert!(outer.contains_aabb(&inner));
        assert!(!inner.contains_aabb(&outer));
        assert!(outer.intersects_aabb(&inner));
        assert!(inner.intersects_aabb(&outer));
        assert!(!outer.intersects_aabb(&disjoint));
        assert!(outer.intersects_aabb(&touching), "touching boxes count as intersecting");
        assert!(outer.contains_point(Vec3::splat(5.0)));
        assert!(!outer.contains_point(Vec3::splat(11.0)));
    }

    #[test]
    fn frustum_from_identity_has_six_planes() {
        let f = Frustum::from_view_proj(Mat4::IDENTITY);
        assert_eq!(f.planes.len(), 6);
    }

    #[test]
    fn ray_at() {
        let r = Ray::new(Vec3::ZERO, Vec3::X);
        assert_eq!(r.at(0.0), Vec3::ZERO);
        assert_eq!(r.at(1.0), Vec3::X);
        assert_eq!(r.at(2.5), Vec3::new(2.5, 0.0, 0.0));
    }

    #[test]
    fn ray_normalizes_direction() {
        let r = Ray::new(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        assert!((r.direction.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ray_hits_aabb() {
        let r = Ray::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::X);
        let a = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        let hit = r.intersects_aabb(&a);
        assert!(hit.is_some(), "ray should hit the box");
        let (t_near, t_far) = hit.unwrap();
        assert!(t_near > 0.0, "t_near should be positive (in front of ray)");
        assert!(t_far > t_near);
    }

    #[test]
    fn ray_misses_aabb_behind() {
        let r = Ray::new(Vec3::new(5.0, 0.0, 0.0), Vec3::X);
        let a = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        assert!(r.intersects_aabb(&a).is_none(), "ray pointing away should miss");
    }

    #[test]
    fn ray_misses_aabb_laterally() {
        let r = Ray::new(Vec3::new(0.0, 5.0, 0.0), Vec3::X);
        let a = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        assert!(r.intersects_aabb(&a).is_none(), "ray passing alongside should miss");
    }
}
