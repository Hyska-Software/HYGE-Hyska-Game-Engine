//! Render-pass barriers: usage transitions for textures and buffers.
//!
//! A [`Barrier`] is a single resource-usage transition emitted by the
//! render-graph compiler between two passes. They are inferred from
//! the `reads`/`writes` declarations of each pass and the
//! optional [`Pass::texture_usages`](crate::pass::Pass::texture_usages)
//! and [`Pass::buffer_usages`](crate::pass::Pass::buffer_usages) hints.
//!
//! At the skeleton stage (R-020) the barriers are computed and
//! returned alongside the compiled pass sequence; the actual
//! `wgpu` transition emission happens in the `execute` step.
//! wgpu 22's high-level API does not expose a per-resource
//! `transition_resources` method, so the encoder-level emission
//! is a TODO and is wired in R-021 (barrier inference) once the
//! per-pass usage hints are fleshed out.

use std::fmt;

use crate::resource::ResourceHandle;

/// A single resource-usage transition between two passes in the
/// compiled render graph.
///
/// The variants mirror the two kinds of GPU resources: textures and
/// buffers. The `from` field is the prior access pattern (or
/// `wgpu::TextureUses::UNINITIALIZED` / `wgpu::BufferUses::empty()` for
/// first-touch barriers).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Barrier {
    /// A texture-usage transition.
    Texture {
        /// The resource the barrier applies to.
        resource: ResourceHandle,
        /// The previous access pattern. `wgpu::TextureUses::UNINITIALIZED` for first touch.
        from: wgpu::TextureUses,
        /// The new access pattern. The pass following this barrier will
        /// read/write with these usages.
        to: wgpu::TextureUses,
    },
    /// A buffer-usage transition.
    Buffer {
        /// The resource the barrier applies to.
        resource: ResourceHandle,
        /// The previous access pattern. `wgpu::BufferUses::empty()` for first touch.
        from: wgpu::BufferUses,
        /// The new access pattern. The pass following this barrier will
        /// read/write with these usages.
        to: wgpu::BufferUses,
    },
}

impl Barrier {
    /// Returns the resource the barrier applies to, regardless of variant.
    #[must_use]
    pub fn resource(&self) -> ResourceHandle {
        match self {
            Barrier::Texture { resource, .. } | Barrier::Buffer { resource, .. } => *resource,
        }
    }

    /// Returns true if this is a texture barrier.
    #[must_use]
    pub fn is_texture(&self) -> bool {
        matches!(self, Barrier::Texture { .. })
    }

    /// Returns true if this is a buffer barrier.
    #[must_use]
    pub fn is_buffer(&self) -> bool {
        matches!(self, Barrier::Buffer { .. })
    }
}

impl fmt::Display for Barrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Barrier::Texture { resource, from, to } => {
                write!(f, "Texture({}: {:?} -> {:?})", resource, from, to)
            }
            Barrier::Buffer { resource, from, to } => {
                write!(f, "Buffer({}: {:?} -> {:?})", resource, from, to)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_extractor() {
        let h = ResourceHandle::from_index(42);
        let b = Barrier::Texture {
            resource: h,
            from: wgpu::TextureUses::UNINITIALIZED,
            to: wgpu::TextureUses::RENDER_ATTACHMENT,
        };
        assert_eq!(b.resource(), h);
        assert!(b.is_texture());
        assert!(!b.is_buffer());
    }

    #[test]
    fn display_is_informative() {
        let h = ResourceHandle::from_index(2);
        let b = Barrier::Buffer {
            resource: h,
            from: wgpu::BufferUses::empty(),
            to: wgpu::BufferUsages::VERTEX,
        };
        let s = format!("{b}");
        assert!(s.contains("r2"));
        assert!(s.contains("VERTEX"));
    }
}
