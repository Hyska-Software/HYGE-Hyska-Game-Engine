//! Engine-wide configuration.

use std::path::PathBuf;

use hyge_core::Vec3;
use hyge_window::WindowConfig;

/// The top-level configuration passed to [`crate::App::new`].
///
/// Every subsystem reads its own field; subsystems not enabled here are
/// still loaded as placeholders (see `docs/roadmap.toml` for the M0–M7
/// rollout). [`Default`] gives a sensible starting point for a full engine
/// run; tweak individual fields to specialize.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// Window configuration (title, size, resizable, vsync, raw input).
    pub window: WindowConfig,

    /// Color to clear the surface to each frame, in linear RGBA.
    /// The renderer is not yet wired in M1, so this value is stored but
    /// not consumed. It is read by the renderer in M3+.
    pub clear_color: [f32; 4],

    /// Renderer backend selection. The actual renderer is a placeholder
    /// in M1; M3 wires the real `wgpu` device.
    pub renderer: RendererConfig,

    /// Asset DB / cache directory and behavior.
    pub assets: AssetsConfig,

    /// Physics simulation configuration.
    pub physics: PhysicsConfig,

    /// Audio engine configuration.
    pub audio: AudioConfig,

    /// Input bindings and gamepad configuration.
    pub input: InputConfig,

    /// Lua scripting configuration.
    pub script: ScriptConfig,

    /// Editor configuration (the editor is a separate window; its
    /// enabled flag here is for the in-process editor in M6+).
    pub editor: EditorConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            clear_color: [0.1, 0.1, 0.1, 1.0],
            renderer: RendererConfig::default(),
            assets: AssetsConfig::default(),
            physics: PhysicsConfig::default(),
            audio: AudioConfig::default(),
            input: InputConfig::default(),
            script: ScriptConfig::default(),
            editor: EditorConfig::default(),
        }
    }
}

/// Renderer backend selection. `Auto` lets `wgpu` pick the best available
/// backend for the current platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererConfig {
    /// Which `wgpu` backend to use.
    pub backend: RendererBackend,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self { backend: RendererBackend::Auto }
    }
}

/// `wgpu` backend selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererBackend {
    /// Let `wgpu` pick the best available backend (DX12 on Windows,
    /// Metal on macOS, Vulkan on Linux).
    Auto,
    /// Force Vulkan.
    Vulkan,
    /// Force DX12.
    Dx12,
    /// Force Metal.
    Metal,
}

impl Default for RendererBackend {
    fn default() -> Self {
        RendererBackend::Auto
    }
}

/// Asset DB and cache configuration.
#[derive(Clone, Debug)]
pub struct AssetsConfig {
    /// Directory where the asset DB and cooked cache live. `None` means
    /// use the default (`./assets/cache/` relative to the executable).
    pub cache_dir: Option<PathBuf>,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self { cache_dir: None }
    }
}

/// Physics simulation configuration. The actual integration is a
/// placeholder in M1; M5 wires `rapier3d`.
#[derive(Clone, Debug)]
pub struct PhysicsConfig {
    /// Whether the physics subsystem is enabled.
    pub enabled: bool,
    /// Fixed timestep in seconds (default 60 Hz).
    pub fixed_timestep: f32,
    /// Gravity vector in world space.
    pub gravity: Vec3,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fixed_timestep: 1.0 / 60.0,
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

/// Audio engine configuration. The actual integration is a placeholder
/// in M1; M5 wires `kira`.
#[derive(Clone, Debug)]
pub struct AudioConfig {
    /// Whether the audio subsystem is enabled.
    pub enabled: bool,
    /// Whether to enable HRTF (requires the `audio-hrtf` feature flag at
    /// compile time and a KEMAR-derived dataset at runtime).
    pub hrtf: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { enabled: true, hrtf: false }
    }
}

/// Input bindings and gamepad configuration.
#[derive(Clone, Debug)]
pub struct InputConfig {
    /// Optional path to a TOML binding file. `None` means use the default
    /// (`assets/input.bind.toml`).
    pub binding_file: Option<PathBuf>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self { binding_file: None }
    }
}

/// Lua scripting configuration. The actual integration is a placeholder
/// in M1; M5 wires `mlua` and the `bevy_reflect`-driven bindings.
#[derive(Clone, Debug)]
pub struct ScriptConfig {
    /// Whether the scripting subsystem is enabled.
    pub enabled: bool,
    /// Whether the sandbox is enforced (recommended for production).
    pub sandbox: bool,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self { enabled: true, sandbox: true }
    }
}

/// Editor configuration. The editor is a separate window in M6+.
#[derive(Clone, Debug)]
pub struct EditorConfig {
    /// Whether the editor is enabled (the in-process editor, not the
    /// game window).
    pub enabled: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        // Default is `false` because the editor is opt-in; games ship
        // without it.
        Self { enabled: false }
    }
}
