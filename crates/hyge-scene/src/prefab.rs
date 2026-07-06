//! R-062 — Prefab system: hierarchical, content-addressed entity templates.
//!
//! A [`Prefab`] is a serializable entity hierarchy. It is stored on disk as
//! `.hyge-prefab` (msgpack) and instantiated into a [`World`] at runtime.
//! Component overrides are reflect-based, so any component registered in the
//! [`TypeRegistry`] can be baked into a prefab and restored on instantiation.

use bevy_ecs::prelude::{Entity, World};
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::serde::{ReflectDeserializer, ReflectSerializer};
use bevy_reflect::{Reflect, TypeRegistry};
use hyge_ecs::AppTypeRegistry;
use serde::de::DeserializeSeed;
use serde::{Deserialize, Serialize};

use hyge_asset::AssetId;
use hyge_core::result::{HygeError, HygeResult};

use crate::components::{Children, GlobalTransform, Name, Parent, PersistOnReload, Transform};
use crate::prefab_id::PrefabId;

/// Asset references stored inside a prefab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrefabAssets {
    /// Mesh assets referenced by this prefab.
    pub meshes: Vec<AssetId>,
    /// Material assets referenced by this prefab.
    pub materials: Vec<AssetId>,
    /// Script assets referenced by this prefab.
    pub scripts: Vec<AssetId>,
}

/// Persistent (msgpack-serializable) form of a component override.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerializedComponentOverride {
    /// Fully-qualified type path used for [`TypeRegistry`] lookup.
    pub type_name: String,
    /// Msgpack bytes produced by [`ReflectSerializer`].
    pub data: Vec<u8>,
}

impl SerializedComponentOverride {
    /// Serializes a reflected component value using the given registry.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] when serialization to msgpack
    /// fails.
    pub fn new(
        type_name: impl Into<String>,
        value: &dyn Reflect,
        registry: &TypeRegistry,
    ) -> HygeResult<Self> {
        let serializer = ReflectSerializer::new(value, registry);
        let data = rmp_serde::to_vec(&serializer).map_err(|e| {
            HygeError::invalid_argument(format!("failed to serialize component override: {e}"))
        })?;
        Ok(Self {
            type_name: type_name.into(),
            data,
        })
    }
}

/// Runtime (in-memory) form of a component override.
#[derive(Debug)]
pub struct ComponentOverride {
    /// Fully-qualified type path.
    pub type_name: String,
    /// Reflected value. Must implement `Component` to be inserted into a
    /// [`World`].
    pub value: Box<dyn Reflect>,
}

impl ComponentOverride {
    /// Creates a runtime override from a reflected value.
    #[must_use]
    pub fn new(type_name: impl Into<String>, value: Box<dyn Reflect>) -> Self {
        Self {
            type_name: type_name.into(),
            value,
        }
    }
}

/// A node in the prefab hierarchy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrefabNode {
    /// Human-readable node name.
    pub name: String,
    /// Serialized component overrides for this node.
    pub components: Vec<SerializedComponentOverride>,
    /// Child nodes.
    pub children: Vec<PrefabNode>,
    /// If true, the spawned entity receives [`PersistOnReload`].
    pub persist: bool,
}

impl PrefabNode {
    /// Creates a leaf node with the given name.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Recursively validates that every serialized override can be hydrated
    /// using `registry`.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] if a type is not registered or
    /// if deserialization of any override fails.
    pub fn hydrate(&self, registry: &TypeRegistry) -> HygeResult<()> {
        for override_ in &self.components {
            deserialize_override(override_, registry)?;
        }
        for child in &self.children {
            child.hydrate(registry)?;
        }
        Ok(())
    }
}

/// Internal serializable representation used to compute the content hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SerializablePrefab {
    name: String,
    root: PrefabNode,
    assets: PrefabAssets,
}

/// A content-addressed hierarchical prefab.
#[derive(Debug, Clone)]
pub struct Prefab {
    /// Human-readable prefab name.
    pub name: String,
    /// Root node of the hierarchy.
    pub root: PrefabNode,
    /// Asset references used by the prefab.
    pub assets: PrefabAssets,
    /// BLAKE3 hash of the serialized msgpack bytes.
    pub prefab_id: PrefabId,
}

impl Prefab {
    /// Creates a prefab from its parts and computes its content-addressed id.
    #[must_use]
    pub fn new(name: impl Into<String>, root: PrefabNode, assets: PrefabAssets) -> Self {
        let name = name.into();
        let serializable = SerializablePrefab {
            name: name.clone(),
            root: root.clone(),
            assets: assets.clone(),
        };
        // Serializing a serializable prefab only fails if rmp-serde itself is
        // broken; the structure contains only basic serde types.
        let bytes = rmp_serde::to_vec(&serializable)
            .expect("serializable prefab should always encode to msgpack");
        let prefab_id = PrefabId::compute(&bytes);
        Self {
            name,
            root,
            assets,
            prefab_id,
        }
    }

    /// Serializes to msgpack bytes.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] when serialization fails.
    pub fn to_bytes(&self) -> HygeResult<Vec<u8>> {
        let serializable = SerializablePrefab {
            name: self.name.clone(),
            root: self.root.clone(),
            assets: self.assets.clone(),
        };
        rmp_serde::to_vec(&serializable)
            .map_err(|e| HygeError::invalid_argument(format!("failed to serialize prefab: {e}")))
    }

    /// Deserializes from msgpack bytes and computes the prefab id from the
    /// raw bytes.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Parse`] when the bytes are not valid msgpack or
    /// the expected structure is missing.
    pub fn from_bytes(bytes: &[u8]) -> HygeResult<Self> {
        let serializable: SerializablePrefab = rmp_serde::from_slice(bytes)
            .map_err(|e| HygeError::parse(format!("failed to deserialize prefab: {e}")))?;
        let prefab_id = PrefabId::compute(bytes);
        Ok(Self {
            name: serializable.name,
            root: serializable.root,
            assets: serializable.assets,
            prefab_id,
        })
    }

    /// Validates that every serialized override can be hydrated using
    /// `registry`.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] if a type is not registered or
    /// if deserialization of any override fails.
    pub fn hydrate(&self, registry: &TypeRegistry) -> HygeResult<()> {
        self.root.hydrate(registry)
    }

    /// Instantiates the prefab into the given world and returns the root
    /// entity.
    ///
    /// This method reads the [`TypeRegistry`] resource from `world`, so the
    /// registry must be registered before calling this function. The root
    /// entity receives the supplied `transform`; children inherit it through
    /// the transform propagation system.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] if the type registry is missing,
    /// a component type is not registered, or a component override cannot be
    /// deserialized.
    pub fn instantiate(
        &self,
        world: &mut World,
        transform: Transform,
        parent: Option<Entity>,
    ) -> HygeResult<Entity> {
        let registry = world
            .get_resource::<AppTypeRegistry>()
            .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
            .clone();

        let mut links: Vec<(Entity, Entity)> = Vec::new();
        let root = self.spawn_node(world, &registry, &self.root, transform, parent, &mut links)?;

        // Apply parent -> children links after the recursive spawn so we do
        // not need to borrow the world mutably while recursion is ongoing.
        for (parent_entity, child_entity) in links {
            if let Some(mut children) = world.get_mut::<Children>(parent_entity) {
                if !children.0.contains(&child_entity) {
                    children.0.push(child_entity);
                }
            } else {
                world
                    .entity_mut(parent_entity)
                    .insert(Children(vec![child_entity]));
            }
        }

        Ok(root)
    }

    fn spawn_node(
        &self,
        world: &mut World,
        registry: &AppTypeRegistry,
        node: &PrefabNode,
        transform: Transform,
        parent: Option<Entity>,
        links: &mut Vec<(Entity, Entity)>,
    ) -> HygeResult<Entity> {
        let entity_id = {
            let mut entity_builder = world.spawn((
                Name::new(node.name.clone()),
                transform,
                GlobalTransform::identity(),
            ));

            if node.persist {
                entity_builder.insert(PersistOnReload);
            }

            if let Some(parent_entity) = parent {
                entity_builder.insert(Parent(parent_entity));
                links.push((parent_entity, entity_builder.id()));
            }

            entity_builder.id()
        };

        for override_ in &node.components {
            let type_registry = registry.read();
            let registration = type_registry
                .get_with_type_path(&override_.type_name)
                .ok_or_else(|| {
                    HygeError::invalid_argument(format!(
                        "component type '{}' not registered in TypeRegistry",
                        override_.type_name
                    ))
                })?;
            let reflect_component = registration.data::<ReflectComponent>().ok_or_else(|| {
                HygeError::invalid_argument(format!(
                    "component type '{}' does not have ReflectComponent data; \
                     did you forget #[reflect(Component)]?",
                    override_.type_name
                ))
            })?;
            let value = deserialize_override(override_, &type_registry)?;
            reflect_component.insert(
                &mut world.entity_mut(entity_id),
                value.as_ref(),
                &type_registry,
            );
        }

        for child in &node.children {
            self.spawn_node(
                world,
                registry,
                child,
                Transform::identity(),
                Some(entity_id),
                links,
            )?;
        }

        Ok(entity_id)
    }
}

fn deserialize_override(
    override_: &SerializedComponentOverride,
    registry: &TypeRegistry,
) -> HygeResult<Box<dyn Reflect>> {
    let mut deserializer = rmp_serde::Deserializer::new(override_.data.as_slice());
    let reflect_de = ReflectDeserializer::new(registry);
    DeserializeSeed::deserialize(reflect_de, &mut deserializer).map_err(|e| {
        HygeError::invalid_argument(format!(
            "failed to deserialize component override for '{}': {e}",
            override_.type_name
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Name, PointLight, Transform};
    use bevy_ecs::prelude::World;
    use bevy_reflect::TypeRegistry;
    use hyge_ecs::AppTypeRegistry;

    fn test_registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        registry.register::<Name>();
        registry.register::<Transform>();
        registry.register::<GlobalTransform>();
        registry.register::<Parent>();
        registry.register::<Children>();
        registry.register::<PersistOnReload>();
        registry.register::<PointLight>();
        registry
    }

    fn sample_prefab() -> Prefab {
        let registry = test_registry();
        let mut root = PrefabNode::named("root");
        root.components.push(
            SerializedComponentOverride::new(
                "hyge_scene::components::Name",
                &Name::new("prefab-root"),
                &registry,
            )
            .expect("name serializes"),
        );
        root.components.push(
            SerializedComponentOverride::new(
                "hyge_scene::components::PointLight",
                &PointLight {
                    color: [1.0, 0.5, 0.25],
                    intensity: 42.0,
                    range: 10.0,
                },
                &registry,
            )
            .expect("light serializes"),
        );

        let mut child = PrefabNode::named("child");
        child.persist = true;
        child.components.push(
            SerializedComponentOverride::new(
                "hyge_scene::components::Name",
                &Name::new("prefab-child"),
                &registry,
            )
            .expect("name serializes"),
        );
        root.children.push(child);

        Prefab::new(
            "sample",
            root,
            PrefabAssets {
                meshes: vec![AssetId::from(blake3::hash(b"mesh"))],
                materials: vec![AssetId::from(blake3::hash(b"mat"))],
                scripts: Vec::new(),
            },
        )
    }

    #[test]
    fn prefab_roundtrip_serialization() {
        let original = sample_prefab();
        let bytes = original.to_bytes().expect("serialize");
        let deserialized = Prefab::from_bytes(&bytes).expect("deserialize");

        assert_eq!(original.name, deserialized.name);
        assert_eq!(original.root.name, deserialized.root.name);
        assert_eq!(original.root.persist, deserialized.root.persist);
        assert_eq!(
            original.root.children.len(),
            deserialized.root.children.len()
        );
        assert_eq!(original.assets, deserialized.assets);
        assert_eq!(original.prefab_id, deserialized.prefab_id);
    }

    #[test]
    fn prefab_blake3_id_matches_content() {
        let prefab = sample_prefab();
        let bytes = prefab.to_bytes().expect("serialize");
        assert_eq!(prefab.prefab_id, PrefabId::compute(&bytes));
    }

    #[test]
    fn prefab_hydrate_and_instantiate() {
        let mut world = World::new();
        let type_registry = AppTypeRegistry::default();
        *type_registry.write() = test_registry();
        world.insert_resource(type_registry);

        let prefab = sample_prefab();
        prefab
            .hydrate(&world.get_resource::<AppTypeRegistry>().unwrap().read())
            .expect("hydrate");

        let root = prefab
            .instantiate(&mut world, Transform::identity(), None)
            .expect("instantiate");

        // Root + one child = 2 entities.
        let entity_count = world
            .query::<bevy_ecs::prelude::Entity>()
            .iter(&world)
            .count();
        assert_eq!(entity_count, 2);

        let name = world.get::<Name>(root).expect("root has Name");
        assert_eq!(name.0, "prefab-root");

        // Verify child exists and has Parent -> root.
        let child_entity = world
            .query::<(&Parent, &Name)>()
            .iter(&world)
            .find(|(parent, _)| parent.0 == root)
            .map(|(_, name)| name.0.clone())
            .expect("child linked to root");
        assert_eq!(child_entity, "prefab-child");

        // Verify root has Children containing the child.
        let children = world.get::<Children>(root).expect("root has Children");
        assert_eq!(children.0.len(), 1);
    }

    #[test]
    fn prefab_persist_flag_inserts_marker() {
        let mut world = World::new();
        let type_registry = AppTypeRegistry::default();
        *type_registry.write() = test_registry();
        world.insert_resource(type_registry);

        let prefab = sample_prefab();
        prefab
            .instantiate(&mut world, Transform::identity(), None)
            .expect("instantiate");

        let child_entity = world
            .query::<(Entity, &PersistOnReload)>()
            .iter(&world)
            .next()
            .map(|(entity, _)| entity)
            .expect("persist marker present");
        assert!(world.get::<Name>(child_entity).is_some());
    }

    #[test]
    fn prefab_instantiate_requires_type_registry() {
        let mut world = World::new();
        let prefab = sample_prefab();
        let err = prefab
            .instantiate(&mut world, Transform::identity(), None)
            .unwrap_err();
        assert!(matches!(err, HygeError::InvalidArgument(_)));
    }
}
