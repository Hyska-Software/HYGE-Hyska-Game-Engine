//! PBR shader contract for the clustered-forward renderer (R-040).
//!
//! This module exposes the WGSL source and the CPU-side constants that
//! mirror the shader ABI. The full clustered-forward render pass lands
//! in R-042/R-043; R-040 establishes the shader itself: bindless mesh /
//! material / instance / draw access, GGX metallic-roughness shading,
//! IBL sampling, emissive contribution, and alpha-mode handling.

/// WGSL source for the PBR pass.
pub const SHADER_SOURCE: &str = include_str!("shader/pbr.wgsl");

/// WGSL source for the IBL prefilter reference compute shader
/// (R-041). naga-validated. The CPU bake in
/// [`crate::ibl::prefilter_env`] is the source of truth; this
/// shader exists for the future online re-bake compute path.
pub const PREFILTER_SHADER_SOURCE: &str = include_str!("shader/prefilter.wgsl");

/// WGSL source for the IBL irradiance reference compute shader
/// (R-041). naga-validated. The CPU bake in
/// [`crate::ibl::diffuse_irradiance`] is the source of truth.
pub const IRRADIANCE_SHADER_SOURCE: &str = include_str!("shader/irradiance.wgsl");

/// WGSL source for the R-042 clustered-forward light-grid compute
/// shader. naga-validated. The CPU-side grid build in
/// [`crate::clustered_forward::ClusteredForwardPass`] is the source
/// of truth; this shader exists for the future GPU-driven culling
/// path and for naga validation.
pub const LIGHT_GRID_SHADER_SOURCE: &str = include_str!("shader/light_grid.wgsl");

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

/// Maximum roughness LOD the PBR shader samples from the
/// prefiltered environment cubemap. R-041 lifted the base
/// cubemap size from the R-040 contract (32) to 256, which
/// yields 9 mips; the shader's
/// `textureSampleLevel(env, ..., roughness * MAX_LOD)` formula
/// therefore needs `MAX_LOD = 8.0` to cover the full chain.
///
/// The CPU-side equivalent lives at
/// [`crate::ibl::PREFILTERED_ENV_MAX_LOD`].
pub const PREFILTERED_ENV_MAX_LOD: f32 = 8.0;
