//! R-063 — `.hyge-world` scene format.
//!
//! A [`WorldDocument`] is the on-disk representation of a Hyge scene. It
//! bundles an [`Environment`] descriptor, a flat list of [`PrefabInstance`]s
//! that should be spawned at load time, and a [`PostProcessProfile`] consumed
//! by the renderer.
//!
//! The file format is binary msgpack (via `rmp-serde`), mirroring the
//! `.hyge-prefab` format established in R-062. Content addressing is handled
//! by the asset layer (see [`crate::world_asset::WorldAsset`]) which hashes
//! the serialized bytes with BLAKE3.
//!
//! Loading a scene into a running ECS world is the responsibility of
//! [`WorldLoader::load`], which resolves every [`PrefabInstance`] by looking
//! up the referenced [`PrefabId`] in the [`TypeRegistry`] and instantiating
//! it via [`Prefab::instantiate`].
//!
//! > **Naming.** The architecture uses `struct World { … }`; that name
//! > clashes with [`bevy_ecs::world::World`], the ECS container. The struct
//! > here is therefore called `WorldDocument`.

use bevy_ecs::prelude::{Entity, World};
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::serde::ReflectDeserializer;
use bevy_reflect::{Reflect, TypeRegistry};
use serde::de::DeserializeSeed;
use serde::{Deserialize, Serialize};

use hyge_core::result::{HygeError, HygeResult};

use crate::components::{Children, Parent, Transform};
use crate::env::{Environment, PostProcessProfile};
use crate::prefab::{Prefab, SerializedComponentOverride};
use crate::prefab_id::PrefabId;
use hyge_ecs::AppTypeRegistry;

// =============================================================================
// PrefabInstance
// =============================================================================

/// One placed prefab inside a [`WorldDocument`].
///
/// The `parent` field uses a flat index into a sibling
/// `root_prefab_instances` slice rather than a typed handle, so the format
/// stays plain JSON/msgpack-serializable with no entity id dependencies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrefabInstance {
    /// Content-addressed prefab to instantiate.
    pub prefab: PrefabId,
    /// Root transform applied to the spawned prefab.
    pub transform: Transform,
    /// Index of the parent instance inside the same `root_prefab_instances`
    /// slice. `None` means this instance is a root of the scene hierarchy.
    pub parent: Option<u32>,
    /// Optional component overrides applied *on top* of whatever the prefab
    /// declares on its root node. Stored in serialized (reflect) form so the
    /// whole document is pure msgpack and does not require a live type
    /// registry to be stored on disk.
    pub overrides: Vec<SerializedComponentOverride>,
}

impl PrefabInstance {
    /// Builds a prefab instance with no overrides and no parent.
    #[must_use]
    pub fn new(prefab: PrefabId, transform: Transform) -> Self {
        Self {
            prefab,
            transform,
            parent: None,
            overrides: Vec::new(),
        }
    }

    /// Sets the parent instance index.
    #[must_use]
    pub fn with_parent(mut self, parent_index: u32) -> Self {
        self.parent = Some(parent_index);
        self
    }

    /// Pushes a serialized override onto the instance.
    #[must_use]
    pub fn with_override(mut self, override_: SerializedComponentOverride) -> Self {
        self.overrides.push(override_);
        self
    }
}

// =============================================================================
// WorldDocument
// =============================================================================

/// A `.hyge-world` document: scene environment, prefab instances and the
/// default post-process profile.
///
/// This is the principal on-disk scene type. It is content-addressed by the
/// asset layer using the BLAKE3 hash of [`WorldDocument::to_bytes`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldDocument {
    /// Scene-wide environment descriptor.
    pub env: Environment,
    /// Flat list of prefab instances to spawn at load time. Entries may
    /// reference siblings by index via [`PrefabInstance::parent`].
    pub root_prefab_instances: Vec<PrefabInstance>,
    /// Default post-process profile applied by the renderer.
    pub post_process: PostProcessProfile,
}

impl WorldDocument {
    /// Builds an empty world document with default environment and post-process.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            env: Environment::default(),
            root_prefab_instances: Vec::new(),
            post_process: PostProcessProfile::default(),
        }
    }

    /// Serializes the document to msgpack bytes.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] if `rmp-serde` fails to encode
    /// the structure. In practice the document only contains basic serde
    /// types so this should not happen on a well-formed value.
    pub fn to_bytes(&self) -> HygeResult<Vec<u8>> {
        rmp_serde::to_vec(self).map_err(|e| {
            HygeError::invalid_argument(format!("failed to serialize world document: {e}"))
        })
    }

    /// Deserializes a document from msgpack bytes.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Parse`] when the bytes are not valid msgpack or
    /// the expected structure is missing.
    pub fn from_bytes(bytes: &[u8]) -> HygeResult<Self> {
        rmp_serde::from_slice(bytes)
            .map_err(|e| HygeError::parse(format!("failed to deserialize world document: {e}")))
    }
}

// =============================================================================
// WorldLoader
// =============================================================================

/// Loads a [`WorldDocument`] into a running ECS [`World`].
///
/// `WorldLoader` is intentionally stateless; the heavy lifting lives in
/// [`Prefab::instantiate`] and the reflection-based override application.
pub struct WorldLoader;

impl WorldLoader {
    /// Spawns every [`PrefabInstance`] in `doc` into `world`, wiring up
    /// parent/child links and applying per-instance overrides.
    ///
    /// The `prefab_resolver` closure maps a [`PrefabId`] to the loaded
    /// [`Prefab`] that should be instantiated. Keeping the resolver out of
    /// `WorldLoader` lets the loader stay decoupled from the asset server —
    /// callers can fetch prefabs from the live cache, the SQLite `AssetDb`,
    /// or a test fixture.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] if the [`TypeRegistry`] is
    /// missing, a prefab cannot be resolved, a parent index is out of range,
    /// or a component override fails to deserialize.
    pub fn load<F>(
        doc: &WorldDocument,
        world: &mut World,
        prefab_resolver: F,
    ) -> HygeResult<Vec<Entity>>
    where
        F: Fn(&PrefabId) -> HygeResult<Prefab>,
    {
        let registry = world
            .get_resource::<AppTypeRegistry>()
            .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
            .clone();

        // Spawn every root prefab instance first, in declaration order, so
        // parent indices resolve to already-spawned entities.
        let mut root_entities: Vec<Entity> = Vec::with_capacity(doc.root_prefab_instances.len());
        for instance in &doc.root_prefab_instances {
            // Resolve the prefab. We do not yet cache it — the resolver is
            // allowed to fetch from the asset server every time.
            let prefab = prefab_resolver(&instance.prefab)?;
            // Validate overrides can hydrate before spawning so a bad override
            // surfaces as an error rather than silently skipping insertion.
            for override_ in &instance.overrides {
                hydrate_override(override_, &registry.read())?;
            }
            let entity = prefab.instantiate(world, instance.transform, None)?;
            // Apply per-instance overrides on top of the prefab root.
            apply_overrides(world, entity, &instance.overrides, &registry.read())?;
            root_entities.push(entity);
        }

        // Second pass: resolve parent links now that every root entity is
        // known. Parent indices are interpreted as sibling references inside
        // `doc.root_prefab_instances`.
        for (index, instance) in doc.root_prefab_instances.iter().enumerate() {
            if let Some(parent_index) = instance.parent {
                let parent_index = parent_index as usize;
                let parent_entity = root_entities.get(parent_index).ok_or_else(|| {
                    HygeError::invalid_argument(format!(
                        "prefab instance {index} references parent index \
                         {parent_index} which is out of range \
                         (have {} root instances)",
                        root_entities.len()
                    ))
                })?;
                let child_entity = root_entities[index];
                if *parent_entity == child_entity {
                    return Err(HygeError::invalid_argument(format!(
                        "prefab instance {index} lists itself as parent"
                    )));
                }
                wire_parent(world, *parent_entity, child_entity)?;
            }
        }

        Ok(root_entities)
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Validates that a serialized override can be hydrated using `registry`.
fn hydrate_override(
    override_: &SerializedComponentOverride,
    registry: &TypeRegistry,
) -> HygeResult<()> {
    let registration = registry
        .get_with_type_path(&override_.type_name)
        .ok_or_else(|| {
            HygeError::invalid_argument(format!(
                "component type '{}' not registered in TypeRegistry",
                override_.type_name
            ))
        })?;
    // Touch ReflectComponent to surface the friendly error early.
    let _ = registration.data::<ReflectComponent>().ok_or_else(|| {
        HygeError::invalid_argument(format!(
            "component type '{}' does not have ReflectComponent data; \
             did you forget #[reflect(Component)]?",
            override_.type_name
        ))
    })?;
    let _ = deserialize_override(override_, registry)?;
    Ok(())
}

/// Applies every override onto `entity` using the supplied type registry.
fn apply_overrides(
    world: &mut World,
    entity: Entity,
    overrides: &[SerializedComponentOverride],
    registry: &TypeRegistry,
) -> HygeResult<()> {
    for override_ in overrides {
        let registration = registry
            .get_with_type_path(&override_.type_name)
            .ok_or_else(|| {
                HygeError::invalid_argument(format!(
                    "component type '{}' not registered in TypeRegistry",
                    override_.type_name
                ))
            })?;
        let reflect_component = registration.data::<ReflectComponent>().ok_or_else(|| {
            HygeError::invalid_argument(format!(
                "component type '{}' does not have ReflectComponent data",
                override_.type_name
            ))
        })?;
        let value = deserialize_override(override_, registry)?;
        reflect_component.insert(&mut world.entity_mut(entity), value.as_ref(), registry);
    }
    Ok(())
}

/// Reparents `child` under `parent`, inserting `Parent` on the child and
/// appending the child to the parent's `Children`.
fn wire_parent(world: &mut World, parent: Entity, child: Entity) -> HygeResult<()> {
    world.entity_mut(child).insert(Parent(parent));
    if let Some(mut children) = world.get_mut::<Children>(parent) {
        if !children.0.contains(&child) {
            children.0.push(child);
        }
    } else {
        world.entity_mut(parent).insert(Children(vec![child]));
    }
    Ok(())
}

/// Deserializes a single serialized override into a runtime reflected value.
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
    use crate::components::{Name, PointLight};
    use crate::plugin::build_scene_type_registry;
    use crate::prefab::{Prefab, PrefabAssets, PrefabNode};

    fn test_registry() -> TypeRegistry {
        build_scene_type_registry()
    }

    fn sample_world_document() -> WorldDocument {
        let registry = test_registry();
        let prefab_bytes = {
            let mut root = PrefabNode::named("root");
            root.components.push(
                SerializedComponentOverride::new(
                    "hyge_scene::components::Name",
                    &Name::new("scene-prefab-root"),
                    &registry,
                )
                .expect("name serializes"),
            );
            let prefab = Prefab::new("sample", root, PrefabAssets::default());
            prefab.to_bytes().expect("serialize")
        };
        let prefab = Prefab::from_bytes(&prefab_bytes).expect("deserialize");
        let instance = PrefabInstance::new(prefab.prefab_id, Transform::identity()).with_override(
            SerializedComponentOverride::new(
                "hyge_scene::components::PointLight",
                &PointLight {
                    color: [1.0, 0.5, 0.25],
                    intensity: 7.0,
                    range: 5.0,
                },
                &registry,
            )
            .expect("override serialize"),
        );
        WorldDocument {
            env: Environment::empty(),
            root_prefab_instances: vec![instance],
            post_process: PostProcessProfile::default(),
        }
    }

    #[test]
    fn world_document_round_trip() {
        let doc = sample_world_document();
        let bytes = doc.to_bytes().expect("serialize");
        let restored = WorldDocument::from_bytes(&bytes).expect("deserialize");
        assert_eq!(doc, restored);
    }

    #[test]
    fn world_document_empty_round_trip() {
        let doc = WorldDocument::empty();
        let bytes = doc.to_bytes().expect("serialize");
        let restored = WorldDocument::from_bytes(&bytes).expect("deserialize");
        assert_eq!(doc, restored);
    }

    #[test]
    fn world_document_rejects_bad_bytes() {
        let err = WorldDocument::from_bytes(b"not msgpack").unwrap_err();
        assert!(matches!(err, HygeError::Parse(_)));
    }

    #[test]
    fn world_to_bytes_is_deterministic() {
        let doc = sample_world_document();
        let a = doc.to_bytes().expect("a");
        let b = doc.to_bytes().expect("b");
        assert_eq!(a, b);
    }

    #[test]
    fn world_loader_requires_type_registry() {
        let mut world = World::new();
        let doc = WorldDocument::empty();
        let err = WorldLoader::load(&doc, &mut world, |_| {
            Err(HygeError::invalid_argument("resolver should not be called"))
        })
        .unwrap_err();
        assert!(matches!(err, HygeError::InvalidArgument(_)));
    }

    #[test]
    fn world_loader_loads_into_ecs() {
        let mut world = World::new();
        let type_registry = AppTypeRegistry::default();
        *type_registry.write() = test_registry();
        world.insert_resource(type_registry);

        // Build a prefab the resolver can hand back to the loader.
        let mut prefab_root = PrefabNode::named("root");
        let test_registry_value = test_registry();
        prefab_root.components.push(
            SerializedComponentOverride::new(
                "hyge_scene::components::Name",
                &Name::new("loader-prefab-root"),
                &test_registry_value,
            )
            .expect("name serializes"),
        );
        let prefab = Prefab::new("loader", prefab_root, PrefabAssets::default());
        let prefab_id = prefab.prefab_id;

        let instance = PrefabInstance::new(prefab_id, Transform::identity());
        let doc = WorldDocument {
            env: Environment::empty(),
            root_prefab_instances: vec![instance],
            post_process: PostProcessProfile::default(),
        };

        let resolver = move |_id: &PrefabId| Ok(prefab.clone());
        let roots = WorldLoader::load(&doc, &mut world, resolver).expect("load");
        assert_eq!(roots.len(), 1);

        let entity_count = world
            .query::<bevy_ecs::prelude::Entity>()
            .iter(&world)
            .count();
        assert_eq!(entity_count, 1);

        let name = world.get::<Name>(roots[0]).expect("root has Name");
        assert_eq!(name.0, "loader-prefab-root");
    }
}
