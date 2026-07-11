//! The [`ScenePlugin`] registers the canonical scene component catalog and
//! the transform propagation / render-extract systems.

use bevy_app::App;
use bevy_reflect::TypeRegistry;
use hyge_ecs::prelude::*;
use hyge_ecs::AppTypeRegistry;

use crate::components::{
    AmbientLight, AudioBus, AudioListener, AudioRolloff, AudioSource, Camera, CharacterController,
    Children, Collider, ColliderShape, DirectionalLight, EditorCamera, FogVolume, GlobalTransform,
    Joint, LightComponent, MaterialHandle, MeshHandle, Name, Parent, PersistOnReload, PointLight,
    PostProcessVolume, RigidBody, RigidBodyKind, SceneNodeId, ScriptRef, SpotLight,
    StaticMeshAssetRefs, Transform, WorldTransform,
};
use crate::env::{AmbientParams, FogParams, PostProcessProfile};
use crate::extract::{add_render_extract_system, FrameSnapshot};
use crate::runtime::{
    resolve_static_mesh_asset_refs_system, scene_hot_reload_system, EnvironmentLibrary,
    LoadedSceneState, PrefabLibrary, SceneDocumentDiff, SceneEnvironmentState,
    ScenePostProcessState,
};
use crate::transform::{hierarchy_cleanup_system, transform_propagate_system};

/// Hyge scene plugin.
///
/// Registers:
/// - the [`TypeRegistry`] resource populated with every canonical scene
///   component;
/// - the [`FrameSnapshot`] resource;
/// - the transform propagation system in [`TransformSet::Propagate`];
/// - a lightweight hierarchy cleanup system;
/// - the render-extract system in [`Label::RenderExtract`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ScenePlugin;

impl HygePlugin for ScenePlugin {
    fn name(&self) -> &'static str {
        "hyge-scene"
    }

    fn build(&self, app: &mut App) {
        tracing::debug!("building hyge-scene plugin");

        // Prefab instantiation and the editor inspector need a populated
        // reflect registry for all canonical components.
        let type_registry = AppTypeRegistry::default();
        *type_registry.write() = build_scene_type_registry();
        app.insert_resource(type_registry);

        // Resource consumed by the renderer and produced by extract.
        app.init_resource::<FrameSnapshot>();
        app.init_resource::<PrefabLibrary>();
        app.init_resource::<EnvironmentLibrary>();
        app.init_resource::<SceneEnvironmentState>();
        app.init_resource::<ScenePostProcessState>();
        app.init_resource::<SceneDocumentDiff>();
        app.init_resource::<LoadedSceneState>();

        // Transform propagation runs during the variable update so gameplay
        // systems can reparent entities before the render extract.
        app.add_systems(
            Label::Update,
            transform_propagate_system.in_set(TransformSet::Propagate),
        );

        // Light hierarchy maintenance; kept in the same set for ordering.
        app.add_systems(
            Label::Update,
            hierarchy_cleanup_system.in_set(TransformSet::Flush),
        );

        // Resolve prefab/world-authored `StaticMeshAssetRefs` into runtime
        // `StaticMesh` handles before the render-extract schedule runs.
        app.add_systems(Label::Update, resolve_static_mesh_asset_refs_system);

        // `.hyge-world` hot-reload is driven by the global ReloadQueue.
        app.add_systems(Label::Update, scene_hot_reload_system);

        // Render extract produces the per-frame snapshot.
        let mut render_extract_schedule = Schedule::new(Label::RenderExtract);
        add_render_extract_system(&mut render_extract_schedule);
        app.add_schedule(render_extract_schedule);
    }
}

/// Builds a [`TypeRegistry`] containing every canonical scene component and
/// the enums they reference.
///
/// This registry is used by the prefab system to serialize and deserialize
/// component overrides and by the editor inspector to iterate component
/// fields reflectively.
#[must_use]
pub fn build_scene_type_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();

    // Core transform / hierarchy components.
    registry.register::<Transform>();
    registry.register::<GlobalTransform>();
    registry.register::<Parent>();
    registry.register::<Children>();
    registry.register::<Name>();
    registry.register::<PersistOnReload>();
    registry.register::<SceneNodeId>();
    registry.register::<StaticMeshAssetRefs>();

    // Legacy render-facing components.
    registry.register::<MeshHandle>();
    registry.register::<MaterialHandle>();
    registry.register::<WorldTransform>();
    registry.register::<LightComponent>();

    // Lights.
    registry.register::<PointLight>();
    registry.register::<SpotLight>();
    registry.register::<DirectionalLight>();
    registry.register::<AmbientLight>();

    // Camera.
    registry.register::<Camera>();
    registry.register::<EditorCamera>();

    // Audio.
    registry.register::<AudioSource>();
    registry.register::<AudioListener>();
    registry.register::<AudioBus>();
    registry.register::<AudioRolloff>();

    // Scripting.
    registry.register::<ScriptRef>();

    // Physics stubs and their supporting enums.
    registry.register::<RigidBody>();
    registry.register::<RigidBodyKind>();
    registry.register::<Collider>();
    registry.register::<ColliderShape>();
    registry.register::<CharacterController>();
    registry.register::<Joint>();

    // Volumes.
    registry.register::<PostProcessVolume>();
    registry.register::<FogVolume>();

    // R-063 — Scene-level environment descriptors (used by .hyge-world
    // overrides and the scene inspector).
    registry.register::<FogParams>();
    registry.register::<AmbientParams>();
    registry.register::<PostProcessProfile>();

    registry
}
