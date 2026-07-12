//! R-101 editor-session watcher and conflict evidence.

#[path = "r084_commands.rs"]
mod fixture;

use std::{thread, time::Duration};

use hyge_editor::{EditorSessionRuntime, SceneReloadEvent};
use hyge_scene::WorldDocument;

#[test]
fn editor_session_detects_real_world_change_and_refreshes_snapshot() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let before = runtime.editor_snapshot().expect("snapshot before");
    let mut document = WorldDocument::from_bytes(
        &std::fs::read(fixture.root.join("main.hyge-world")).expect("read scene"),
    )
    .expect("decode scene");
    document.post_process.exposure = 1.5;
    std::fs::write(
        fixture.root.join("main.hyge-world"),
        document.to_bytes().expect("encode scene"),
    )
    .expect("write scene");

    let event = (0..60).find_map(|_| {
        thread::sleep(Duration::from_millis(20));
        runtime.poll_scene_reload().expect("poll")
    });
    let Some(SceneReloadEvent::Reloaded(report)) = event else {
        panic!("watcher did not produce a reload event");
    };
    assert!(report.diff.post_process_changed);
    let after = runtime.editor_snapshot().expect("snapshot after");
    assert!(after.revision > before.revision);
    assert!(!runtime.is_scene_dirty());
}

#[test]
fn editor_session_reports_conflict_and_accepts_keep_editor() {
    let fixture = fixture::Fixture::new();
    let mut runtime: EditorSessionRuntime = fixture::runtime(&fixture);
    let revision = runtime.editor_snapshot().expect("snapshot").revision;
    let before = runtime.editor_snapshot().expect("snapshot");
    let entity = before.entities.first().expect("entity").entity;
    let name_component = before
        .entities
        .iter()
        .flat_map(|entity| entity.components.iter())
        .find(|component| component.type_path.ends_with("::Name"))
        .expect("name component");
    let _ = runtime
        .apply_command(
            revision,
            hyge_editor::EditorCommand::EditComponent(hyge_editor::EditComponentCommand::new(
                entity,
                name_component.type_path.clone(),
                name_component.value.clone().expect("name value"),
            )),
        )
        .expect("local edit");
    let mut document = WorldDocument::from_bytes(
        &std::fs::read(fixture.root.join("main.hyge-world")).expect("read scene"),
    )
    .expect("decode scene");
    document.post_process.exposure = 2.0;
    std::fs::write(
        fixture.root.join("main.hyge-world"),
        document.to_bytes().expect("encode scene"),
    )
    .expect("write scene");
    let event = (0..60).find_map(|_| {
        thread::sleep(Duration::from_millis(20));
        runtime.poll_scene_reload().expect("poll")
    });
    assert!(matches!(event, Some(SceneReloadEvent::Conflict(_))));
    assert!(runtime
        .resolve_scene_reload("keep_editor")
        .expect("decision")
        .is_none());
    assert!(runtime.is_scene_dirty());
}
