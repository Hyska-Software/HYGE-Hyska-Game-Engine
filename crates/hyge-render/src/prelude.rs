//! The renderer prelude.
//!
//! `use hyge_render::prelude::*;` brings the public types into
//! scope without naming them individually.

pub use crate::bindless::{
    BindlessConfig, BindlessSlot, BindlessTable, DrawCommand, DrawCommand as DrawCmd, GpuMaterial,
    GpuMesh, Instance, Light, LightGrid, MaterialId, MeshId, MeshletVisibility, Refcount, SlotKind,
    SlotTag, TextureId,
};
pub use crate::config::RendererConfig;
pub use crate::lambert::{
    make_uv_sphere, LambertPass, MaterialUniform, MvpUniform, Vertex as LambertVertex,
    SHADER_SOURCE as LAMBERT_SHADER_SOURCE,
};
pub use crate::profiler::{debug_overlay, fps_from_duration, FrameStats, GpuProfiler, PassStats};
pub use crate::renderer::Renderer;
pub use crate::triangle::{TrianglePass, Vertex, SHADER_SOURCE, VERTICES};
