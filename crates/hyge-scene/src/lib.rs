//! Hyge scene: glTF loader, prefab system, instancing extraction, and the
//! canonical ECS component catalog.
//!
//! Owns the `Prefab` / `PrefabNode` / `ComponentOverride` types and the
//! `.hyge-prefab` / `.hyge-world` serialization formats. Performs meshlet
//! bake via `meshopt` during import (delegated to the importer in `hyge-tools`).
//!
//! See `docs/architecture.md` §6.6 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-035, R-060..R-064.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod components;
pub mod env;
pub mod extract;
pub mod plugin;
pub mod prefab;
pub mod prefab_asset;
pub mod prefab_id;
pub mod prelude;
pub mod runtime;
pub mod transform;
pub mod world;
pub mod world_asset;

pub use components::{
    AmbientLight, AudioBus, AudioListener, AudioRolloff, AudioSource, Camera, CharacterController,
    Children, Collider, ColliderShape, DirectionalLight, EditorCamera, FogVolume, GlobalTransform,
    Joint, LightComponent, MaterialHandle, MeshHandle, Name, Parent, PersistOnReload, PointLight,
    PostProcessVolume, RigidBody, RigidBodyKind, SceneNodeId, ScriptRef, SpotLight, StaticMesh,
    StaticMeshAssetRefs, Transform, WorldTransform,
};
pub use env::{AmbientParams, Environment, FogParams, PostProcessProfile};
pub use plugin::{build_scene_type_registry, ScenePlugin};
pub use prefab::{
    ComponentOverride, Prefab, PrefabAssets, PrefabNode, SerializedComponentOverride,
};
pub use prefab_asset::PrefabAsset;
pub use prefab_id::PrefabId;
pub use runtime::{
    assign_legacy_scene_node_ids, assign_new_scene_node_ids, capture_editor_scene_layer,
    load_world_document_from_bytes, load_world_document_from_path, reload_loaded_scene_from_disk,
    resolve_static_mesh_asset_refs, resolve_static_mesh_asset_refs_system, scene_hot_reload_system,
    sync_editor_layer_from_world, EnvironmentLibrary, LoadedSceneState, PrefabLibrary,
    SceneDocumentDiff, SceneEnvironmentState, SceneManagedEntity, ScenePostProcessState,
};
pub use world::{
    PrefabInstance, SceneEditLayer, SceneNodeRecord, WorldDocument, WorldLoader,
    EDITOR_SCENE_LAYER_VERSION,
};
pub use world_asset::WorldAsset;
