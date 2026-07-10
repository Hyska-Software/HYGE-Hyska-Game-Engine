//! Lua state ownership and script evaluation.

use bevy_ecs::world::World;
use hyge_core::prelude::{HygeError, HygeResult};
use mlua::{Lua, Value};

use crate::{api, sandbox};

/// Owns one Lua state and evaluates scripts against a live ECS world.
pub struct ScriptEngine {
    lua: Lua,
    sandboxed: bool,
}

impl ScriptEngine {
    /// Creates a Lua engine. Production callers should keep `sandboxed=true`.
    pub fn new(sandboxed: bool) -> HygeResult<Self> {
        let lua = if sandboxed {
            sandbox::create_sandboxed_lua()
        } else {
            Lua::new_with(mlua::StdLib::ALL, mlua::LuaOptions::default())
        };
        lua.map(|lua| Self { lua, sandboxed })
            .map_err(|error| HygeError::parse(format!("Lua initialization: {error}")))
    }

    /// Returns whether this engine strips the unsafe Lua standard libraries.
    #[must_use]
    pub fn is_sandboxed(&self) -> bool {
        self.sandboxed
    }

    /// Returns the underlying Lua state for advanced registration.
    #[must_use]
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Evaluates a pure Lua chunk without the ECS API.
    pub fn eval(&self, source: &str) -> HygeResult<Value> {
        self.lua
            .load(source)
            .eval()
            .map_err(|error| HygeError::parse(format!("Lua evaluation: {error}")))
    }

    /// Evaluates a chunk with a scoped `hyge.*` API bound to `world`.
    pub fn execute_with_world(
        &self,
        world: &mut World,
        source: &str,
        delta: f32,
    ) -> HygeResult<Value> {
        self.lua
            .scope(|scope| {
                let hyge = api::install(&self.lua, scope, world, delta)?;
                self.lua.globals().set("hyge", hyge)?;
                self.lua.load(source).eval()
            })
            .map_err(|error| HygeError::parse(format!("Lua execution: {error}")))
    }
}

impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEngine")
            .field("sandboxed", &self.sandboxed)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ScriptAudio, ScriptEvents, ScriptInput};
    use bevy_ecs::{
        component::Component,
        reflect::{AppTypeRegistry, ReflectComponent},
        world::World,
    };
    use bevy_reflect::{Reflect, TypeRegistryArc};

    #[derive(Component, Reflect, Clone, Debug, PartialEq)]
    #[reflect(Component)]
    struct Position {
        value: f32,
    }

    #[test]
    fn scoped_hyge_api_reads_writes_and_emits_services() {
        let engine = ScriptEngine::new(true).expect("Lua should initialize");
        let registry = TypeRegistryArc::default();
        registry.write().register::<Position>();
        let mut world = World::new();
        world.insert_resource(AppTypeRegistry(registry));
        world.insert_resource(ScriptInput {
            actions: [("move".into(), 3.5)].into_iter().collect(),
        });
        world.insert_resource(ScriptAudio::default());
        world.insert_resource(ScriptEvents::default());
        let entity = world.spawn(Position { value: 1.0 }).id();
        let source = format!(
            r#"
            local current = hyge.world.get({}, "Position")
            assert(current.value == 1.0)
            hyge.world.set({}, "Position", {{ value = 9.0 }})
            assert(hyge.input.action("move") == 3.5)
            assert(hyge.time.delta() == 0.25)
            hyge.audio.play("laser")
            hyge.event.send("hit", {{ damage = 2 }})
        "#,
            entity.to_bits(),
            entity.to_bits()
        );
        engine
            .execute_with_world(&mut world, &source, 0.25)
            .expect("script API should execute");
        assert_eq!(
            world
                .get::<Position>(entity)
                .expect("position exists")
                .value,
            9.0
        );
        assert_eq!(world.resource::<ScriptAudio>().played, vec!["laser"]);
        assert_eq!(world.resource::<ScriptEvents>().events[0].name, "hit");
        assert_eq!(
            world.resource::<ScriptEvents>().events[0].payload["damage"],
            2
        );
    }
}
