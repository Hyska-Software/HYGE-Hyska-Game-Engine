//! R-065 scene runtime hot-reload integration.

use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use bevy_ecs::prelude::World;
use hyge_asset::{AssetId, AssetResolver, FileWatcher, ReloadQueue};
use hyge_ecs::AppTypeRegistry;
use hyge_scene::prelude::{
    build_scene_type_registry, load_world_document_from_path, resolve_static_mesh_asset_refs,
    scene_hot_reload_system, LoadedSceneState, PostProcessProfile, Prefab, PrefabAssets,
    PrefabInstance, PrefabLibrary, PrefabNode, SceneEnvironmentState, SceneManagedEntity,
    ScenePostProcessState, SerializedComponentOverride, StaticMesh, StaticMeshAssetRefs, Transform,
    WorldDocument,
};

fn temp_scene_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "hyge-scene-r065-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}

fn sample_prefab(mesh: AssetId, material: AssetId) -> Prefab {
    let registry = build_scene_type_registry();
    let mut root = PrefabNode::named("helmet-root");
    root.components.push(
        SerializedComponentOverride::new(
            "hyge_scene::components::StaticMeshAssetRefs",
            &StaticMeshAssetRefs::new(mesh, material),
            &registry,
        )
        .expect("static mesh refs serialize"),
    );
    Prefab::new(
        "helmet-prefab",
        root,
        PrefabAssets {
            meshes: vec![mesh],
            materials: vec![material],
            scripts: Vec::new(),
        },
    )
}

fn sample_document(prefab_id: hyge_scene::PrefabId, exposure: f32) -> WorldDocument {
    WorldDocument {
        env: hyge_scene::Environment {
            skybox: Some(AssetId::from(blake3::hash(b"r065-sky"))),
            sun: Some(hyge_scene::DirectionalLight {
                direction: [0.3, -1.0, 0.2],
                color: [1.0, 0.95, 0.85],
                illuminance: 70_000.0,
            }),
            fog: None,
            ambient: hyge_scene::AmbientParams {
                color: [0.12, 0.13, 0.16],
                intensity: 0.35,
            },
        },
        root_prefab_instances: (0..5)
            .map(|i| {
                PrefabInstance::new(
                    prefab_id,
                    Transform {
                        translation: [i as f32 * 1.5, 0.0, 0.0],
                        ..Transform::identity()
                    },
                )
            })
            .collect(),
        post_process: PostProcessProfile {
            exposure,
            ..PostProcessProfile::default()
        },
        editor_layer: None,
    }
}

fn write_scene(path: &PathBuf, doc: &WorldDocument) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent dir creates");
    }
    let bytes = doc.to_bytes().expect("world bytes");
    std::fs::write(path, bytes).expect("scene file writes");
}

#[test]
fn world_hot_reload_diffs_and_reapplies() {
    let mesh = AssetId::from(blake3::hash(b"r065-mesh"));
    let material = AssetId::from(blake3::hash(b"r065-material"));
    let prefab = sample_prefab(mesh, material);
    let doc_a = sample_document(prefab.prefab_id, 1.0);
    let mut doc_b = sample_document(prefab.prefab_id, 1.25);
    doc_b.root_prefab_instances[2].transform.translation[0] = 42.0;

    let root = temp_scene_root();
    let watched_dir = root.join("assets").join("source");
    let scene_path = watched_dir.join("smoke.hyge-world");
    write_scene(&scene_path, &doc_a);

    let mut world = World::new();
    let type_registry = AppTypeRegistry::default();
    *type_registry.write() = build_scene_type_registry();
    world.insert_resource(type_registry);
    let mut library = PrefabLibrary::default();
    library.insert(prefab.clone());
    world.insert_resource(library);
    let queue = ReloadQueue::new();
    world.insert_resource(queue.clone());

    let initial_bytes = std::fs::read(&scene_path).expect("scene bytes");
    let initial_id = AssetId::from(blake3::hash(&initial_bytes));
    let resolver: AssetResolver = Arc::new(move |path| {
        if path.ends_with("smoke.hyge-world") {
            Some(initial_id)
        } else {
            None
        }
    });
    let _watcher = FileWatcher::watch_paths(vec![watched_dir.clone()], queue.clone(), resolver)
        .expect("file watcher starts");

    let roots = load_world_document_from_path(&mut world, &scene_path).expect("scene loads");
    assert_eq!(roots.len(), 5);
    resolve_static_mesh_asset_refs(&mut world);
    assert_eq!(world.query::<&StaticMesh>().iter(&world).count(), 5);
    assert_eq!(
        world
            .get_resource::<ScenePostProcessState>()
            .expect("post state")
            .profile
            .exposure,
        1.0
    );

    // Rewrite the scene with one changed instance and a changed post profile.
    thread::sleep(Duration::from_millis(50));
    write_scene(&scene_path, &doc_b);

    let deadline = Instant::now() + Duration::from_millis(500);
    let mut received = false;
    while Instant::now() < deadline {
        if !queue.is_empty() {
            received = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(received, "ReloadQueue must receive the .hyge-world change");

    scene_hot_reload_system(&mut world);
    resolve_static_mesh_asset_refs(&mut world);

    let state = world
        .get_resource::<LoadedSceneState>()
        .expect("loaded scene state updated");
    assert_eq!(state.root_entities.len(), 5);
    assert_eq!(state.last_diff.changed_instances, 1);
    assert!(state.last_diff.post_process_changed);
    assert_eq!(
        world
            .get_resource::<ScenePostProcessState>()
            .expect("post state updated")
            .profile
            .exposure,
        1.25
    );
    assert_eq!(world.query::<&StaticMesh>().iter(&world).count(), 5);
    assert!(
        world.query::<&SceneManagedEntity>().iter(&world).count() >= 5,
        "scene-managed entities should still be present after reload"
    );
    assert!(
        world.get_resource::<SceneEnvironmentState>().is_some(),
        "environment state remains present after reload"
    );
}
