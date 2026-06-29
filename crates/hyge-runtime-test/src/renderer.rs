//! Headless wgpu test renderer.
//!
//! [`TestRenderer`] is a thin wrapper over a headless
//! [`hyge_render::Renderer`] (constructed via
//! `Renderer::new_headless`) plus a convenience method for
//! rendering the first-triangle pass into a caller-supplied
//! off-screen target.
//!
//! The headless path uses `force_fallback_adapter: true`, so on
//! machines with a real GPU the test still uses a software
//! adapter — this makes the test deterministic across machines
//! and avoids GPU-driver-specific variations in the rendered
//! output (which is exactly what we want for a snapshot test).

use hyge_core::prelude::*;
use hyge_render::prelude::*;

/// Headless wgpu renderer for snapshot tests.
///
/// Wraps a [`hyge_render::Renderer`] constructed via
/// `Renderer::new_headless` (no surface, software adapter
/// preferred). Use [`TestRenderer::new`] to create; the
/// underlying wgpu device / queue can be accessed via
/// [`TestRenderer::device`] / [`TestRenderer::queue`].
pub struct TestRenderer {
    renderer: Renderer,
}

impl std::fmt::Debug for TestRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestRenderer").finish_non_exhaustive()
    }
}

impl TestRenderer {
    /// Creates a new headless renderer. Returns `None` if no wgpu
    /// adapter is available (e.g. on a CI runner without lavapipe
    /// and without a software backend).
    ///
    /// Callers should early-return on `None` so the test is
    /// treated as "skipped" rather than "failed".
    #[must_use]
    pub fn new() -> Option<Self> {
        let config = RendererConfig::default();
        match Renderer::new_headless(&config) {
            Ok(renderer) => Some(Self { renderer }),
            Err(e) => {
                eprintln!("TestRenderer::new: no wgpu adapter available ({e}); skipping");
                None
            }
        }
    }

    /// Returns the wgpu device. The caller is responsible for
    /// creating off-screen render targets with the right format
    /// (call [`TestRenderer::surface_format`] to get the format
    /// the test pipeline was created with).
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        self.renderer.device()
    }

    /// Returns the wgpu queue.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        self.renderer.queue()
    }

    /// Returns the texture format the pre-built triangle pipeline
    /// was created with. Off-screen render targets should use
    /// this format.
    #[must_use]
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.renderer.surface_format()
    }

    /// Renders the first triangle (R-024) into the off-screen
    /// `target` texture. The `clear_color` is the color the
    /// target is cleared to before the triangle is drawn.
    ///
    /// After this call returns, the target has been rendered and
    /// the GPU has been polled (`Maintain::Wait`), so the caller
    /// can immediately call [`capture_frame`] to read the bytes
    /// back.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the underlying renderer
    /// fails to submit the commands.
    pub fn render_triangle(
        &mut self,
        target: &wgpu::Texture,
        clear_color: [f32; 4],
    ) -> HygeResult<()> {
        self.renderer.render_triangle_to_texture(
            target,
            wgpu::Color {
                r: clear_color[0] as f64,
                g: clear_color[1] as f64,
                b: clear_color[2] as f64,
                a: clear_color[3] as f64,
            },
        )
    }
}
