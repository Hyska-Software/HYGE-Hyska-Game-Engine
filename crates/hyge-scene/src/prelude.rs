//! Hyge scene prelude.

pub use crate::components::{LightComponent, MaterialHandle, MeshHandle, WorldTransform};
pub use crate::extract::{
    add_render_extract_system, render_extract, render_extract_system, DrawCommand, FrameSnapshot,
    Instance, Light,
};
