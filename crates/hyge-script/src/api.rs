//! The scoped `hyge.*` Lua API.

use std::{cell::RefCell, rc::Rc};

use bevy_ecs::{entity::Entity, world::World};
use hyge_core::prelude::HygeError;
use mlua::{Lua, LuaSerdeExt, Scope, Table, Value};

use crate::reflect_bind::{get_component, set_component};

/// Runtime clock exposed through `hyge.time.delta()`.
#[derive(hyge_ecs::Resource, Clone, Copy, Debug)]
pub struct ScriptTime {
    /// Duration of the current variable update in seconds.
    pub delta: f32,
}

impl Default for ScriptTime {
    fn default() -> Self {
        Self { delta: 1.0 / 60.0 }
    }
}

/// Numeric action values exposed through `hyge.input.action(name)`.
#[derive(hyge_ecs::Resource, Clone, Debug, Default)]
pub struct ScriptInput {
    /// Current action values by name.
    pub actions: std::collections::HashMap<String, f32>,
}

/// Audio requests emitted through `hyge.audio.play(handle)`.
#[derive(hyge_ecs::Resource, Clone, Debug, Default)]
pub struct ScriptAudio {
    /// Handles requested by scripts in FIFO order.
    pub played: Vec<String>,
}

/// An event emitted through `hyge.event.send(name, payload)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptEventRecord {
    /// Event name.
    pub name: String,
    /// JSON-compatible payload captured from the Lua table.
    pub payload: serde_json::Value,
}

/// Event queue exposed to the host application.
#[derive(hyge_ecs::Resource, Clone, Debug, Default)]
pub struct ScriptEvents {
    /// Events emitted since the last consumer drain.
    pub events: Vec<ScriptEventRecord>,
}

/// Installs the scoped API used for one script execution.
pub fn install<'scope, 'env>(
    lua: &Lua,
    scope: &'scope mut Scope<'scope, 'env>,
    world: &'env mut World,
    delta: f32,
) -> mlua::Result<Table> {
    let world_cell = Rc::new(RefCell::new(world));
    let hyge = lua.create_table()?;

    let log = lua.create_function(|_, (level, message): (String, String)| {
        match level.to_ascii_lowercase().as_str() {
            "error" => tracing::error!(target: "hyge.script", "{message}"),
            "warn" => tracing::warn!(target: "hyge.script", "{message}"),
            "debug" => tracing::debug!(target: "hyge.script", "{message}"),
            _ => tracing::info!(target: "hyge.script", "{message}"),
        }
        Ok(())
    })?;
    hyge.set("log", log)?;

    let world_table = lua.create_table()?;
    let entity_table = lua.create_table()?;
    let world_for_new = Rc::clone(&world_cell);
    entity_table.set(
        "new",
        scope.create_function_mut(move |_, ()| {
            Ok(world_for_new.borrow_mut().spawn_empty().id().to_bits())
        })?,
    )?;
    let world_for_destroy = Rc::clone(&world_cell);
    entity_table.set(
        "destroy",
        scope.create_function_mut(move |_, entity: u64| {
            let entity = Entity::from_bits(entity);
            if world_for_destroy.borrow_mut().despawn(entity) {
                Ok(true)
            } else {
                Err(mlua::Error::external(format!(
                    "entity {entity:?} does not exist"
                )))
            }
        })?,
    )?;
    world_table.set("entity", entity_table)?;

    let world_for_get = Rc::clone(&world_cell);
    world_table.set(
        "get",
        scope.create_function(move |lua, (entity, type_name): (u64, String)| {
            let world_ref = world_for_get.borrow();
            match get_component(lua, &world_ref, Entity::from_bits(entity), &type_name)
                .map_err(mlua::Error::external)?
            {
                Some(value) => Ok(value),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    let world_for_set = Rc::clone(&world_cell);
    world_table.set(
        "set",
        scope.create_function_mut(
            move |lua, (entity, type_name, value): (u64, String, Value)| {
                set_component(
                    lua,
                    &mut world_for_set.borrow_mut(),
                    Entity::from_bits(entity),
                    &type_name,
                    value,
                )
                .map_err(mlua::Error::external)
            },
        )?,
    )?;
    hyge.set("world", world_table)?;

    let world_for_events = Rc::clone(&world_cell);
    let event_table = lua.create_table()?;
    event_table.set(
        "send",
        scope.create_function_mut(move |lua, (name, value): (String, Value)| {
            let payload = lua
                .from_value(value)
                .map_err(|error| mlua::Error::external(format!("event payload: {error}")))?;
            world_for_events
                .borrow_mut()
                .get_resource_or_insert_with(ScriptEvents::default)
                .events
                .push(ScriptEventRecord { name, payload });
            Ok(())
        })?,
    )?;
    hyge.set("event", event_table)?;

    let world_for_input = Rc::clone(&world_cell);
    let input_table = lua.create_table()?;
    input_table.set(
        "action",
        scope.create_function(move |_, name: String| {
            Ok(world_for_input
                .borrow()
                .get_resource::<ScriptInput>()
                .and_then(|input| input.actions.get(&name).copied())
                .unwrap_or_default())
        })?,
    )?;
    hyge.set("input", input_table)?;

    let world_for_audio = Rc::clone(&world_cell);
    let audio_table = lua.create_table()?;
    audio_table.set(
        "play",
        scope.create_function_mut(move |_, handle: String| {
            world_for_audio
                .borrow_mut()
                .get_resource_or_insert_with(ScriptAudio::default)
                .played
                .push(handle);
            Ok(())
        })?,
    )?;
    hyge.set("audio", audio_table)?;

    let time_table = lua.create_table()?;
    time_table.set("delta", lua.create_function(move |_, ()| Ok(delta))?)?;
    hyge.set("time", time_table)?;

    Ok(hyge)
}

/// Converts an engine error into a Lua external error without losing context.
pub fn script_error(error: HygeError) -> mlua::Error {
    mlua::Error::external(error)
}
