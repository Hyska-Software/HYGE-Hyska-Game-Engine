//! ECS registration for scripting.

use std::path::PathBuf;

use hyge_ecs::prelude::*;

use crate::{
    api::{ScriptAudio, ScriptEvents, ScriptInput, ScriptTime},
    engine::ScriptEngine,
    events::ScriptError,
    hot_reload::{process_script_hot_reload, ScriptState, ScriptWatcher},
};

/// Application configuration for the Lua runtime.
#[derive(Clone, Copy, Debug)]
pub struct ScriptConfig {
    /// Enables script execution resources.
    pub enabled: bool,
    /// Strips unsafe Lua standard libraries.
    pub sandbox: bool,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sandbox: true,
        }
    }
}

/// Runtime resource holding the configured Lua engine.
#[derive(Resource, Debug)]
pub struct ScriptRuntime {
    /// Lua engine, absent only when initialization failed.
    pub engine: Option<ScriptEngine>,
}

/// Registers script resources, errors, and the runtime engine.
#[derive(Clone, Debug)]
pub struct ScriptPlugin {
    config: ScriptConfig,
    project_root: PathBuf,
}

impl ScriptPlugin {
    /// Creates a plugin with explicit runtime configuration.
    #[must_use]
    pub fn new(config: ScriptConfig) -> Self {
        Self {
            config,
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// Uses an explicit project root for script discovery and hot-reload.
    #[must_use]
    pub fn with_project_root(mut self, project_root: impl Into<PathBuf>) -> Self {
        self.project_root = project_root.into();
        self
    }
}

impl HygePlugin for ScriptPlugin {
    fn name(&self) -> &'static str {
        "hyge-script"
    }

    fn build(&self, app: &mut bevy_app::App) {
        app.add_event::<ScriptError>();
        app.init_resource::<ScriptTime>();
        app.init_resource::<ScriptInput>();
        app.init_resource::<ScriptAudio>();
        app.init_resource::<ScriptEvents>();
        app.init_resource::<ScriptState>();
        let engine = if self.config.enabled {
            ScriptEngine::new(self.config.sandbox).ok()
        } else {
            None
        };
        app.insert_resource(ScriptRuntime { engine });
        if self.config.enabled {
            match ScriptWatcher::new(&self.project_root) {
                Ok(watcher) => {
                    app.insert_resource(watcher);
                    app.add_systems(Label::Update, process_script_hot_reload);
                }
                Err(error) => {
                    tracing::warn!(%error, "script hot-reload watcher unavailable");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::event::Events;

    #[test]
    fn plugin_registers_runtime_resources_and_error_event() {
        let mut app = bevy_app::App::new();
        ScriptPlugin::new(ScriptConfig {
            enabled: true,
            sandbox: true,
        })
        .build(&mut app);
        assert!(app.world().resource::<ScriptRuntime>().engine.is_some());
        assert!(app.world().get_resource::<ScriptTime>().is_some());
        assert!(app.world().get_resource::<ScriptInput>().is_some());
        assert!(app.world().get_resource::<ScriptAudio>().is_some());
        assert!(app.world().get_resource::<ScriptEvents>().is_some());
        assert!(app.world().get_resource::<Events<ScriptError>>().is_some());
    }

    #[test]
    fn disabled_plugin_keeps_runtime_explicitly_disabled() {
        let mut app = bevy_app::App::new();
        ScriptPlugin::new(ScriptConfig {
            enabled: false,
            sandbox: true,
        })
        .build(&mut app);
        assert!(app.world().resource::<ScriptRuntime>().engine.is_none());
    }
}
