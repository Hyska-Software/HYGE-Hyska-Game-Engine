//! Skeletal animation and skinning helpers.

use hyge_core::prelude::{Mat4, Vec3};

/// CPU-side source vertex consumed by the skinning compute pass.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SkinnedVertex {
    /// Local bind-pose position.
    pub position: Vec3,
    /// Local bind-pose normal.
    pub normal: Vec3,
    /// Four joint indices.
    pub joint_indices: [u32; 4],
    /// Four normalized joint weights.
    pub joint_weights: [f32; 4],
}

/// Output of skinning one vertex.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SkinnedVertexOutput {
    /// Skinned position.
    pub position: Vec3,
    /// Skinned normal.
    pub normal: Vec3,
}

/// Runtime animation component state.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct SkeletalAnimation {
    /// Clip id in the animation asset.
    pub clip: u32,
    /// Current clip time in seconds.
    pub time: f32,
    /// Whether sampled root motion should be extracted.
    pub root_motion: bool,
}

/// Skins one vertex with up to four influences.
#[must_use]
pub fn skin_vertex(vertex: &SkinnedVertex, joint_matrices: &[Mat4]) -> SkinnedVertexOutput {
    let mut position = Vec3::ZERO;
    let mut normal = Vec3::ZERO;
    for i in 0..4 {
        let weight = vertex.joint_weights[i];
        if weight <= 0.0 {
            continue;
        }
        let Some(matrix) = joint_matrices.get(vertex.joint_indices[i] as usize) else {
            continue;
        };
        position += matrix.transform_point3(vertex.position) * weight;
        normal += matrix.transform_vector3(vertex.normal) * weight;
    }
    SkinnedVertexOutput {
        position,
        normal: normal.try_normalize().unwrap_or(vertex.normal),
    }
}

/// Skins a vertex slice into a new output vector.
#[must_use]
pub fn skin_vertices(
    vertices: &[SkinnedVertex],
    joint_matrices: &[Mat4],
) -> Vec<SkinnedVertexOutput> {
    vertices
        .iter()
        .map(|vertex| skin_vertex(vertex, joint_matrices))
        .collect()
}
