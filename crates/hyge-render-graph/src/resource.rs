//! Render-graph resources: textures and buffers with their descriptors and
//! lifetimes.
//!
//! A [`ResourceHandle`] is a stable, opaque index into the graph's resource
//! table. Passes declare which resources they read or write by passing
//! these handles back to [`RenderGraph::add_pass`](crate::graph::RenderGraph::add_pass)
//! via their [`Pass::reads`](crate::pass::Pass::reads) and
//! [`Pass::writes`](crate::pass::Pass::writes) implementations.
//!
//! A resource is either a texture ([`TextureDesc`]) or a buffer
//! ([`BufferDesc`]) and is tagged with a [`ResourceLifetime`] that says
//! whether the graph should manage its allocation (Transient) or whether
//! the caller owns it across frames (Persistent).

use std::fmt;

/// Stable handle to a resource declared in a [`RenderGraph`](crate::graph::RenderGraph).
///
/// Handles are `Copy` and compare by index. Two handles with the same
/// index refer to the same resource slot; the index is unique within
/// the graph that minted it.
#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct ResourceHandle(u32);

impl ResourceHandle {
    /// Constructs a `ResourceHandle` from a raw index. Used internally
    /// by [`RenderGraph::add_resource`](crate::graph::RenderGraph::add_resource);
    /// downstream code should treat the handle as opaque and not synthesize
    /// indices on its own.
    #[inline]
    #[must_use]
    pub(crate) fn from_index(idx: u32) -> Self {
        Self(idx)
    }

    /// Returns the raw index of this handle.
    #[inline]
    #[must_use]
    pub fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ResourceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// Description of a texture resource managed by the graph.
///
/// `usage` is the *allowed* usage set; the actual wgpu `Texture` is
/// created with these usage bits ORed together. The runtime never
/// silently widens usage beyond what was declared here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Depth (for 3D textures) or 1 for 2D / 2D array.
    pub depth: u32,
    /// Array layer count (1 for non-array).
    pub array_layers: u32,
    /// Mip level count (1 for no mip chain).
    pub mip_levels: u32,
    /// Sample count (1 for no MSAA).
    pub sample_count: u32,
    /// Pixel format.
    pub format: wgpu::TextureFormat,
    /// Allowed usage bits; the actual wgpu texture is created with this
    /// exact mask.
    pub usage: wgpu::TextureUsages,
    /// Human-readable name used in wgpu debug labels and tracing spans.
    pub label: Option<String>,
}

impl TextureDesc {
    /// Convenience constructor for a 2D, single-mip, single-sample texture.
    #[must_use]
    pub fn new_2d(width: u32, height: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> Self {
        Self {
            width,
            height,
            depth: 1,
            array_layers: 1,
            mip_levels: 1,
            sample_count: 1,
            format,
            usage,
            label: None,
        }
    }
}

/// Description of a buffer resource managed by the graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BufferDesc {
    /// Size in bytes.
    pub size: u64,
    /// Allowed usage bits.
    pub usage: wgpu::BufferUsages,
    /// Human-readable name used in wgpu debug labels and tracing spans.
    pub label: Option<String>,
}

impl BufferDesc {
    /// Convenience constructor.
    #[must_use]
    pub fn new(size: u64, usage: wgpu::BufferUsages) -> Self {
        Self { size, usage, label: None }
    }
}

/// What kind of resource a handle refers to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// A texture resource.
    Texture(TextureDesc),
    /// A buffer resource.
    Buffer(BufferDesc),
}

impl ResourceKind {
    /// Returns the optional debug label, regardless of variant.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            ResourceKind::Texture(t) => t.label.as_deref(),
            ResourceKind::Buffer(b) => b.label.as_deref(),
        }
    }

    /// Returns true if this resource is a texture.
    #[must_use]
    pub fn is_texture(&self) -> bool {
        matches!(self, ResourceKind::Texture(_))
    }

    /// Returns true if this resource is a buffer.
    #[must_use]
    pub fn is_buffer(&self) -> bool {
        matches!(self, ResourceKind::Buffer(_))
    }
}

/// Lifetime of a resource in the render graph.
///
/// `Transient` resources are allocated by the graph at first touch and
/// freed at the end of the frame; they are the dominant case for
/// per-frame scratch textures (depth, motion vectors, etc.).
///
/// `Persistent` resources are owned externally; the graph never frees
/// them and assumes they outlive the compiled graph (shadow atlas,
/// bindless heap, swapchain image).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResourceLifetime {
    /// Allocated and freed within a single frame by the
    /// [`TransientAllocator`](crate::allocator::TransientAllocator).
    Transient,
    /// Lives across frames; the graph never frees it.
    Persistent,
}

impl ResourceLifetime {
    /// Returns true if this is a transient resource.
    #[must_use]
    pub fn is_transient(self) -> bool {
        matches!(self, ResourceLifetime::Transient)
    }

    /// Returns true if this is a persistent resource.
    #[must_use]
    pub fn is_persistent(self) -> bool {
        matches!(self, ResourceLifetime::Persistent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_is_copy_and_distinct() {
        let a = ResourceHandle::from_index(0);
        let b = ResourceHandle::from_index(1);
        let c = a; // Copy
        assert_eq!(a, c);
        assert_ne!(a, b);
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
    }

    #[test]
    fn handle_display_is_compact() {
        let h = ResourceHandle::from_index(7);
        assert_eq!(format!("{h}"), "r7");
    }

    #[test]
    fn texture_desc_2d_constructor() {
        let d = TextureDesc::new_2d(1920, 1080, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureUsages::RENDER_ATTACHMENT);
        assert_eq!(d.width, 1920);
        assert_eq!(d.height, 1080);
        assert_eq!(d.depth, 1);
        assert_eq!(d.array_layers, 1);
        assert_eq!(d.mip_levels, 1);
        assert_eq!(d.sample_count, 1);
    }

    #[test]
    fn buffer_desc_constructor() {
        let d = BufferDesc::new(1024, wgpu::BufferUsages::VERTEX);
        assert_eq!(d.size, 1024);
    }

    #[test]
    fn resource_kind_predicates() {
        let tex = ResourceKind::Texture(TextureDesc::new_2d(1, 1, wgpu::TextureFormat::R8Unorm, wgpu::TextureUsages::TEXTURE_BINDING));
        let buf = ResourceKind::Buffer(BufferDesc::new(16, wgpu::BufferUsages::STORAGE));
        assert!(tex.is_texture());
        assert!(!tex.is_buffer());
        assert!(buf.is_buffer());
        assert!(!buf.is_texture());
    }

    #[test]
    fn resource_lifetime_predicates() {
        assert!(ResourceLifetime::Transient.is_transient());
        assert!(!ResourceLifetime::Transient.is_persistent());
        assert!(ResourceLifetime::Persistent.is_persistent());
        assert!(!ResourceLifetime::Persistent.is_transient());
    }
}
