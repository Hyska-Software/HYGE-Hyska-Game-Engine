//! R-084 mixed command and transactional history evidence.

use hyge_editor::{
    AddComponentCommand, DestroyCommand, DuplicateCommand, EditComponentCommand, EditorCommand,
    InstantiateCommand, RemoveComponentCommand, ReparentCommand,
};
use hyge_scene::Transform;

#[path = "r084_commands.rs"]
mod fixture;

#[test]
fn mixed_commands_undo_five_redo_three_and_save_preserves_history() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let initial = runtime.editor_snapshot().expect("initial snapshot");
    let root = initial
        .hierarchy
        .iter()
        .find(|node| node.name == "Root")
        .expect("root")
        .entity;
    let child = initial
        .hierarchy
        .iter()
        .find(|node| node.name == "Child")
        .expect("child")
        .entity;
    let name = initial
        .component_catalog
        .iter()
        .find(|component| component.short_name == "Name")
        .expect("Name descriptor");
    let name_value = initial
        .entities
        .iter()
        .find(|entity| entity.entity == child)
        .and_then(|entity| {
            entity
                .components
                .iter()
                .find(|component| component.type_id == name.type_id)
        })
        .and_then(|component| component.value.clone())
        .expect("Name value");

    let mut revision = initial.revision;
    let commands = vec![
        EditorCommand::Reparent(ReparentCommand::new(child, None)),
        EditorCommand::EditComponent(EditComponentCommand::new(
            child,
            name.type_path.clone(),
            name_value.clone(),
        )),
        EditorCommand::RemoveComponent(RemoveComponentCommand::new(child, name.type_path.clone())),
        EditorCommand::AddComponent(AddComponentCommand::new(
            child,
            name.type_path.clone(),
            name_value,
        )),
        EditorCommand::Duplicate(DuplicateCommand::new(root)),
        EditorCommand::Instantiate(InstantiateCommand::new(
            fixture.prefab_id,
            Transform::identity(),
            None,
        )),
        EditorCommand::Destroy(DestroyCommand::new(child)),
    ];
    for command in commands {
        revision = runtime
            .apply_command(revision, command)
            .expect("apply command")
            .1
            .revision;
    }

    for _ in 0..5 {
        revision = runtime.undo_command(revision).expect("undo").1.revision;
    }
    for _ in 0..3 {
        revision = runtime.redo_command(revision).expect("redo").1.revision;
    }
    assert!(runtime.can_undo());
    runtime.save_scene().expect("save");
    assert!(runtime.can_undo(), "save must preserve undo history");
    assert!(runtime.can_redo(), "save must preserve redo history");
    let saved_revision = runtime.editor_snapshot().expect("saved snapshot").revision;
    let stale = runtime
        .undo_command(saved_revision.saturating_sub(1))
        .expect_err("stale undo");
    assert_eq!(stale.code, "stale_revision");
}

#[test]
fn new_edit_clears_redo_history() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let snapshot = runtime.editor_snapshot().expect("snapshot");
    let entity = snapshot.hierarchy[0].entity;
    let revision = runtime
        .apply_command(
            snapshot.revision,
            EditorCommand::Reparent(ReparentCommand::new(entity, None)),
        )
        .expect("edit")
        .1
        .revision;
    let revision = runtime.undo_command(revision).expect("undo").1.revision;
    assert!(runtime.can_redo());
    runtime
        .apply_command(
            revision,
            EditorCommand::Reparent(ReparentCommand::new(entity, None)),
        )
        .expect("new edit");
    assert!(!runtime.can_redo());
}
