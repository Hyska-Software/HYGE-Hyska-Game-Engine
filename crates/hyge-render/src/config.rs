//! Configuration for the runtime [`Renderer`](crate::renderer::Renderer).
//!
//! The config covers the wgpu backends to enable, the present mode
//! (vsync), power-preference for adapter selection, and the
//! validation-layer toggle. A sensible default is provided via
//! [`RendererConfig::default`]; the `validation` field follows the
//! build profile (on in debug, off in release) so a debug build
//! always gets the validation layer automatically.

/// The user-facing configuration for the runtime renderer.
///
/// All fields are public so callers can override individual settings
/// after cloning the default. None of the fields are validated at
/// construction time — invalid combinations (e.g. requesting a
/// present mode the surface doesn't support) are reported by
/// [`Renderer::new`](crate::renderer::Renderer::new) as an unsupported
/// configuration error.
#[derive(Debug, Clone)]
pub struct RendererConfig {
    /// The wgpu backends to enable. Defaults to
    /// [`wgpu::Backends::all`] (every backend wgpu was built with).
    pub backend: wgpu::Backends,
    /// Whether to enable vsync. When `true`, the present mode
    /// defaults to [`wgpu::PresentMode::Fifo`] (the only
    /// vsync-guaranteed mode in wgpu 22); when `false`, the
    /// present mode defaults to [`wgpu::PresentMode::Mailbox`]
    /// (low-latency, no tearing, if the surface supports it).
    pub vsync: bool,
    /// The present mode the surface will be configured with. If
    /// the surface does not advertise this mode, the renderer
    /// will fall back to `Fifo` during `configure`.
    pub present_mode: wgpu::PresentMode,
    /// The power preference hint passed to
    /// `wgpu::Instance::request_adapter`. Defaults to
    /// [`wgpu::PowerPreference::HighPerformance`] (discrete GPU
    /// when available).
    pub power_preference: wgpu::PowerPreference,
    /// Whether to enable the wgpu validation layer. The default
    /// follows `cfg!(debug_assertions)` — on in debug, off in
    /// release — so debug builds get full validation for free.
    pub validation: bool,
    /// The label applied to the wgpu instance (for debug captures
    /// and `wgpu-info`).
    pub instance_label: String,
    /// The label applied to the wgpu device (for debug captures,
    /// error messages, and tracing spans).
    pub device_label: String,
}

impl RendererConfig {
    /// Returns a configuration with safe defaults: all backends,
    /// vsync on (`Fifo` present), high-performance adapter, debug
    /// validation on (in debug builds) or off (in release).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the [`wgpu::Backends`] to enable and returns `self`
    /// for chaining.
    #[must_use]
    pub fn with_backend(mut self, backend: wgpu::Backends) -> Self {
        self.backend = backend;
        self
    }

    /// Sets the vsync toggle. When `true`, the present mode is
    /// forced to `Fifo`; when `false`, it is forced to `Mailbox`.
    #[must_use]
    pub fn with_vsync(mut self, vsync: bool) -> Self {
        self.vsync = vsync;
        // Keep the present_mode field in sync with vsync so
        // downstream code can read either.
        self.present_mode = if vsync {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::Mailbox
        };
        self
    }

    /// Overrides the present mode. Use this when you need a
    /// specific mode (e.g. `Immediate` for benchmarks or
    /// `FifoRelaxed` for sub-1-frame latency).
    #[must_use]
    pub fn with_present_mode(mut self, mode: wgpu::PresentMode) -> Self {
        self.present_mode = mode;
        // Explicit present_mode overrides the vsync-derived default.
        self.vsync = matches!(
            mode,
            wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed
        );
        self
    }

    /// Overrides the power preference.
    #[must_use]
    pub fn with_power_preference(mut self, pref: wgpu::PowerPreference) -> Self {
        self.power_preference = pref;
        self
    }

    /// Toggles the validation layer explicitly. When `None`, the
    /// layer follows the build profile (`cfg!(debug_assertions)`).
    #[must_use]
    pub fn with_validation(mut self, on: bool) -> Self {
        self.validation = on;
        self
    }
}

impl Default for RendererConfig {
    /// `debug_assertions` on → validation layer on; off → off. All
    /// other defaults: all backends, vsync, `Fifo`, high
    /// performance, labels `"hyge-render"` / `"hyge-device"`.
    fn default() -> Self {
        let vsync = true;
        Self {
            backend: wgpu::Backends::all(),
            vsync,
            present_mode: wgpu::PresentMode::Fifo,
            power_preference: wgpu::PowerPreference::HighPerformance,
            validation: cfg!(debug_assertions),
            instance_label: "hyge-render".to_string(),
            device_label: "hyge-device".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let c = RendererConfig::default();
        assert!(c.vsync);
        assert_eq!(c.present_mode, wgpu::PresentMode::Fifo);
        assert_eq!(c.backend, wgpu::Backends::all());
        assert_eq!(c.power_preference, wgpu::PowerPreference::HighPerformance);
        // The validation field follows the build profile, which is
        // what the user expects (debug = on, release = off).
        assert_eq!(c.validation, cfg!(debug_assertions));
        assert!(!c.instance_label.is_empty());
        assert!(!c.device_label.is_empty());
    }

    #[test]
    fn with_vsync_toggles_present_mode() {
        let on = RendererConfig::default().with_vsync(true);
        assert!(on.vsync);
        assert_eq!(on.present_mode, wgpu::PresentMode::Fifo);

        let off = RendererConfig::default().with_vsync(false);
        assert!(!off.vsync);
        assert_eq!(off.present_mode, wgpu::PresentMode::Mailbox);
    }

    #[test]
    fn with_present_mode_overrides_vsync() {
        let c = RendererConfig::default()
            .with_vsync(true)
            .with_present_mode(wgpu::PresentMode::Immediate);
        assert_eq!(c.present_mode, wgpu::PresentMode::Immediate);
        // Immediate is not a vsync mode, so `vsync` is reset to false.
        assert!(!c.vsync);
    }

    #[test]
    fn with_backend_and_power_preference() {
        let c = RendererConfig::default()
            .with_backend(wgpu::Backends::VULKAN)
            .with_power_preference(wgpu::PowerPreference::LowPower);
        assert_eq!(c.backend, wgpu::Backends::VULKAN);
        assert_eq!(c.power_preference, wgpu::PowerPreference::LowPower);
    }

    #[test]
    fn with_validation_toggles() {
        let on = RendererConfig::default().with_validation(true);
        assert!(on.validation);
        let off = RendererConfig::default().with_validation(false);
        assert!(!off.validation);
    }

    #[test]
    fn config_is_cloneable() {
        let a = RendererConfig::default();
        let b = a.clone();
        assert_eq!(a.backend, b.backend);
        assert_eq!(a.vsync, b.vsync);
        assert_eq!(a.present_mode, b.present_mode);
        assert_eq!(a.validation, b.validation);
    }
}
