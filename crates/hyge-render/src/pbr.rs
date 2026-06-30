//! PBR shader contract for the clustered-forward renderer (R-040).
//!
//! This module exposes the WGSL source and the CPU-side constants that
//! mirror the shader ABI. The full clustered-forward render pass lands
//! in R-042/R-043; R-040 establishes the shader itself: bindless mesh /
//! material / instance / draw access, GGX metallic-roughness shading,
//! IBL sampling, emissive contribution, and alpha-mode handling.

/// WGSL source for the PBR pass.
pub const SHADER_SOURCE: &str = include_str!("shader/pbr.wgsl");

/// Opaque material alpha mode.
pub const ALPHA_MODE_OPAQUE: u32 = 0;
/// Alpha-test / cutout material alpha mode.
pub const ALPHA_MODE_CUTOUT: u32 = 1;
/// Alpha-blended material alpha mode.
pub const ALPHA_MODE_BLEND: u32 = 2;

/// `GpuMaterial::flags` bit indicating that the emissive texture slot
/// should be sampled and added to the final radiance.
pub const MATERIAL_FLAG_EMISSIVE_MAP: u32 = 1 << 0;

/// Byte stride of `PbrPackedVertex` in `shader/pbr.wgsl`.
///
/// The shader packs one vertex as three `vec4<f32>` records:
/// position.xyz + normal.x, normal.yz + tangent.xy, tangent.zw + uv.xy.
/// `GpuMesh::vertex_offset` is a byte offset, so the vertex shader divides
/// by this value before indexing the storage-buffer vertex array.
pub const PBR_PACKED_VERTEX_STRIDE_BYTES: u32 = 48;
