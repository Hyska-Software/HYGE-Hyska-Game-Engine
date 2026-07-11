//! R-084 command model and stale-revision evidence.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hyge_editor::{
    DestroyCommand, EditComponentCommand, EditorCommand, RemoveComponentCommand, ReparentCommand,
};
use hyge_scene::{
    Environment, PostProcessProfile, Prefab, PrefabAssets, PrefabInstance, PrefabNode, Transform,
    WorldDocument,
};

static NEXT_PROJECT: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    #[allow(dead_code)]
    pub(crate) prefab_id: hyge_scene::PrefabId,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hyge-r084-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("create fixture");
        let mut prefab_root = PrefabNode::named("Root");
        prefab_root.children.push(PrefabNode::named("Child"));
        let prefab = Prefab::new("r084", prefab_root, PrefabAssets::default());
        fs::write(
            root.join("root.hyge-prefab"),
            prefab.to_bytes().expect("prefab"),
        )
        .expect("write prefab");
        let document = WorldDocument {
            env: Environment::empty(),
            root_prefab_instances: vec![PrefabInstance::new(
                prefab.prefab_id,
                Transform::identity(),
            )],
            post_process: PostProcessProfile::default(),
            editor_layer: None,
        };
        fs::write(
            root.join("main.hyge-world"),
            document.to_bytes().expect("world"),
        )
        .expect("write world");
        Self {
            root,
            prefab_id: prefab.prefab_id,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn runtime(fixture: &Fixture) -> hyge_editor::EditorSessionRuntime {
    let mut runtime = hyge_editor::EditorSessionRuntime::new();
    runtime.open_project(&fixture.root).expect("project");
    runtime
        .open_scene(&fixture.root.join("main.hyge-world"))
        .expect("scene");
    runtime
}

#[test]
fn component_commands_apply_and_revert_with_explicit_failures() {
    let fixture = Fixture::new();
    let mut runtime = runtime(&fixture);
    let snapshot = runtime.editor_snapshot().expect("snapshot");
    let entity = snapshot
        .hierarchy
        .iter()
        .find(|node| node.name == "Child")
        .expect("child")
        .entity;
    let name_path = snapshot
        .component_catalog
        .iter()
        .find(|c| c.short_name == "Name")
        .expect("Name catalog")
        .type_path
        .clone();
    let name_value = snapshot
        .entities
        .iter()
        .find(|e| e.entity == entity)
        .and_then(|e| e.components.iter().find(|c| c.type_path == name_path))
        .and_then(|c| c.value.clone())
        .expect("name value");

    let before = snapshot;
    let (_, edited) = runtime
        .apply_command(
            before.revision,
            EditorCommand::EditComponent(EditComponentCommand::new(
                entity,
                name_path.clone(),
                name_value.clone(),
            )),
        )
        .expect("apply");
    runtime.undo_command(edited.revision).expect("revert");

    let (_, removed) = runtime
        .apply_command(
            runtime.editor_snapshot().expect("snapshot").revision,
            EditorCommand::RemoveComponent(RemoveComponentCommand::new(entity, name_path.clone())),
        )
        .expect("remove");
    runtime.undo_command(removed.revision).expect("restore");

    let failed = runtime
        .apply_command(
            runtime.editor_snapshot().expect("snapshot").revision,
            EditorCommand::Destroy(DestroyCommand::new(u64::MAX)),
        )
        .expect_err("invalid entity");
    assert_eq!(failed.code, "invalid_entity");
}

#[test]
fn stale_revision_rejects_without_mutating_snapshot() {
    let fixture = Fixture::new();
    let mut runtime = runtime(&fixture);
    let initial = runtime.editor_snapshot().expect("snapshot");
    let entity = initial.hierarchy[0].entity;
    runtime
        .apply_command(
            initial.revision,
            EditorCommand::Reparent(ReparentCommand::new(entity, None)),
        )
        .expect("edit");
    let after = runtime.editor_snapshot().expect("after");
    let error = runtime
        .apply_command(
            initial.revision,
            EditorCommand::Destroy(DestroyCommand::new(entity)),
        )
        .expect_err("stale");
    assert_eq!(error.code, "stale_revision");
    assert_eq!(
        runtime.editor_snapshot().expect("snapshot after rejection"),
        after
    );
}

#[test]
fn reparent_cycle_is_rejected_before_mutation() {
    let fixture = Fixture::new();
    let mut runtime = runtime(&fixture);
    let snapshot = runtime.editor_snapshot().expect("snapshot");
    let root = snapshot
        .hierarchy
        .iter()
        .find(|node| node.name == "Root")
        .expect("root")
        .entity;
    let child = snapshot
        .hierarchy
        .iter()
        .find(|node| node.name == "Child")
        .expect("child")
        .entity;
    let error = runtime
        .apply_command(
            snapshot.revision,
            EditorCommand::Reparent(ReparentCommand::new(root, Some(child))),
        )
        .expect_err("cycle");
    assert_eq!(error.code, "cycle_detected");
    assert_eq!(
        runtime.editor_snapshot().expect("unchanged").hierarchy,
        snapshot.hierarchy
    );
}
