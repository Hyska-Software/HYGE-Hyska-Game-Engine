//! R-085 hierarchy, selection and persistent scene editing evidence.

use hyge_editor::{DestroyCommand, DuplicateCommand, EditorCommand, ReparentCommand};

#[path = "r084_commands.rs"]
mod fixture;

#[test]
fn shift_selection_and_reparent_undo_update_fresh_snapshot() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let initial = runtime.editor_snapshot().expect("initial snapshot");
    let root = initial
        .hierarchy
        .iter()
        .find(|node| node.name == "Root")
        .expect("root");
    let child = initial
        .hierarchy
        .iter()
        .find(|node| node.name == "Child")
        .expect("child");
    assert!(root.scene_id.is_some());
    assert!(child.scene_id.is_some());

    let _selected = runtime
        .select_entities(vec![child.entity])
        .expect("single selection");
    let selected = runtime
        .select_entities_with_shift(vec![root.entity], true)
        .expect("shift selection");
    assert_eq!(selected.selection, vec![root.entity, child.entity]);

    let reparented = runtime
        .apply_command(
            selected.revision,
            EditorCommand::Reparent(ReparentCommand::new(child.entity, None)),
        )
        .expect("reparent")
        .1;
    let child_after = reparented
        .hierarchy
        .iter()
        .find(|node| node.entity == child.entity)
        .expect("child after reparent");
    assert_eq!(child_after.parent, None);
    assert!(!reparented
        .hierarchy
        .iter()
        .find(|node| node.entity == root.entity)
        .expect("root after reparent")
        .children
        .contains(&child.entity));

    let undone = runtime.undo_command(reparented.revision).expect("undo").1;
    let child_restored = undone
        .hierarchy
        .iter()
        .find(|node| node.entity == child.entity)
        .expect("child after undo");
    assert_eq!(child_restored.parent, Some(root.entity));
    assert_eq!(child_restored.scene_id, child.scene_id);
}

#[test]
fn duplicate_and_destroy_preserve_parent_order_ids_and_reopen() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let initial = runtime.editor_snapshot().expect("initial snapshot");
    let root = initial
        .hierarchy
        .iter()
        .find(|node| node.name == "Root")
        .expect("root");
    let child = initial
        .hierarchy
        .iter()
        .find(|node| node.name == "Child")
        .expect("child");
    let child_scene_id = child.scene_id.clone();

    let duplicated = runtime
        .apply_command(
            initial.revision,
            EditorCommand::Duplicate(DuplicateCommand::new(child.entity)),
        )
        .expect("duplicate");
    let duplicate_id = duplicated
        .0
        .entity_remappings
        .get(&child.entity)
        .copied()
        .expect("duplicate mapping");
    let snapshot = duplicated.1;
    let duplicate = snapshot
        .hierarchy
        .iter()
        .find(|node| node.entity == duplicate_id)
        .expect("duplicate node");
    assert_eq!(duplicate.parent, Some(root.entity));
    assert_ne!(duplicate.scene_id, child_scene_id);
    let children = &snapshot
        .hierarchy
        .iter()
        .find(|node| node.entity == root.entity)
        .expect("root with duplicate")
        .children;
    assert_eq!(children, &vec![child.entity, duplicate_id]);

    let destroyed = runtime
        .apply_command(
            snapshot.revision,
            EditorCommand::Destroy(DestroyCommand::new(child.entity)),
        )
        .expect("destroy")
        .1;
    assert!(destroyed
        .hierarchy
        .iter()
        .all(|node| node.entity != child.entity));
    let restored = runtime
        .undo_command(destroyed.revision)
        .expect("undo destroy")
        .1;
    let restored_child = restored
        .hierarchy
        .iter()
        .find(|node| node.entity == child.entity)
        .expect("restored child");
    assert_eq!(restored_child.scene_id, child_scene_id);
    assert_eq!(restored_child.parent, Some(root.entity));

    runtime.save_scene().expect("save edited scene");
    drop(runtime);
    let mut reopened = hyge_editor::EditorSessionRuntime::new();
    reopened
        .open_project(&fixture.root)
        .expect("reopen project");
    reopened
        .open_scene(&fixture.root.join("main.hyge-world"))
        .expect("reopen scene");
    let reopened_snapshot = reopened.editor_snapshot().expect("reopened snapshot");
    assert_eq!(reopened_snapshot.hierarchy, restored.hierarchy);
    assert_eq!(
        reopened_snapshot
            .hierarchy
            .iter()
            .find(|node| node.entity == child.entity)
            .and_then(|node| node.scene_id.clone()),
        child_scene_id
    );
}

#[test]
fn legacy_scene_ids_are_deterministic_across_runtime_loads() {
    let fixture = fixture::Fixture::new();
    let first = fixture::runtime(&fixture).editor_snapshot().expect("first");
    let second = fixture::runtime(&fixture)
        .editor_snapshot()
        .expect("second");
    let first_ids: Vec<_> = first
        .hierarchy
        .iter()
        .map(|node| node.scene_id.clone())
        .collect();
    let second_ids: Vec<_> = second
        .hierarchy
        .iter()
        .map(|node| node.scene_id.clone())
        .collect();
    assert_eq!(first_ids, second_ids);
}
