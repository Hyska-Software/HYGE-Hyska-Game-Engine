//! ECS registration for scripting.

use hyge_ecs::prelude::*;

use crate::{
    api::{ScriptAudio, ScriptEvents, ScriptInput, ScriptTime},
    engine::ScriptEngine,
    events::ScriptError,
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
#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptPlugin {
    config: ScriptConfig,
}

impl ScriptPlugin {
    /// Creates a plugin with explicit runtime configuration.
    #[must_use]
    pub fn new(config: ScriptConfig) -> Self {
        Self { config }
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
        let engine = if self.config.enabled {
            ScriptEngine::new(self.config.sandbox).ok()
        } else {
            None
        };
        app.insert_resource(ScriptRuntime { engine });
    }
}
