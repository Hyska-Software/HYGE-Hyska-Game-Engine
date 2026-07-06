//! R-043 ECS components: the minimum catalog a frame's
//! `RenderExtract` system needs to walk a world and produce a
//! `FrameSnapshot` for the renderer.
//!
//! The component types deliberately live in `hyge-scene` (not in
//! `hyge-render`) so the render module does not need to depend
//! on `bevy_ecs`. The render-side mirrors live in
//! `hyge-render::clustered_forward` as POD
//! `Instance`/`DrawCommand`/`Light` structs; the extract system
//! (TBD in R-043) walks the ECS and writes those POD structs.

use bytemuck::{Pod, Zeroable};
use hyge_ecs::prelude::Component;

/// The bindless mesh id (slot index in the bindless table).
/// Mirrors the `BindlessSlot<MeshTag>` index exposed by
/// `hyge_render::bindless`; stored on the ECS as a plain `u32`
/// so the scene does not need to import the render type.
#[derive(Component, Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

/// The bindless material id.
#[derive(Component, Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialHandle(pub u32);

/// World-space transform of a renderable entity. Three rows of
/// a column-major affine matrix; the fourth component is the
/// `w` of the homogeneous row and is left at `1.0`.
#[repr(C)]
#[derive(Component, Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct WorldTransform {
    /// Column-major 3x4 affine matrix.
    pub cols: [[f32; 4]; 3],
}

impl WorldTransform {
    /// Builds an identity transform.
    #[must_use]
    pub fn identity() -> Self {
        Self {
            cols: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        }
    }

    /// Builds a translation-only transform.
    #[must_use]
    pub fn from_translation(x: f32, y: f32, z: f32) -> Self {
        Self {
            cols: [
                [1.0, 0.0, 0.0, x],
                [0.0, 1.0, 0.0, y],
                [0.0, 0.0, 1.0, z],
            ],
        }
    }
}

/// A directional light, point light, or spot light attached to
/// the scene. The packed `Light` GPU type is the render-side
/// mirror; this component is the ECS source of truth.
#[derive(Component, Copy, Clone, Debug, Default)]
pub struct LightComponent {
    /// World-space position. `w` is light type (0=point, 1=spot,
    /// 2=directional).
    pub position: [f32; 4],
    /// RGB color in `xyz`, scalar intensity in `w`.
    pub color_intensity: [f32; 4],
    /// Direction (xyz) for spot/directional, cos(outer_angle) in `w`.
    pub direction_cos_outer: [f32; 4],
}

impl LightComponent {
    /// Builds a directional sun.
    #[must_use]
    pub fn sun(direction: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            position: [0.0, 0.0, 0.0, 2.0],
            color_intensity: [color[0], color[1], color[2], intensity],
            direction_cos_outer: [direction[0], direction[1], direction[2], 0.0],
        }
    }

    /// Builds a point light at `position`.
    #[must_use]
    pub fn point(position: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            position: [position[0], position[1], position[2], 0.0],
            color_intensity: [color[0], color[1], color[2], intensity],
            direction_cos_outer: [0.0, -1.0, 0.0, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_transform_is_pod() {
        let t = WorldTransform::identity();
        let bytes = bytemuck::bytes_of(&t);
        assert_eq!(bytes.len(), std::mem::size_of::<WorldTransform>());
    }

    #[test]
    fn translation_lands_in_w_components() {
        let t = WorldTransform::from_translation(1.0, 2.0, 3.0);
        assert_eq!(t.cols[0], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(t.cols[1], [0.0, 1.0, 0.0, 2.0]);
        assert_eq!(t.cols[2], [0.0, 0.0, 1.0, 3.0]);
    }

    #[test]
    fn sun_packs_type_in_w() {
        let s = LightComponent::sun([0.0, -1.0, 0.0], [1.0, 0.95, 0.9], 1.5);
        assert_eq!(s.position[3], 2.0);
        assert_eq!(s.color_intensity[3], 1.5);
    }

    #[test]
    fn point_packs_type_zero_in_w() {
        let p = LightComponent::point([0.0, 5.0, 0.0], [1.0, 0.0, 0.0], 2.0);
        assert_eq!(p.position[3], 0.0);
        assert_eq!(p.position[0], 0.0);
        assert_eq!(p.position[1], 5.0);
    }
}
