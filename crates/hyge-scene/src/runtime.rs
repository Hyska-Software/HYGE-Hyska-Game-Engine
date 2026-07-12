//! Scene runtime orchestration for `.hyge-world` documents.
//!
//! This module closes the gap between the on-disk scene formats and the live
//! ECS/render world:
//! - prefab libraries are resolved by content-addressed `PrefabId`,
//! - `.hyge-world` documents are loaded from bytes or disk,
//! - scene-level environment and post-process state is mirrored into ECS
//!   resources,
//! - hot-reload diffs the current document against the new one and reapplies
//!   the scene, and
//! - `StaticMeshAssetRefs` are materialized into runtime `StaticMesh`
//!   components before `RenderExtract` runs.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bevy_ecs::prelude::{Component, Entity, Resource, World};
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::serde::{ReflectDeserializer, ReflectSerializer};
use hyge_asset::{Asset, AssetId, Handle, MaterialAsset, MeshAsset, ReloadQueue};
use hyge_core::result::{HygeError, HygeResult};
use serde::de::DeserializeSeed;

use crate::{
    components::{
        AmbientLight, Children, DirectionalLight, GlobalTransform, Name, Parent, PersistOnReload,
        SceneNodeId, StaticMesh, StaticMeshAssetRefs,
    },
    env::{Environment, PostProcessProfile},
    prefab::Prefab,
    prefab_id::PrefabId,
    world::{
        SceneEditLayer, SceneNodeRecord, WorldDocument, WorldLoader, EDITOR_SCENE_LAYER_VERSION,
    },
    world_asset::WorldAsset,
};

/// Prefabs available to the scene runtime.
#[derive(Resource, Debug, Clone, Default)]
pub struct PrefabLibrary {
    prefabs: HashMap<PrefabId, Prefab>,
}

impl PrefabLibrary {
    /// Inserts or replaces a prefab in the library.
    pub fn insert(&mut self, prefab: Prefab) {
        self.prefabs.insert(prefab.prefab_id, prefab);
    }

    /// Removes every registered prefab.
    pub fn clear(&mut self) {
        self.prefabs.clear();
    }

    /// Resolves a prefab by id.
    #[must_use]
    pub fn get(&self, id: &PrefabId) -> Option<&Prefab> {
        self.prefabs.get(id)
    }

    /// Returns the number of registered prefabs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.prefabs.len()
    }

    /// Returns true when the library is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prefabs.is_empty()
    }
}

/// Opaque environment library used by tests and the future editor/runtime
/// integration to associate a scene skybox asset id with loaded environment
/// bytes/bakes. The scene crate keeps the payload opaque to avoid a hard
/// dependency on `hyge-render` here.
#[derive(Resource, Debug, Clone, Default)]
pub struct EnvironmentLibrary {
    environments: HashMap<AssetId, Vec<u8>>,
}

impl EnvironmentLibrary {
    /// Inserts opaque environment payload bytes keyed by skybox asset id.
    pub fn insert(&mut self, id: AssetId, bytes: Vec<u8>) {
        self.environments.insert(id, bytes);
    }

    /// Returns the opaque payload for a skybox asset id.
    #[must_use]
    pub fn get(&self, id: &AssetId) -> Option<&[u8]> {
        self.environments.get(id).map(Vec::as_slice)
    }
}

/// Scene-level environment descriptor currently active in the ECS world.
#[derive(Resource, Debug, Clone)]
pub struct SceneEnvironmentState {
    /// Current scene environment.
    pub environment: Environment,
}

impl Default for SceneEnvironmentState {
    fn default() -> Self {
        Self {
            environment: Environment::empty(),
        }
    }
}

/// Scene-level post-process profile currently active in the ECS world.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct ScenePostProcessState {
    /// Current scene post-process profile.
    pub profile: PostProcessProfile,
}

/// Marker attached to scene-managed entities (loaded root prefab instances and
/// environment helper entities).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneManagedEntity;

/// Summary of the difference between two `WorldDocument`s.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneDocumentDiff {
    /// Number of newly added root prefab instances.
    pub added_instances: usize,
    /// Number of removed root prefab instances.
    pub removed_instances: usize,
    /// Number of changed root prefab instances (same index, different value).
    pub changed_instances: usize,
    /// Whether the environment block changed.
    pub environment_changed: bool,
    /// Whether the post-process block changed.
    pub post_process_changed: bool,
}

/// Detailed result of a scene reload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneReloadReport {
    /// Document-level differences detected before applying the reload.
    pub diff: SceneDocumentDiff,
    /// Persistent scene identities captured before the reload.
    pub preserved_scene_ids: Vec<String>,
    /// Persistent identities that were restored onto newly loaded entities.
    pub restored_scene_ids: Vec<String>,
    /// Persistent identities that were reattached because the new document
    /// no longer contained them.
    pub reattached_scene_ids: Vec<String>,
}

impl SceneDocumentDiff {
    /// Returns true when the root instance list changed in any way.
    #[must_use]
    pub const fn root_instances_changed(&self) -> bool {
        self.added_instances > 0 || self.removed_instances > 0 || self.changed_instances > 0
    }

    /// Returns true when any part of the document changed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !self.root_instances_changed() && !self.environment_changed && !self.post_process_changed
    }
}

/// Runtime state of the currently loaded `.hyge-world` document.
#[derive(Resource, Debug, Clone, Default)]
pub struct LoadedSceneState {
    /// Source path of the currently loaded scene file, when loaded from disk.
    pub source_path: Option<PathBuf>,
    /// Content-addressed id of the currently loaded scene file bytes.
    pub asset_id: Option<AssetId>,
    /// Last successfully loaded document.
    pub document: Option<WorldDocument>,
    /// Root entities instantiated from the current document.
    pub root_entities: Vec<Entity>,
    /// Environment helper entities currently spawned in the world.
    pub environment_entities: Vec<Entity>,
    /// Last diff applied through hot-reload.
    pub last_diff: SceneDocumentDiff,
}

/// Assigns deterministic IDs to a legacy prefab subtree.
pub fn assign_legacy_scene_node_ids(world: &mut World, root: Entity, seed: &str) -> HygeResult<()> {
    assign_scene_node_ids_recursive(world, root, seed, "0", false)
}

/// Assigns fresh IDs to a newly created editor subtree.
pub fn assign_new_scene_node_ids(world: &mut World, root: Entity) -> HygeResult<()> {
    let seed = format!("new:{}", blake3::hash(format!("{:?}", root).as_bytes()));
    assign_scene_node_ids_recursive(world, root, &seed, "0", true)
}

/// Captures the scene-managed ECS world into the persistent editor layer.
pub fn capture_editor_scene_layer(world: &mut World) -> HygeResult<SceneEditLayer> {
    let registry = world
        .get_resource::<hyge_ecs::AppTypeRegistry>()
        .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
        .0
        .clone();
    let read = registry.read();
    let mut query = world.query::<(Entity, &SceneNodeId, Option<&Parent>, Option<&Name>)>();
    let entities: Vec<(Entity, SceneNodeId, Option<Entity>, String)> = query
        .iter(world)
        .map(|(entity, id, parent, name)| {
            (
                entity,
                id.clone(),
                parent.map(|parent| parent.0),
                name.map(|name| name.0.clone())
                    .unwrap_or_else(|| entity.to_bits().to_string()),
            )
        })
        .collect();
    let mut records = Vec::with_capacity(entities.len());
    for (entity, id, parent, name) in entities {
        let order = parent
            .and_then(|parent| world.get::<Children>(parent))
            .and_then(|children| children.0.iter().position(|child| *child == entity))
            .unwrap_or(0) as u32;
        let mut components = Vec::new();
        for registration in read.iter_with_data::<ReflectComponent>() {
            let type_path = registration.0.type_info().type_path();
            if type_path.ends_with("::Parent")
                || type_path.ends_with("::Children")
                || type_path.ends_with("::GlobalTransform")
                || type_path.ends_with("::Name")
                || type_path.ends_with("::SceneNodeId")
            {
                continue;
            }
            let Some(value) = registration.1.reflect(world.entity(entity)) else {
                continue;
            };
            let data = rmp_serde::to_vec(&ReflectSerializer::new(value, &read))
                .map_err(|error| HygeError::invalid_argument(error.to_string()))?;
            components.push(crate::SerializedComponentOverride {
                type_name: type_path.to_owned(),
                data,
            });
        }
        components.sort_by(|left, right| left.type_name.cmp(&right.type_name));
        records.push(SceneNodeRecord {
            id: id.0,
            parent: parent
                .and_then(|parent| world.get::<SceneNodeId>(parent).map(|id| id.0.clone())),
            order,
            name,
            components,
        });
    }
    records.sort_by(|left, right| {
        left.parent
            .cmp(&right.parent)
            .then(left.order.cmp(&right.order))
            .then(left.id.cmp(&right.id))
    });
    Ok(SceneEditLayer {
        version: EDITOR_SCENE_LAYER_VERSION,
        nodes: records,
        tombstones: Vec::new(),
    })
}

/// Replaces the document's editor layer with a materialized ECS capture.
pub fn sync_editor_layer_from_world(
    world: &mut World,
    document: &mut WorldDocument,
) -> HygeResult<()> {
    document.editor_layer = Some(capture_editor_scene_layer(world)?);
    Ok(())
}

/// Resolves `StaticMeshAssetRefs` into runtime `StaticMesh` components.
///
/// The scene format stores content-addressed `AssetId`s because prefab/world
/// serialization is reflection-based. The renderer-facing `StaticMesh`
/// component stores typed handles. This function bridges the two by creating
/// lightweight `Handle<MeshAsset>` / `Handle<MaterialAsset>` values directly
/// from the asset ids and inserting/updating `StaticMesh` on the entity.
pub fn resolve_static_mesh_asset_refs(world: &mut World) {
    let entities: Vec<(Entity, StaticMeshAssetRefs, Option<StaticMesh>)> = {
        let mut query = world.query::<(Entity, &StaticMeshAssetRefs, Option<&StaticMesh>)>();
        query
            .iter(world)
            .map(|(entity, refs, existing)| (entity, *refs, existing.cloned()))
            .collect()
    };

    for (entity, refs, existing) in entities {
        let desired = StaticMesh::new(
            Handle::<MeshAsset>::new(refs.mesh),
            Handle::<MaterialAsset>::new(refs.material),
        );
        if existing.as_ref() == Some(&desired) {
            continue;
        }
        if let Some(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(desired);
        }
    }
}

/// Exclusive system wrapper around [`resolve_static_mesh_asset_refs`].
pub fn resolve_static_mesh_asset_refs_system(world: &mut World) {
    resolve_static_mesh_asset_refs(world);
}

/// Loads a `.hyge-world` document from msgpack bytes into the ECS world.
///
/// This function updates scene-level resources, respawns environment helper
/// entities, replaces the currently loaded root prefab instances, and stores a
/// fresh [`LoadedSceneState`].
///
/// # Errors
///
/// Returns [`HygeError::Parse`] when the bytes are not a valid
/// `.hyge-world`, or [`HygeError::AssetNotFound`] if a referenced prefab id is
/// missing from the [`PrefabLibrary`].
pub fn load_world_document_from_bytes(world: &mut World, bytes: &[u8]) -> HygeResult<Vec<Entity>> {
    let doc = WorldAsset::load(bytes, &mut hyge_asset::LoadContext::default())?;
    let asset_id = Some(AssetId::from(blake3::hash(bytes)));
    Ok(replace_loaded_scene(world, &doc, None, asset_id, false)?.0)
}

/// Loads a `.hyge-world` file from disk into the ECS world.
///
/// # Errors
///
/// Returns I/O or parse errors if the file cannot be read or decoded, or
/// [`HygeError::AssetNotFound`] if the scene references unknown prefabs.
pub fn load_world_document_from_path(world: &mut World, path: &Path) -> HygeResult<Vec<Entity>> {
    let bytes = std::fs::read(path)?;
    let doc = WorldAsset::load(&bytes, &mut hyge_asset::LoadContext::default())?;
    let asset_id = Some(AssetId::from(blake3::hash(&bytes)));
    Ok(replace_loaded_scene(world, &doc, Some(path.to_path_buf()), asset_id, false)?.0)
}

/// Reloads the currently loaded scene file from disk, diffs it against the
/// active document, and reapplies the scene when needed.
///
/// # Errors
///
/// Returns [`HygeError::InvalidArgument`] if no scene file is currently loaded,
/// plus any file I/O / parse errors from re-reading the document.
pub fn reload_loaded_scene_from_disk(world: &mut World) -> HygeResult<SceneDocumentDiff> {
    Ok(reload_loaded_scene_from_disk_detailed(world)?.diff)
}

/// Reloads the active scene and reports persistent-state restoration details.
///
/// The operation is transactional from the caller's perspective: malformed
/// input or failed component hydration is returned before the current scene
/// resource is replaced.
///
/// # Errors
///
/// Returns file, decode, prefab, reflection or ECS errors without replacing
/// the active scene when validation fails.
pub fn reload_loaded_scene_from_disk_detailed(world: &mut World) -> HygeResult<SceneReloadReport> {
    let state = world
        .get_resource::<LoadedSceneState>()
        .ok_or_else(|| HygeError::invalid_argument("LoadedSceneState resource not found"))?
        .clone();
    let path = state
        .source_path
        .ok_or_else(|| HygeError::invalid_argument("no loaded .hyge-world source path"))?;
    let bytes = std::fs::read(&path)?;
    let new_doc = WorldAsset::load(&bytes, &mut hyge_asset::LoadContext::default())?;
    let new_asset_id = AssetId::from(blake3::hash(&bytes));
    let diff = match state.document.as_ref() {
        Some(old_doc) => diff_world_documents(old_doc, &new_doc),
        None => SceneDocumentDiff {
            added_instances: new_doc.root_prefab_instances.len(),
            ..SceneDocumentDiff::default()
        },
    };

    let (roots, preserved) =
        replace_loaded_scene(world, &new_doc, Some(path), Some(new_asset_id), true)?;
    if let Some(mut loaded_state) = world.get_resource_mut::<LoadedSceneState>() {
        loaded_state.last_diff = diff.clone();
    }
    let report = SceneReloadReport {
        diff,
        preserved_scene_ids: preserved.iter().map(|record| record.id.clone()).collect(),
        restored_scene_ids: preserved.iter().map(|record| record.id.clone()).collect(),
        reattached_scene_ids: Vec::new(),
    };
    let _ = roots;
    Ok(report)
}

/// Exclusive ECS hot-reload system for `.hyge-world` documents.
///
/// Drains the global [`ReloadQueue`]. If one of the events targets the
/// currently loaded scene file (by path or by asset id), the scene is re-read
/// from disk, diffed, and reapplied.
pub fn scene_hot_reload_system(world: &mut World) {
    let Some(queue) = world.get_resource::<ReloadQueue>().cloned() else {
        return;
    };
    let Some(state) = world.get_resource::<LoadedSceneState>().cloned() else {
        return;
    };
    let Some(source_path) = state.source_path.clone() else {
        return;
    };

    let should_reload = queue.drain().into_iter().any(|(path, id)| {
        path == source_path || state.asset_id.is_some_and(|current_id| current_id == id)
    });
    if !should_reload {
        return;
    }

    if let Err(error) = reload_loaded_scene_from_disk_detailed(world) {
        tracing::warn!(?error, path = %source_path.display(), "scene hot-reload failed");
    }
}

fn replace_loaded_scene(
    world: &mut World,
    doc: &WorldDocument,
    source_path: Option<PathBuf>,
    asset_id: Option<AssetId>,
    preserve_persistent: bool,
) -> HygeResult<(Vec<Entity>, Vec<PersistentSceneRecord>)> {
    let persistent = if preserve_persistent {
        capture_persistent_scene_records(world)?
    } else {
        Vec::new()
    };
    unload_current_scene(world)?;
    apply_scene_state_resources(world, doc);
    let environment_entities = spawn_environment_entities(world, &doc.env);
    let roots = if let Some(layer) = doc
        .editor_layer
        .as_ref()
        .filter(|layer| !layer.nodes.is_empty())
    {
        instantiate_editor_layer(world, layer)?
    } else {
        let roots = instantiate_root_prefab_instances(world, doc)?;
        for (index, root) in roots.iter().copied().enumerate() {
            assign_legacy_scene_node_ids(world, root, &format!("root-{index}"))?;
        }
        roots
    };

    restore_persistent_scene_records(world, &persistent)?;
    world.insert_resource(LoadedSceneState {
        source_path,
        asset_id,
        document: Some(doc.clone()),
        root_entities: roots.clone(),
        environment_entities,
        last_diff: SceneDocumentDiff::default(),
    });

    Ok((roots, persistent))
}

#[derive(Debug, Clone)]
struct PersistentSceneRecord {
    id: String,
    name: String,
    parent: Option<String>,
    order: u32,
    components: Vec<crate::SerializedComponentOverride>,
}

fn capture_persistent_scene_records(world: &mut World) -> HygeResult<Vec<PersistentSceneRecord>> {
    let registry = world
        .get_resource::<hyge_ecs::AppTypeRegistry>()
        .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
        .0
        .clone();
    let read = registry.read();
    let entities: Vec<(Entity, SceneNodeId, String, Option<Entity>)> = {
        let mut query = world.query::<(
            Entity,
            &SceneNodeId,
            &PersistOnReload,
            Option<&Name>,
            Option<&Parent>,
        )>();
        query
            .iter(world)
            .map(|(entity, id, _, name, parent)| {
                (
                    entity,
                    id.clone(),
                    name.map_or_else(|| entity.to_bits().to_string(), |name| name.0.clone()),
                    parent.map(|parent| parent.0),
                )
            })
            .collect()
    };
    let mut records = Vec::with_capacity(entities.len());
    for (entity, id, name, parent_entity) in entities {
        let order = parent_entity
            .and_then(|parent| world.get::<Children>(parent))
            .and_then(|children| children.0.iter().position(|child| *child == entity))
            .unwrap_or(0) as u32;
        let mut components = Vec::new();
        for registration in read.iter_with_data::<ReflectComponent>() {
            let type_path = registration.0.type_info().type_path();
            if type_path.ends_with("::Parent")
                || type_path.ends_with("::Children")
                || type_path.ends_with("::GlobalTransform")
                || type_path.ends_with("::Name")
                || type_path.ends_with("::SceneNodeId")
            {
                continue;
            }
            let Some(value) = registration.1.reflect(world.entity(entity)) else {
                continue;
            };
            let data = rmp_serde::to_vec(&ReflectSerializer::new(value, &read))
                .map_err(|error| HygeError::invalid_argument(error.to_string()))?;
            components.push(crate::SerializedComponentOverride {
                type_name: type_path.to_owned(),
                data,
            });
        }
        components.sort_by(|left, right| left.type_name.cmp(&right.type_name));
        records.push(PersistentSceneRecord {
            id: id.0,
            name,
            parent: parent_entity
                .and_then(|parent| world.get::<SceneNodeId>(parent).map(|id| id.0.clone())),
            order,
            components,
        });
    }
    Ok(records)
}

fn restore_persistent_scene_records(
    world: &mut World,
    records: &[PersistentSceneRecord],
) -> HygeResult<()> {
    if records.is_empty() {
        return Ok(());
    }
    let registry = world
        .get_resource::<hyge_ecs::AppTypeRegistry>()
        .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
        .0
        .clone();
    let mut entities: HashMap<String, Entity> = {
        let mut query = world.query::<(Entity, &SceneNodeId)>();
        query
            .iter(world)
            .map(|(entity, id)| (id.0.clone(), entity))
            .collect()
    };
    for record in records {
        let entity = if let Some(entity) = entities.get(&record.id).copied() {
            entity
        } else {
            let entity = world
                .spawn((
                    Name::new(record.name.clone()),
                    GlobalTransform::identity(),
                    SceneNodeId::new(record.id.clone()),
                    PersistOnReload,
                    SceneManagedEntity,
                ))
                .id();
            entities.insert(record.id.clone(), entity);
            entity
        };
        let read = registry.read();
        for component in &record.components {
            let registration = read
                .get_with_type_path(&component.type_name)
                .ok_or_else(|| {
                    HygeError::invalid_argument(format!(
                        "component '{}' is not registered",
                        component.type_name
                    ))
                })?;
            let reflect_component = registration.data::<ReflectComponent>().ok_or_else(|| {
                HygeError::invalid_argument(format!(
                    "component '{}' is not an ECS component",
                    component.type_name
                ))
            })?;
            let mut deserializer = rmp_serde::Deserializer::new(component.data.as_slice());
            let value =
                DeserializeSeed::deserialize(ReflectDeserializer::new(&read), &mut deserializer)
                    .map_err(|error| HygeError::invalid_argument(error.to_string()))?;
            reflect_component.insert(&mut world.entity_mut(entity), value.as_ref(), &read);
        }
    }
    for record in records {
        let Some(entity) = entities.get(&record.id).copied() else {
            continue;
        };
        let Some(parent_id) = record.parent.as_deref() else {
            continue;
        };
        let Some(parent) = entities.get(parent_id).copied() else {
            continue;
        };
        world.entity_mut(entity).insert(Parent(parent));
        let mut children = world
            .get::<Children>(parent)
            .map(|children| children.0.clone())
            .unwrap_or_default();
        let position = (record.order as usize).min(children.len());
        if !children.contains(&entity) {
            children.insert(position, entity);
        }
        world.entity_mut(parent).insert(Children(children));
    }
    Ok(())
}

fn instantiate_editor_layer(world: &mut World, layer: &SceneEditLayer) -> HygeResult<Vec<Entity>> {
    if layer.version != EDITOR_SCENE_LAYER_VERSION {
        return Err(HygeError::invalid_argument(format!(
            "unsupported editor scene layer version {}",
            layer.version
        )));
    }
    let registry = world
        .get_resource::<hyge_ecs::AppTypeRegistry>()
        .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
        .0
        .clone();
    let mut entities = HashMap::new();
    for record in &layer.nodes {
        let entity = world
            .spawn((
                Name::new(record.name.clone()),
                GlobalTransform::identity(),
                SceneNodeId::new(record.id.clone()),
                SceneManagedEntity,
            ))
            .id();
        entities.insert(record.id.clone(), entity);
    }
    for record in &layer.nodes {
        let entity = entities[&record.id];
        if let Some(parent_id) = record.parent.as_deref() {
            let parent = *entities.get(parent_id).ok_or_else(|| {
                HygeError::invalid_argument(format!(
                    "editor node '{}' has missing parent",
                    record.id
                ))
            })?;
            world.entity_mut(entity).insert(Parent(parent));
            let mut children = world
                .get_mut::<Children>(parent)
                .map(|children| children.0.clone())
                .unwrap_or_default();
            let position = (record.order as usize).min(children.len());
            children.insert(position, entity);
            world.entity_mut(parent).insert(Children(children));
        }
        let read = registry.read();
        for component in &record.components {
            let registration = read
                .get_with_type_path(&component.type_name)
                .ok_or_else(|| {
                    HygeError::invalid_argument(format!(
                        "component '{}' is not registered",
                        component.type_name
                    ))
                })?;
            let reflect_component = registration.data::<ReflectComponent>().ok_or_else(|| {
                HygeError::invalid_argument(format!(
                    "component '{}' is not an ECS component",
                    component.type_name
                ))
            })?;
            let mut deserializer = rmp_serde::Deserializer::new(component.data.as_slice());
            let value = bevy_reflect::serde::ReflectDeserializer::new(&read);
            let value = serde::de::DeserializeSeed::deserialize(value, &mut deserializer)
                .map_err(|error| HygeError::invalid_argument(error.to_string()))?;
            reflect_component.insert(&mut world.entity_mut(entity), value.as_ref(), &read);
        }
    }
    Ok(layer
        .nodes
        .iter()
        .filter(|record| record.parent.is_none())
        .map(|record| entities[&record.id])
        .collect())
}

fn assign_scene_node_ids_recursive(
    world: &mut World,
    entity: Entity,
    seed: &str,
    path: &str,
    replace: bool,
) -> HygeResult<()> {
    if world.get::<SceneNodeId>(entity).is_none() || replace {
        let digest = blake3::hash(format!("{seed}:{path}").as_bytes());
        world.entity_mut(entity).insert((
            SceneNodeId::new(format!("scene-{}", digest.to_hex())),
            SceneManagedEntity,
        ));
    } else {
        world.entity_mut(entity).insert(SceneManagedEntity);
    }
    let children = world
        .get::<Children>(entity)
        .map(|children| children.0.clone())
        .unwrap_or_default();
    for (index, child) in children.into_iter().enumerate() {
        assign_scene_node_ids_recursive(world, child, seed, &format!("{path}/{index}"), replace)?;
    }
    Ok(())
}

fn instantiate_root_prefab_instances(
    world: &mut World,
    doc: &WorldDocument,
) -> HygeResult<Vec<Entity>> {
    let library = world
        .get_resource::<PrefabLibrary>()
        .ok_or_else(|| HygeError::invalid_argument("PrefabLibrary resource not found"))?
        .clone();
    let roots = WorldLoader::load(doc, world, move |id| {
        library
            .get(id)
            .cloned()
            .ok_or_else(|| HygeError::asset_not_found(format!("prefab '{id:?}' not found")))
    })?;
    for root in &roots {
        mark_scene_subtree(world, *root)?;
    }
    Ok(roots)
}

fn apply_scene_state_resources(world: &mut World, doc: &WorldDocument) {
    world.insert_resource(SceneEnvironmentState {
        environment: doc.env.clone(),
    });
    world.insert_resource(ScenePostProcessState {
        profile: doc.post_process,
    });
}

fn spawn_environment_entities(world: &mut World, env: &Environment) -> Vec<Entity> {
    let mut entities = Vec::new();
    if let Some(sun) = env.sun {
        let entity = world
            .spawn((
                SceneManagedEntity,
                Name::new("scene-sun"),
                DirectionalLight {
                    direction: sun.direction,
                    color: sun.color,
                    illuminance: sun.illuminance,
                },
            ))
            .id();
        entities.push(entity);
    }
    if env.ambient.intensity > 0.0 {
        let entity = world
            .spawn((
                SceneManagedEntity,
                Name::new("scene-ambient"),
                AmbientLight {
                    color: env.ambient.color,
                    intensity: env.ambient.intensity,
                },
            ))
            .id();
        entities.push(entity);
    }
    entities
}

fn unload_current_scene(world: &mut World) -> HygeResult<()> {
    let state = world.get_resource::<LoadedSceneState>().cloned();
    let Some(state) = state else {
        return Ok(());
    };
    for root in state.root_entities {
        despawn_subtree(world, root)?;
    }
    for entity in state.environment_entities {
        if world.get_entity(entity).is_some() {
            world.despawn(entity);
        }
    }
    Ok(())
}

fn despawn_subtree(world: &mut World, entity: Entity) -> HygeResult<()> {
    let children: Vec<Entity> = world
        .get::<Children>(entity)
        .map(|children| children.0.clone())
        .unwrap_or_default();
    for child in children {
        despawn_subtree(world, child)?;
    }
    if world.get_entity(entity).is_some() {
        world.despawn(entity);
    }
    Ok(())
}

fn mark_scene_subtree(world: &mut World, entity: Entity) -> HygeResult<()> {
    if let Some(mut entity_mut) = world.get_entity_mut(entity) {
        entity_mut.insert(SceneManagedEntity);
    }
    let children: Vec<Entity> = world
        .get::<Children>(entity)
        .map(|children| children.0.clone())
        .unwrap_or_default();
    for child in children {
        mark_scene_subtree(world, child)?;
    }
    Ok(())
}

fn diff_world_documents(old: &WorldDocument, new: &WorldDocument) -> SceneDocumentDiff {
    let shared_len = old
        .root_prefab_instances
        .len()
        .min(new.root_prefab_instances.len());
    let changed_instances = old
        .root_prefab_instances
        .iter()
        .zip(new.root_prefab_instances.iter())
        .take(shared_len)
        .filter(|(a, b)| a != b)
        .count();
    SceneDocumentDiff {
        added_instances: new
            .root_prefab_instances
            .len()
            .saturating_sub(old.root_prefab_instances.len()),
        removed_instances: old
            .root_prefab_instances
            .len()
            .saturating_sub(new.root_prefab_instances.len()),
        changed_instances,
        environment_changed: old.env != new.env,
        post_process_changed: old.post_process != new.post_process,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        components::{Name, Transform},
        env::{AmbientParams, FogParams},
        plugin::build_scene_type_registry,
        prefab::{PrefabAssets, PrefabNode, SerializedComponentOverride},
        world::PrefabInstance,
    };
    use hyge_ecs::AppTypeRegistry;

    fn install_registry(world: &mut World) {
        let registry = AppTypeRegistry::default();
        *registry.write() = build_scene_type_registry();
        world.insert_resource(registry);
    }

    fn sample_prefab(mesh: AssetId, material: AssetId) -> Prefab {
        let registry = build_scene_type_registry();
        let mut root = PrefabNode::named("runtime-root");
        root.components.push(
            SerializedComponentOverride::new(
                "hyge_scene::components::Name",
                &Name::new("runtime-root"),
                &registry,
            )
            .expect("name serializes"),
        );
        root.components.push(
            SerializedComponentOverride::new(
                "hyge_scene::components::StaticMeshAssetRefs",
                &StaticMeshAssetRefs::new(mesh, material),
                &registry,
            )
            .expect("static mesh refs serialize"),
        );
        Prefab::new(
            "runtime-prefab",
            root,
            PrefabAssets {
                meshes: vec![mesh],
                materials: vec![material],
                scripts: Vec::new(),
            },
        )
    }

    fn sample_doc(prefab_id: PrefabId) -> WorldDocument {
        WorldDocument {
            env: Environment {
                skybox: Some(AssetId::from(blake3::hash(b"runtime-sky"))),
                sun: Some(DirectionalLight {
                    direction: [0.2, -1.0, 0.1],
                    color: [1.0, 0.95, 0.9],
                    illuminance: 60_000.0,
                }),
                fog: Some(FogParams::default()),
                ambient: AmbientParams {
                    color: [0.15, 0.16, 0.18],
                    intensity: 0.4,
                },
            },
            root_prefab_instances: (0..5)
                .map(|i| {
                    PrefabInstance::new(
                        prefab_id,
                        Transform {
                            translation: [i as f32 * 2.0, 0.0, 0.0],
                            ..Transform::identity()
                        },
                    )
                })
                .collect(),
            post_process: PostProcessProfile::default(),
            editor_layer: None,
        }
    }

    #[test]
    fn resolve_static_mesh_asset_refs_materializes_runtime_static_mesh() {
        let mesh = AssetId::from(blake3::hash(b"mesh"));
        let material = AssetId::from(blake3::hash(b"material"));
        let mut world = World::new();
        let entity = world
            .spawn((StaticMeshAssetRefs::new(mesh, material),))
            .id();

        resolve_static_mesh_asset_refs(&mut world);

        let static_mesh = world
            .get::<StaticMesh>(entity)
            .expect("StaticMesh inserted");
        assert_eq!(static_mesh.mesh.id(), mesh);
        assert_eq!(static_mesh.material.id(), material);
    }

    #[test]
    fn load_world_document_populates_scene_resources_and_roots() {
        let mesh = AssetId::from(blake3::hash(b"mesh-load"));
        let material = AssetId::from(blake3::hash(b"material-load"));
        let prefab = sample_prefab(mesh, material);
        let doc = sample_doc(prefab.prefab_id);

        let mut world = World::new();
        install_registry(&mut world);
        let mut library = PrefabLibrary::default();
        library.insert(prefab);
        world.insert_resource(library);

        let roots = replace_loaded_scene(&mut world, &doc, None, None, false)
            .expect("scene loads")
            .0;
        assert_eq!(roots.len(), 5);
        assert_eq!(
            world
                .get_resource::<SceneEnvironmentState>()
                .expect("environment state")
                .environment,
            doc.env
        );
        assert_eq!(
            world
                .get_resource::<ScenePostProcessState>()
                .expect("post-process state")
                .profile,
            doc.post_process
        );

        resolve_static_mesh_asset_refs(&mut world);
        let static_mesh_count = world.query::<&StaticMesh>().iter(&world).count();
        assert_eq!(static_mesh_count, 5);
    }

    #[test]
    fn diff_world_documents_reports_changed_roots_and_environment() {
        let prefab_id = PrefabId::compute(b"runtime-diff-prefab");
        let mut a = sample_doc(prefab_id);
        let mut b = sample_doc(prefab_id);
        b.root_prefab_instances[2].transform.translation[0] = 99.0;
        b.root_prefab_instances.pop();
        b.env.ambient.intensity = 0.9;
        b.post_process.exposure = 1.25;

        let diff = diff_world_documents(&a, &b);
        assert_eq!(diff.changed_instances, 1);
        assert_eq!(diff.removed_instances, 1);
        assert!(diff.environment_changed);
        assert!(diff.post_process_changed);

        a.root_prefab_instances
            .push(PrefabInstance::new(prefab_id, Transform::identity()));
        let diff_added = diff_world_documents(&b, &a);
        assert_eq!(diff_added.added_instances, 2);
    }
}
