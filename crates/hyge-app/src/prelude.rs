//! Convenience re-exports for `hyge-app`.

pub use crate::config::{
    AppConfig, AssetsConfig, AudioConfig, EditorConfig, InputConfig, PhysicsConfig,
    RendererBackend, RendererConfig, ScriptConfig,
};
pub use crate::{default_plugins, App, AppBuilder};

// Re-export `WindowConfig` so examples and tests can write
// `use hyge_app::prelude::WindowConfig;` (the example uses it directly).
pub use hyge_window::prelude::WindowConfig;
