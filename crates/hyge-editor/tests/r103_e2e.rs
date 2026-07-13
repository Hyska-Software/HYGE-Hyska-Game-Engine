//! R-103 real project workflow and retained fixture evidence.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use hyge_editor::{EditComponentsCommand, EditorCommand, EditorSessionRuntime, SceneReloadEvent};
use hyge_scene::prelude::{
    build_scene_type_registry, MaterialHandle, MeshHandle, PersistOnReload, Transform,
    WorldTransform,
};
use hyge_scene::{
    Environment, PostProcessProfile, Prefab, PrefabAssets, PrefabInstance, PrefabNode,
    SerializedComponentOverride, WorldDocument,
};
use serde_json::json;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("r103-editor-project")
}

fn generate_fixture(root: &Path) {
    fs::create_dir_all(root.join("assets")).expect("fixture directory");
    let registry = build_scene_type_registry();
    let mut node = PrefabNode::named("Persistent Cube");
    node.persist = true;
    node.components = vec![
        SerializedComponentOverride::new(
            "hyge_scene::components::Transform",
            &Transform::identity(),
            &registry,
        )
        .expect("transform override"),
        SerializedComponentOverride::new(
            "hyge_scene::components::PersistOnReload",
            &PersistOnReload,
            &registry,
        )
        .expect("persist override"),
        SerializedComponentOverride::new(
            "hyge_scene::components::MeshHandle",
            &MeshHandle(0),
            &registry,
        )
        .expect("mesh override"),
        SerializedComponentOverride::new(
            "hyge_scene::components::MaterialHandle",
            &MaterialHandle(0),
            &registry,
        )
        .expect("material override"),
        SerializedComponentOverride::new(
            "hyge_scene::components::WorldTransform",
            &WorldTransform::identity(),
            &registry,
        )
        .expect("world transform override"),
    ];
    let prefab = Prefab::new("r103-cube", node, PrefabAssets::default());
    fs::write(
        root.join("assets").join("persistent-cube.hyge-prefab"),
        prefab.to_bytes().expect("serialize prefab"),
    )
    .expect("write prefab");
    let document = |exposure| WorldDocument {
        env: Environment::empty(),
        root_prefab_instances: vec![PrefabInstance::new(prefab.prefab_id, Transform::identity())],
        post_process: PostProcessProfile {
            exposure,
            ..PostProcessProfile::default()
        },
        editor_layer: None,
    };
    fs::write(
        root.join("main.hyge-world"),
        document(1.0).to_bytes().expect("serialize main world"),
    )
    .expect("write main world");
    fs::write(
        root.join("external.hyge-world"),
        document(2.0).to_bytes().expect("serialize external world"),
    )
    .expect("write external world");
}

fn copy_fixture() -> PathBuf {
    let source = fixture_root();
    assert!(
        source.join("main.hyge-world").is_file(),
        "run ignored fixture generator"
    );
    let destination = std::env::temp_dir().join(format!(
        "hyge-r103-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(destination.join("assets")).expect("destination");
    for relative in [
        "main.hyge-world",
        "external.hyge-world",
        "assets/persistent-cube.hyge-prefab",
    ] {
        fs::copy(source.join(relative), destination.join(relative)).expect("copy fixture file");
    }
    destination
}

fn translation(snapshot: &hyge_editor::EditorSnapshot, scene_id: &str) -> serde_json::Value {
    let entity = snapshot
        .hierarchy
        .iter()
        .find(|node| node.scene_id.as_deref() == Some(scene_id))
        .expect("persistent hierarchy node")
        .entity;
    let components = &snapshot
        .entities
        .iter()
        .find(|item| item.entity == entity)
        .expect("persistent entity")
        .components;
    components
        .iter()
        .find(|component| component.type_path.ends_with("::Transform"))
        .and_then(|component| component.value.as_ref())
        .and_then(|value| value.get("hyge_scene::components::Transform"))
        .and_then(|value| value.get("translation"))
        .cloned()
        .unwrap_or_else(|| panic!("serialized Transform.translation in {components:#?}"))
}

#[test]
#[ignore = "fixture regeneration is explicit"]
fn regenerate_checked_in_fixture() {
    generate_fixture(&fixture_root());
}

#[test]
fn real_scene_edit_history_save_and_reload_preserve_identity() {
    let root = copy_fixture();
    let scene = root.join("main.hyge-world");
    let mut runtime = EditorSessionRuntime::new();
    runtime.open_project(&root).expect("open project");
    runtime.open_scene(&scene).expect("open scene");
    let before = runtime.editor_snapshot().expect("snapshot");
    let node = before
        .hierarchy
        .iter()
        .find(|node| node.name == "Persistent Cube")
        .expect("persistent cube");
    let scene_id = node.scene_id.clone().expect("stable scene id");
    let transform_path = before
        .component_catalog
        .iter()
        .find(|component| component.short_name == "Transform")
        .expect("transform descriptor")
        .type_path
        .clone();
    let (_, edited) = runtime
        .apply_command(
            before.revision,
            EditorCommand::EditComponents(EditComponentsCommand::new(
                vec![node.entity],
                transform_path,
                Some("translation".into()),
                json!([3.0, 1.0, 0.0]),
            )),
        )
        .expect("reflect edit");
    let before_global = before.entities[0]
        .components
        .iter()
        .find(|component| component.type_path.ends_with("::GlobalTransform"))
        .and_then(|component| component.value.clone());
    let edited_global = edited.entities[0]
        .components
        .iter()
        .find(|component| component.type_path.ends_with("::GlobalTransform"))
        .and_then(|component| component.value.clone());
    assert_ne!(
        before_global, edited_global,
        "Transform edit must propagate to render state"
    );
    let (_, undone) = runtime.undo_command(edited.revision).expect("undo");
    let (_, redone) = runtime.redo_command(undone.revision).expect("redo");
    runtime.save_scene().expect("save");
    assert!(redone.revision < runtime.editor_snapshot().expect("saved snapshot").revision);
    let reopen_root = copy_fixture();
    fs::copy(&scene, reopen_root.join("main.hyge-world")).expect("copy saved scene");
    let mut reopened = EditorSessionRuntime::new();
    reopened
        .open_project(&reopen_root)
        .expect("reopen saved project");
    reopened
        .open_scene(&reopen_root.join("main.hyge-world"))
        .expect("reopen saved scene");
    assert_eq!(
        translation(
            &reopened.editor_snapshot().expect("reopened snapshot"),
            &scene_id
        ),
        json!([3.0, 1.0, 0.0])
    );
    reopened.shutdown();
    let _ = fs::remove_dir_all(reopen_root);

    fs::copy(root.join("external.hyge-world"), &scene).expect("external edit");
    let event = (0..100).find_map(|_| {
        thread::sleep(Duration::from_millis(20));
        runtime.poll_scene_reload().expect("poll reload")
    });
    let Some(SceneReloadEvent::Reloaded(report)) = event else {
        panic!("scene reload was not observed");
    };
    assert!(report.restored_scene_ids.contains(&scene_id));
    let after = runtime.editor_snapshot().expect("reloaded snapshot");
    assert!(after
        .hierarchy
        .iter()
        .any(|node| node.scene_id.as_deref() == Some(&scene_id)));
    assert_eq!(translation(&after, &scene_id), json!([3.0, 1.0, 0.0]));
    let _ = fs::remove_dir_all(root);
}
