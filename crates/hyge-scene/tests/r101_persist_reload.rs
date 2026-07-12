//! R-101 evidence for persistent scene state across a real disk reload.

use std::path::PathBuf;

use bevy_ecs::prelude::World;
use hyge_ecs::AppTypeRegistry;
use hyge_scene::prelude::{
    build_scene_type_registry, load_world_document_from_path,
    reload_loaded_scene_from_disk_detailed, PersistOnReload, PrefabLibrary,
    SerializedComponentOverride, Transform, WorldDocument,
};
use hyge_scene::{SceneEditLayer, SceneNodeId, SceneNodeRecord, EDITOR_SCENE_LAYER_VERSION};

fn temp_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "hyge-r101-persist-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[test]
fn reload_restores_persistent_component_by_scene_identity() {
    let root = temp_path();
    std::fs::create_dir_all(&root).expect("fixture directory");
    let path = root.join("scene.hyge-world");
    let registry = build_scene_type_registry();
    let id = "persistent-player";
    let initial_transform = Transform::identity();
    let disk_transform = Transform {
        translation: [2.0, 0.0, 0.0],
        ..Transform::identity()
    };
    let layer = |transform: &Transform| {
        let serialization_registry = build_scene_type_registry();
        SceneEditLayer {
            version: EDITOR_SCENE_LAYER_VERSION,
            nodes: vec![SceneNodeRecord {
                id: id.to_owned(),
                parent: None,
                order: 0,
                name: "Player".to_owned(),
                components: vec![
                    SerializedComponentOverride::new(
                        "hyge_scene::components::Transform",
                        transform,
                        &serialization_registry,
                    )
                    .expect("transform override"),
                    SerializedComponentOverride::new(
                        "hyge_scene::components::PersistOnReload",
                        &PersistOnReload,
                        &serialization_registry,
                    )
                    .expect("persist override"),
                ],
            }],
            tombstones: Vec::new(),
        }
    };
    let document = WorldDocument {
        editor_layer: Some(layer(&initial_transform)),
        ..WorldDocument::empty()
    };
    std::fs::write(&path, document.to_bytes().expect("serialize initial")).expect("write initial");

    let mut world = World::new();
    let type_registry = AppTypeRegistry::default();
    *type_registry.write() = registry;
    world.insert_resource(type_registry);
    world.insert_resource(PrefabLibrary::default());
    load_world_document_from_path(&mut world, &path).expect("load initial");
    let entity = world
        .query::<(bevy_ecs::prelude::Entity, &SceneNodeId, &PersistOnReload)>()
        .iter(&world)
        .find(|(_, scene_id, _)| scene_id.as_str() == id)
        .map(|(entity, _, _)| entity)
        .expect("persistent entity");
    world.entity_mut(entity).insert((
        PersistOnReload,
        Transform {
            translation: [99.0, 0.0, 0.0],
            ..Transform::identity()
        },
    ));
    assert_eq!(world.query::<&PersistOnReload>().iter(&world).count(), 1);

    let changed = WorldDocument {
        editor_layer: Some(layer(&disk_transform)),
        ..WorldDocument::empty()
    };
    std::fs::write(&path, changed.to_bytes().expect("serialize changed")).expect("write changed");
    let report = reload_loaded_scene_from_disk_detailed(&mut world).expect("reload");
    assert_eq!(report.restored_scene_ids, vec![id.to_owned()]);
    let (_, transform) = world
        .query::<(&SceneNodeId, &Transform)>()
        .iter(&world)
        .find(|(scene_id, _)| scene_id.as_str() == id)
        .expect("restored transform");
    assert_eq!(transform.translation, [99.0, 0.0, 0.0]);
    assert_eq!(world.query::<&SceneNodeId>().iter(&world).count(), 1);
    let _ = std::fs::remove_dir_all(root);
}
