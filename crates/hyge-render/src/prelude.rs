//! The renderer prelude.
//!
//! `use hyge_render::prelude::*;` brings the public types into
//! scope without naming them individually.

pub use crate::config::RendererConfig;
pub use crate::renderer::Renderer;
pub use crate::triangle::{TrianglePass, Vertex, SHADER_SOURCE, VERTICES};
