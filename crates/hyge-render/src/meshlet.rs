//! GPU meshlet culling data structures and deterministic CPU mirror.

use hyge_core::prelude::{Aabb, Mat4};

use crate::cull::{transform_aabb, SimpleFrustum};

/// Bounds and LOD metadata for one meshlet.
#[derive(Copy, Clone, Debug)]
pub struct MeshletBounds {
    /// Mesh id in the bindless table.
    pub mesh_id: u32,
    /// Meshlet id inside the mesh.
    pub meshlet_id: u32,
    /// Local meshlet bounds.
    pub bounds: Aabb,
    /// World transform for this meshlet instance.
    pub transform: Mat4,
    /// Screen-space error estimate.
    pub screen_error: f32,
}

/// Visibility record matching the bindless meshlet-visibility concept.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct VisibleMeshlet {
    /// Mesh id in the bindless table.
    pub mesh_id: u32,
    /// Meshlet id inside the mesh.
    pub meshlet_id: u32,
    /// Selected LOD level.
    pub lod: u32,
}

/// Mirrors the GPU culling shader on CPU for tests and validation.
#[must_use]
pub fn cull_and_select_lod(
    frustum: &SimpleFrustum,
    meshlets: &[MeshletBounds],
    lod_error_threshold: f32,
) -> Vec<VisibleMeshlet> {
    meshlets
        .iter()
        .filter_map(|meshlet| {
            let world = transform_aabb(meshlet.bounds, meshlet.transform);
            if !frustum.intersects_aabb(&world) {
                return None;
            }
            let lod = u32::from(meshlet.screen_error <= lod_error_threshold);
            Some(VisibleMeshlet {
                mesh_id: meshlet.mesh_id,
                meshlet_id: meshlet.meshlet_id,
                lod,
            })
        })
        .collect()
}
