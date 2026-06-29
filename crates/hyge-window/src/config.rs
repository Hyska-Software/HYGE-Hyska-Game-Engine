//! Window configuration.

/// Configuration for the application window.
///
/// Construct with [`WindowConfig::default`] and override the fields you
/// care about. `WindowConfig` is `Clone` so the [`crate::plugin::WindowPlugin`]
/// can keep a copy of the config after the user passes it in.
///
/// Implements [`hyge_ecs::Resource`] so it can be stored in the
/// [`bevy_app::App`]'s `World` and read back by systems.
///
/// # Example
///
/// ```
/// use hyge_window::prelude::WindowConfig;
///
/// let config = WindowConfig {
///     title: "My Game".to_string(),
///     width: 1920,
///     height: 1080,
///     ..WindowConfig::default()
/// };
/// ```
#[derive(Clone, Debug, hyge_ecs::Resource)]
pub struct WindowConfig {
    /// Window title (shown in the title bar and task bar).
    pub title: String,

    /// Initial width in physical pixels.
    pub width: u32,

    /// Initial height in physical pixels.
    pub height: u32,

    /// Whether the OS allows the user to resize the window.
    pub resizable: bool,

    /// Whether to enable VSync (limits frame rate to the display refresh).
    /// This is a hint; the actual swap-chain present mode is selected by
    /// `wgpu` and may be `Fifo` (vsync) or `Immediate` depending on driver
    /// and platform support.
    pub vsync: bool,

    /// Whether to register raw input devices on Windows. When `true`,
    /// mouse delta is reported at the hardware level (no OS cursor
    /// acceleration); when `false`, the OS may apply smoothing.
    /// Has no effect on non-Windows platforms.
    pub raw_input: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Hyge Engine".to_string(),
            width: 1280,
            height: 720,
            resizable: true,
            vsync: true,
            raw_input: true,
        }
    }
}
