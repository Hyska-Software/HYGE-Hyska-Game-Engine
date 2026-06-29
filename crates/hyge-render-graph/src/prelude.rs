//! Convenience re-exports for `hyge-render-graph`.
//!
//! `use hyge_render_graph::prelude::*;` brings the most-used
//! types into scope without naming each one individually.

pub use crate::allocator::TransientAllocator;
pub use crate::barrier::Barrier;
pub use crate::compile::{CompiledGraph, CompiledPass};
pub use crate::graph::{PassId, RenderGraph};
pub use crate::pass::{Pass, PassContext};
pub use crate::resource::{BufferDesc, ResourceHandle, ResourceKind, ResourceLifetime, TextureDesc};
