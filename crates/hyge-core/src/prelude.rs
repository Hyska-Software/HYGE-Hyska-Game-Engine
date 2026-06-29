//! Convenience re-exports for the most-used types in `hyge-core`.
//!
//! `use hyge_core::prelude::*;` brings the engine's foundational data
//! types into scope without naming each one individually.

pub use crate::color::{LinearRGB, Srgb};
pub use crate::id::AssetId;
pub use crate::math::{Aabb, Frustum, Mat3, Mat4, Plane, Quat, Ray, Vec2, Vec3, Vec4};
pub use crate::result::{HygeError, HygeResult};
