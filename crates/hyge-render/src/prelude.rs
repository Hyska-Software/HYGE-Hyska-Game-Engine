//! The renderer prelude.
//!
//! `use hyge_render::prelude::*;` brings the public types into
//! scope without naming them individually.

pub use crate::bindless::{
    pod_collect_to_vec, BindlessConfig, BindlessSlot, BindlessTable, DrawCommand,
    DrawCommand as DrawCmd, GpuMaterial, GpuMesh, Instance, Light, LightGrid, MaterialId, MeshId,
    MeshletVisibility, Refcount, SlotKind, SlotTag, TextureId,
};
pub use crate::config::RendererConfig;
pub use crate::ibl::{
    bake_from_rgbe_hdr, decode_rgbe_hdr, env_file_hash, read_env_file, write_env_file, BrdfLut,
    EnvironmentBake, IrradianceCubemap, PrefilterCubemap, BRDF_LUT_SIZE, ENV_FILE_MAGIC,
    ENV_FILE_VERSION, IRRADIANCE_SIZE, PREFILTERED_ENV_MAX_LOD, PREFILTER_BASE_SIZE,
};
pub use crate::ibl_gpu::{upload, IblResources};
pub use crate::lambert::{
    make_uv_sphere, LambertPass, MaterialUniform, MvpUniform, Vertex as LambertVertex,
    SHADER_SOURCE as LAMBERT_SHADER_SOURCE,
};
pub use crate::pbr::{
    ALPHA_MODE_BLEND, ALPHA_MODE_CUTOUT, ALPHA_MODE_OPAQUE, IRRADIANCE_SHADER_SOURCE,
    MATERIAL_FLAG_EMISSIVE_MAP, PBR_PACKED_VERTEX_STRIDE_BYTES,
    PREFILTERED_ENV_MAX_LOD as PBR_PREFILTERED_ENV_MAX_LOD, PREFILTER_SHADER_SOURCE,
    SHADER_SOURCE as PBR_SHADER_SOURCE,
};
pub use crate::profiler::{debug_overlay, fps_from_duration, FrameStats, GpuProfiler, PassStats};
pub use crate::renderer::Renderer;
pub use crate::triangle::{TrianglePass, Vertex, SHADER_SOURCE, VERTICES};
