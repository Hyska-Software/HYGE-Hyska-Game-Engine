//! The [`Pass`] trait and the [`PassContext`] provided to it.
//!
//! Every render-graph pass implements [`Pass`]. The graph calls
//! `reads()` and `writes()` during build to infer edges and barriers,
//! and `record()` during execute to emit GPU commands into the
//! `wgpu::CommandEncoder` the graph provides.

use crate::frame::FrameContext;
use crate::resource::ResourceHandle;

/// Context passed to [`Pass::record`].
///
/// The context exposes:
/// - the compiled resource table, so the pass can resolve
///   [`ResourceHandle`]s back to concrete `wgpu` objects
///   ([`PassContext::texture`], [`PassContext::buffer`]);
/// - the `wgpu::CommandEncoder` the pass records into
///   ([`PassContext::encoder`]);
/// - the optional per-frame [`FrameContext`] (the current
///   surface view + format), for passes that draw to the
///   swapchain — [`PassContext::frame`].
pub struct PassContext<'a> {
    pub(crate) resources: &'a ResourceTable,
    pub(crate) encoder: &'a mut wgpu::CommandEncoder,
    /// `Some` for passes that draw to the current frame (the
    /// windowed `Renderer::render_frame` path); `None` for the
    /// headless test path (where the pass records into a
    /// user-provided `wgpu::CommandEncoder` with no surface).
    pub(crate) frame: Option<&'a mut FrameContext>,
}

impl<'a> PassContext<'a> {
    /// Constructs a new `PassContext` bound to the compiled
    /// resource table, the encoder the pass records into, and
    /// the optional per-frame context.
    #[inline]
    #[must_use]
    pub(crate) fn new(
        resources: &'a ResourceTable,
        encoder: &'a mut wgpu::CommandEncoder,
        frame: Option<&'a mut FrameContext>,
    ) -> Self {
        Self {
            resources,
            encoder,
            frame,
        }
    }

    /// Looks up the concrete `wgpu::Texture` backing a resource handle.
    /// Returns `None` if the handle is unknown or the resource is a buffer.
    #[inline]
    #[must_use]
    pub fn texture(&self, handle: ResourceHandle) -> Option<&wgpu::Texture> {
        self.resources.texture(handle)
    }

    /// Looks up the concrete `wgpu::Buffer` backing a resource handle.
    /// Returns `None` if the handle is unknown or the resource is a texture.
    #[inline]
    #[must_use]
    pub fn buffer(&self, handle: ResourceHandle) -> Option<&wgpu::Buffer> {
        self.resources.buffer(handle)
    }

    /// Returns a mutable reference to the `wgpu::CommandEncoder` the
    /// pass records into. The pass may begin render / compute passes,
    /// issue copies, push debug groups, and write timestamps.
    #[inline]
    #[must_use]
    pub fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder
    }

    /// Returns the encoder and immutable per-frame context together.
    ///
    /// This split borrow is useful for render passes whose descriptor
    /// needs a frame texture view while also beginning a pass on the
    /// command encoder.
    #[inline]
    pub fn encoder_and_frame(&mut self) -> (&mut wgpu::CommandEncoder, Option<&FrameContext>) {
        (self.encoder, self.frame.as_deref())
    }

    /// Returns a mutable reference to the per-frame
    /// [`FrameContext`], or `None` if no surface is bound (the
    /// headless test path).
    #[inline]
    #[must_use]
    pub fn frame(&mut self) -> Option<&mut FrameContext> {
        self.frame.as_deref_mut()
    }

    /// Constructs a `PassContext` for callers outside the graph
    /// (e.g. the windowed renderer's `render_frame` path) that
    /// need to record a single pass directly. The returned
    /// context borrows from `frame` and `encoder`; the caller's
    /// borrows must stay alive for the duration of `record()`.
    ///
    /// The `resources` table is a default empty table; passes
    /// that read graph-managed resources must use the compiled
    /// graph's `execute` path instead.
    #[inline]
    pub fn for_frame<'b>(
        frame: &'b mut FrameContext,
        encoder: &'b mut wgpu::CommandEncoder,
    ) -> PassContext<'b> {
        // Build a default empty resource table by leaking one
        // `ResourceTable` from a static. This sidesteps the
        // `pub(crate)` constructor while keeping the type
        // compatible with `Pass::record`.
        static EMPTY: std::sync::OnceLock<ResourceTable> = std::sync::OnceLock::new();
        let resources: &ResourceTable = EMPTY.get_or_init(ResourceTable::default);
        PassContext {
            resources,
            encoder,
            frame: Some(frame),
        }
    }
}

/// The compiled resource table: a side-table that maps
/// [`ResourceHandle`]s to their backing `wgpu::Texture` / `wgpu::Buffer`.
///
/// The graph populates this in [`CompiledGraph::execute`](crate::compile::CompiledGraph::execute)
/// for transient resources (allocated on the fly) and at any time
/// for persistent resources (via [`ResourceTable::set_texture`] and
/// [`ResourceTable::set_buffer`]).
#[derive(Default)]
pub(crate) struct ResourceTable {
    textures: Vec<Option<wgpu::Texture>>,
    buffers: Vec<Option<wgpu::Buffer>>,
}

impl ResourceTable {
    /// Ensures the internal texture slot exists for the given handle.
    pub(crate) fn ensure_texture_slot(&mut self, handle: ResourceHandle) {
        let idx = handle.index() as usize;
        if self.textures.len() <= idx {
            self.textures.resize_with(idx + 1, || None);
        }
    }

    /// Ensures the internal buffer slot exists for the given handle.
    pub(crate) fn ensure_buffer_slot(&mut self, handle: ResourceHandle) {
        let idx = handle.index() as usize;
        if self.buffers.len() <= idx {
            self.buffers.resize_with(idx + 1, || None);
        }
    }

    /// Installs a concrete `wgpu::Texture` at the given handle.
    #[allow(dead_code)]
    pub(crate) fn set_texture(&mut self, handle: ResourceHandle, texture: wgpu::Texture) {
        self.ensure_texture_slot(handle);
        self.textures[handle.index() as usize] = Some(texture);
    }

    /// Installs a concrete `wgpu::Buffer` at the given handle.
    #[allow(dead_code)]
    pub(crate) fn set_buffer(&mut self, handle: ResourceHandle, buffer: wgpu::Buffer) {
        self.ensure_buffer_slot(handle);
        self.buffers[handle.index() as usize] = Some(buffer);
    }

    /// Borrows the texture at the given handle, if any.
    #[must_use]
    pub(crate) fn texture(&self, handle: ResourceHandle) -> Option<&wgpu::Texture> {
        self.textures
            .get(handle.index() as usize)
            .and_then(Option::as_ref)
    }

    /// Borrows the buffer at the given handle, if any.
    #[must_use]
    pub(crate) fn buffer(&self, handle: ResourceHandle) -> Option<&wgpu::Buffer> {
        self.buffers
            .get(handle.index() as usize)
            .and_then(Option::as_ref)
    }
}

/// A render pass in the graph.
///
/// Implement this trait for any operation that needs to read or write
/// graph-managed resources. The graph calls [`Pass::reads`] and
/// [`Pass::writes`] during the build phase to determine edges and
/// infer barriers, and [`Pass::record`] during the execute phase to
/// emit GPU commands into the encoder exposed via
/// [`PassContext::encoder`].
///
/// # Usage hint methods
///
/// The default impls of [`Pass::texture_usages`] and
/// [`Pass::buffer_usages`] return an empty list. Passes that want the
/// graph to emit precise `wgpu` usage transitions for them should
/// override these to declare the `wgpu::TextureUsages` /
/// `wgpu::BufferUsages` they will read/write per resource; the
/// compiler threads those into the [`Barrier`](crate::barrier::Barrier)
/// inference. Overrides are optional — passes that emit their own
/// `wgpu` transitions inside `record` can ignore them.
pub trait Pass: Send + Sync + 'static {
    /// Returns the human-readable name of this pass. Used in debug
    /// labels and tracing spans.
    fn name(&self) -> &str;

    /// Returns the list of resources this pass reads. The graph uses
    /// these declarations to:
    /// 1. infer producer/consumer edges in the DAG, and
    /// 2. compute the barrier that transitions each read resource
    ///    into the read-access pattern.
    fn reads(&self) -> Vec<ResourceHandle>;

    /// Returns the list of resources this pass writes. The graph uses
    /// these declarations similarly to `reads`.
    fn writes(&self) -> Vec<ResourceHandle>;

    /// Declares the `wgpu::TextureUsages` this pass needs for each
    /// texture it reads or writes. The default returns an empty
    /// list, which means "no declared usages" — the graph still emits
    /// a barrier marker but cannot fill in the precise `to` usage
    /// field; the pass is then expected to manage its own transitions.
    fn texture_usages(&self) -> Vec<(ResourceHandle, wgpu::TextureUsages)> {
        Vec::new()
    }

    /// Declares the `wgpu::BufferUsages` this pass needs for each
    /// buffer it reads or writes. Same semantics as
    /// [`Pass::texture_usages`].
    fn buffer_usages(&self) -> Vec<(ResourceHandle, wgpu::BufferUsages)> {
        Vec::new()
    }

    /// Records GPU commands into the encoder for this pass.
    ///
    /// The encoder is the one the calling
    /// [`CompiledGraph::execute`](crate::compile::CompiledGraph::execute)
    /// built. Resources are resolved via the [`PassContext`].
    fn record(&mut self, ctx: &mut PassContext<'_>);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trivial pass used in compile-cycle tests. Doesn't touch any
    /// resources; its sole role is to be a stable identifier for
    /// the pass slot.
    struct NoopPass(&'static str);
    impl Pass for NoopPass {
        fn name(&self) -> &str {
            self.0
        }
        fn reads(&self) -> Vec<ResourceHandle> {
            Vec::new()
        }
        fn writes(&self) -> Vec<ResourceHandle> {
            Vec::new()
        }
        fn record(&mut self, _ctx: &mut PassContext<'_>) {}
    }

    #[test]
    fn pass_trait_defaults_are_empty() {
        let p = NoopPass("noop");
        assert_eq!(p.name(), "noop");
        assert!(p.texture_usages().is_empty());
        assert!(p.buffer_usages().is_empty());
        assert!(p.reads().is_empty());
        assert!(p.writes().is_empty());
    }
}
