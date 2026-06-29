//! Application window wrapper around `winit`.

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window as WinitWindow;

use crate::config::WindowConfig;
use crate::result::HygeError;

/// The application window.
///
/// Wraps a `winit::Window` in an `Arc` so it can be shared with the
/// renderer (for `wgpu::Surface` creation) and the input system (for raw
/// input). The window is created from an `ActiveEventLoop`; the plugin
/// itself does not create it (the event loop is owned by `hyge-app` or
/// the user's main function).
///
/// # Surface integration with `hyge-render`
///
/// `hyge-window` does not depend on `wgpu`. The `wgpu::Surface` is created
/// in `hyge-render` from the [`handle`](Self::handle) returned here:
///
/// ```ignore
/// let window = Window::new(&event_loop, config)?;
/// let surface = instance.create_surface_unsafe(
///     wgpu::SurfaceTargetUnsafe::from_window(&window.handle())?,
/// );
/// ```
pub struct Window {
    inner: Arc<WinitWindow>,
    config: WindowConfig,
}

impl Window {
    /// Creates a new window from the given event loop and config.
    pub fn new(event_loop: &ActiveEventLoop, config: WindowConfig) -> Result<Self, HygeError> {
        use winit::window::WindowAttributes;
        let attrs = WindowAttributes::default()
            .with_title(&config.title)
            .with_inner_size(PhysicalSize::new(config.width, config.height))
            .with_resizable(config.resizable);
        let window = event_loop
            .create_window(attrs)
            .map_err(|e| HygeError::Unsupported(format!("create_window: {e}")))?;
        Ok(Self {
            inner: Arc::new(window),
            config,
        })
    }

    /// Returns a clone of the inner `Arc<winit::Window>`. Pass this to
    /// `wgpu::Instance::create_surface_unsafe` to create a render surface
    /// in `hyge-render`.
    pub fn handle(&self) -> Arc<WinitWindow> {
        self.inner.clone()
    }

    /// Returns the window configuration used to create this window.
    pub fn config(&self) -> &WindowConfig {
        &self.config
    }

    /// Returns the current size in physical pixels.
    pub fn size(&self) -> PhysicalSize<u32> {
        self.inner.inner_size()
    }

    /// Returns the current scale factor (DPI multiplier).
    pub fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }

    /// Requests a redraw on the next event-loop iteration. The window
    /// receives a `RedrawRequested` event after this call.
    pub fn request_redraw(&self) {
        self.inner.request_redraw();
    }

    /// Sets the window title.
    pub fn set_title(&self, title: &str) {
        self.inner.set_title(title);
    }
}
