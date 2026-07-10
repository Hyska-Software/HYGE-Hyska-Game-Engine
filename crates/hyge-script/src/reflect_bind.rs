//! Conversion between Lua values and reflected ECS components.

use bevy_ecs::{reflect::ReflectComponent, world::World};
use bevy_reflect::{
    serde::{ReflectDeserializer, ReflectSerializer},
    Reflect, TypeRegistry,
};
use hyge_core::prelude::{HygeError, HygeResult};
use mlua::{Lua, LuaSerdeExt, Value};
use serde::de::DeserializeSeed;
use serde_json::Value as JsonValue;

/// Serializes a reflected value into a Lua value.
pub fn reflect_to_lua(
    lua: &Lua,
    value: &dyn Reflect,
    registry: &TypeRegistry,
) -> HygeResult<Value> {
    let serialized = serde_json::to_value(ReflectSerializer::new(value, registry))
        .map_err(|error| HygeError::parse(format!("reflect serialize: {error}")))?;
    let unwrapped = unwrap_reflect_value(serialized);
    lua.to_value(&unwrapped)
        .map_err(|error| HygeError::parse(format!("Lua value conversion: {error}")))
}

/// Deserializes a Lua table into the registered reflected type.
pub fn lua_to_reflect(
    lua: &Lua,
    value: Value,
    type_path: &str,
    registry: &TypeRegistry,
) -> HygeResult<Box<dyn Reflect>> {
    let json: JsonValue = lua
        .from_value(value)
        .map_err(|error| HygeError::parse(format!("Lua component table: {error}")))?;
    let wrapped = serde_json::json!({ type_path: json });
    let text = serde_json::to_string(&wrapped)
        .map_err(|error| HygeError::parse(format!("reflect input: {error}")))?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    ReflectDeserializer::new(registry)
        .deserialize(&mut deserializer)
        .map_err(|error| HygeError::parse(format!("reflect deserialize {type_path}: {error}")))
}

/// Resolves a reflected ECS component registration by its short or full name.
pub fn reflect_component<'a>(
    registry: &'a TypeRegistry,
    type_name: &str,
) -> HygeResult<&'a ReflectComponent> {
    let registration = registry
        .get_with_short_type_path(type_name)
        .or_else(|| registry.get_with_type_path(type_name))
        .ok_or_else(|| {
            HygeError::InvalidArgument(format!("unknown reflected component {type_name:?}"))
        })?;
    registration.data::<ReflectComponent>().ok_or_else(|| {
        HygeError::InvalidArgument(format!("{type_name:?} is not a reflected ECS component"))
    })
}

/// Returns the value inside the type-path wrapper emitted by
/// `ReflectSerializer`.
fn unwrap_reflect_value(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) if object.len() == 1 => object
            .into_iter()
            .next()
            .map(|(_, value)| value)
            .unwrap_or(JsonValue::Null),
        other => other,
    }
}

/// Retrieves a reflected component and converts it to Lua.
pub fn get_component(
    lua: &Lua,
    world: &World,
    entity: bevy_ecs::entity::Entity,
    type_name: &str,
) -> HygeResult<Option<Value>> {
    let registry = world
        .get_resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .ok_or_else(|| HygeError::InvalidArgument("AppTypeRegistry is not installed".into()))?
        .0
        .read();
    let component = reflect_component(&registry, type_name)?;
    let Some(value) = component.reflect(world.entity(entity)) else {
        return Ok(None);
    };
    Ok(Some(reflect_to_lua(lua, value, &registry)?))
}

/// Applies or inserts a reflected component on an entity from Lua data.
pub fn set_component(
    lua: &Lua,
    world: &mut World,
    entity: bevy_ecs::entity::Entity,
    type_name: &str,
    value: Value,
) -> HygeResult<()> {
    if !world.entities().contains(entity) {
        return Err(HygeError::InvalidArgument(format!(
            "entity {entity:?} does not exist"
        )));
    }
    let registry_arc = world
        .get_resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .ok_or_else(|| HygeError::InvalidArgument("AppTypeRegistry is not installed".into()))?
        .0
        .clone();
    let registry = registry_arc.read();
    let registration = registry
        .get_with_short_type_path(type_name)
        .or_else(|| registry.get_with_type_path(type_name))
        .ok_or_else(|| {
            HygeError::InvalidArgument(format!("unknown reflected component {type_name:?}"))
        })?;
    let type_path = registration.type_info().type_path();
    let component = registration.data::<ReflectComponent>().ok_or_else(|| {
        HygeError::InvalidArgument(format!("{type_name:?} is not a reflected ECS component"))
    })?;
    let reflected = lua_to_reflect(lua, value, type_path, &registry)?;
    component.apply_or_insert(&mut world.entity_mut(entity), reflected.as_ref(), &registry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::reflect::AppTypeRegistry;
    use bevy_reflect::Reflect;
    use mlua::Lua;

    #[derive(bevy_ecs::component::Component, Reflect, Clone, Debug, PartialEq)]
    #[reflect(Component)]
    struct TestComponent {
        value: f32,
    }

    #[test]
    fn component_get_set_round_trip() {
        let lua = Lua::new();
        let mut registry = TypeRegistry::default();
        registry.register::<TestComponent>();
        let registry_arc = bevy_reflect::TypeRegistryArc::default();
        *registry_arc.write() = registry;
        let mut world = World::new();
        world.insert_resource(AppTypeRegistry(registry_arc));
        let entity = world.spawn(TestComponent { value: 2.0 }).id();
        let first = get_component(&lua, &world, entity, "TestComponent")
            .expect("get should work")
            .expect("component exists");
        lua.globals()
            .set("component", first)
            .expect("component should be registered");
        let value: f32 = lua
            .load("return component.value")
            .eval()
            .expect("Lua should read reflected value");
        assert_eq!(value, 2.0);
        let table = lua
            .load("return { value = 7.5 }")
            .eval()
            .expect("table should build");
        set_component(&lua, &mut world, entity, "TestComponent", table).expect("set should work");
        assert_eq!(
            world
                .get::<TestComponent>(entity)
                .expect("component exists")
                .value,
            7.5
        );
    }

    #[test]
    fn unknown_component_and_missing_entity_are_rejected() {
        let lua = Lua::new();
        let registry_arc = bevy_reflect::TypeRegistryArc::default();
        registry_arc.write().register::<TestComponent>();
        let mut world = World::new();
        world.insert_resource(AppTypeRegistry(registry_arc));
        let missing = bevy_ecs::entity::Entity::from_raw(99);
        let error =
            get_component(&lua, &world, missing, "Missing").expect_err("unknown type should fail");
        assert!(error.to_string().contains("unknown reflected component"));
        let value = lua
            .load("return { value = 1.0 }")
            .eval()
            .expect("table should build");
        let error = set_component(&lua, &mut world, missing, "TestComponent", value)
            .expect_err("missing entity should fail");
        assert!(error.to_string().contains("does not exist"));
    }
}
