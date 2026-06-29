//! The per-frame [`FrameContext`] passed to render passes.
//!
//! A `FrameContext` carries the data passes need that is
//! per-frame rather than per-resource: the current swapchain
//! texture view, the surface format, and (later) timing
//! queries. The context is created by
//! `Renderer::begin_frame`
//! and consumed by
//! `Renderer::end_frame`.
//!
//! # Why a separate type?
//!
//! `FrameContext` is the natural place to grow the per-frame
//! payload (camera data, lighting data, frame index, GPU timing
//! queries). Keeping it as a concrete type rather than a pile of
//! loose parameters on `record()` keeps the [`crate::pass::PassContext`] API
//! stable as M2+ adds more per-frame state.

/// The data a pass needs to render to the current frame's
/// color attachment(s).
///
/// Constructed by
/// `Renderer::begin_frame`
/// (which hands the pass a view of the swapchain image) or by
/// the headless test path (which hands the pass a view of an
/// off-screen render target).
///
pub struct FrameContext {
    /// The texture view the pass renders to (the swapchain
    /// image, or an off-screen render target).
    surface_view: wgpu::TextureView,
    /// The format of the surface / render target (cached for
    /// the pass's color-target state).
    surface_format: wgpu::TextureFormat,
}

impl FrameContext {
    /// Constructs a new `FrameContext` from a view and format.
    /// The caller is responsible for ensuring the view is
    /// valid for the `'surface` lifetime (typically by holding
    /// the underlying texture).
    #[must_use]
    pub fn new(surface_view: wgpu::TextureView, surface_format: wgpu::TextureFormat) -> Self {
        Self {
            surface_view,
            surface_format,
        }
    }

    /// Returns a reference to the surface / render-target view
    /// the pass renders to.
    #[must_use]
    pub fn surface_view(&self) -> &wgpu::TextureView {
        &self.surface_view
    }

    /// Returns the format of the surface / render target.
    #[must_use]
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }
}

impl std::fmt::Debug for FrameContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameContext")
            .field("surface_format", &self.surface_format)
            .finish_non_exhaustive()
    }
}
