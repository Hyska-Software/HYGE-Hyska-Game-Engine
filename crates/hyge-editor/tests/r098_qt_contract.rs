//! R-098 batch reflection and refresh contract evidence.

use serde_json::json;
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use hyge_editor::{EditComponentsCommand, EditorCommand, EditorServer, EditorServerConfig};
use hyge_editor_protocol::{read_envelope, write_envelope, Envelope, MessageType};

#[path = "r084_commands.rs"]
mod fixture;

#[test]
fn batch_edit_applies_atomically_and_undoes_as_one_command() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let before = runtime.editor_snapshot().expect("snapshot");
    let entities: Vec<_> = before.hierarchy.iter().map(|node| node.entity).collect();
    let name_path = before
        .component_catalog
        .iter()
        .find(|component| component.short_name == "Name")
        .expect("Name descriptor")
        .type_path
        .clone();
    let name_value = json!({"hyge_scene::components::Name": ["Batch Edited"]});
    let (_, edited) = runtime
        .apply_command(
            before.revision,
            EditorCommand::EditComponents(EditComponentsCommand::new(
                entities.clone(),
                name_path.clone(),
                None,
                name_value,
            )),
        )
        .expect("batch edit");
    for entity in &entities {
        let value = edited
            .entities
            .iter()
            .find(|record| record.entity == *entity)
            .and_then(|record| {
                record
                    .components
                    .iter()
                    .find(|component| component.type_path == name_path)
            })
            .and_then(|component| component.value.clone())
            .expect("edited value");
        assert_eq!(
            value,
            json!({"hyge_scene::components::Name": ["Batch Edited"]})
        );
    }
    let restored = runtime.undo_command(edited.revision).expect("undo").1;
    assert_eq!(restored.revision, edited.revision + 1);
    assert_ne!(restored.entities, edited.entities);
}

#[test]
fn batch_edit_nested_field_is_atomic_and_invalid_entity_does_not_mutate() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let before = runtime.editor_snapshot().expect("snapshot");
    let entities: Vec<_> = before.hierarchy.iter().map(|node| node.entity).collect();
    let transform_path = before
        .component_catalog
        .iter()
        .find(|component| component.short_name == "Transform")
        .expect("Transform descriptor")
        .type_path
        .clone();
    let (_, edited) = runtime
        .apply_command(
            before.revision,
            EditorCommand::EditComponents(EditComponentsCommand::new(
                entities.clone(),
                transform_path.clone(),
                Some("translation".into()),
                json!([2.0, 3.0, 4.0]),
            )),
        )
        .expect("nested batch edit");
    assert!(edited
        .entities
        .iter()
        .all(
            |record| record.components.iter().any(|component| component.type_path
                == transform_path
                && component
                    .value
                    .as_ref()
                    .and_then(|value| value.get(&transform_path))
                    .and_then(|value| value.get("translation"))
                    .is_some_and(|value| value == &json!([2.0, 3.0, 4.0])))
        ));

    let unchanged = edited.clone();
    let failure = runtime
        .apply_command(
            edited.revision,
            EditorCommand::EditComponents(EditComponentsCommand::new(
                vec![entities[0], u64::MAX],
                "hyge_scene::components::Name",
                None,
                json!({"value": "must not apply"}),
            )),
        )
        .expect_err("invalid entity must fail");
    assert_eq!(failure.code, "invalid_entity");
    assert_eq!(runtime.editor_snapshot().expect("unchanged"), unchanged);
}

#[test]
fn stale_batch_edit_is_rejected_without_mutating_snapshot() {
    let fixture = fixture::Fixture::new();
    let mut runtime = fixture::runtime(&fixture);
    let before = runtime.editor_snapshot().expect("snapshot");
    let entity = before.hierarchy[0].entity;
    runtime
        .apply_command(
            before.revision,
            EditorCommand::Reparent(hyge_editor::ReparentCommand::new(entity, None)),
        )
        .expect("first edit");
    let current = runtime.editor_snapshot().expect("current");
    let error = runtime
        .apply_command(
            before.revision,
            EditorCommand::EditComponents(EditComponentsCommand::new(
                vec![entity],
                "hyge_scene::components::Name",
                None,
                json!({"value": "stale"}),
            )),
        )
        .expect_err("stale edit");
    assert_eq!(error.code, "stale_revision");
    assert_eq!(runtime.editor_snapshot().expect("still current"), current);
}

#[test]
fn refresh_and_batch_edit_round_trip_through_real_tcp_service() {
    let fixture = fixture::Fixture::new();
    let server = Arc::new(EditorServer::bind(EditorServerConfig::default()).expect("bind"));
    let address = server.local_addr().expect("address");
    let server_thread = Arc::clone(&server);
    let thread = thread::spawn(move || server_thread.run().expect("server"));
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    write_envelope(&mut stream, &Envelope::hello("hello", "hyge-local-dev")).expect("hello");
    assert_eq!(
        read_envelope(&mut stream).expect("hello ack").message_type,
        MessageType::HelloAck
    );
    write_envelope(
        &mut stream,
        &Envelope::new(
            "project",
            MessageType::OpenProject,
            json!({"path": fixture.root}),
        ),
    )
    .expect("project");
    let _ = read_envelope(&mut stream).expect("project loading");
    let _ = read_envelope(&mut stream).expect("project ready");
    let _ = read_envelope(&mut stream).expect("project completed");
    write_envelope(
        &mut stream,
        &Envelope::new(
            "scene",
            MessageType::OpenScene,
            json!({"path": fixture.root.join("main.hyge-world")}),
        ),
    )
    .expect("scene");
    let _ = read_envelope(&mut stream).expect("scene loading");
    let _ = read_envelope(&mut stream).expect("scene ready");
    let scene_snapshot = read_envelope(&mut stream).expect("scene world");
    let entity = scene_snapshot.payload["hierarchy"][0]["entity"]
        .as_u64()
        .expect("entity");
    let revision = scene_snapshot.payload["revision"]
        .as_u64()
        .expect("revision");
    let _ = read_envelope(&mut stream).expect("scene selection");
    let _ = read_envelope(&mut stream).expect("scene completed");
    write_envelope(
        &mut stream,
        &Envelope::new("refresh", MessageType::RequestWorldSnapshot, json!({})),
    )
    .expect("refresh");
    assert_eq!(
        read_envelope(&mut stream).expect("world").message_type,
        MessageType::WorldSnapshot
    );
    assert_eq!(
        read_envelope(&mut stream).expect("selection").message_type,
        MessageType::SelectionChanged
    );
    write_envelope(&mut stream, &Envelope::new("edit", MessageType::EditComponents, json!({"expected_revision": revision, "entities": [entity], "type_path": "hyge_scene::components::Name", "value": {"hyge_scene::components::Name": ["tcp"]}}))).expect("batch edit");
    assert_eq!(
        read_envelope(&mut stream)
            .expect("edited world")
            .message_type,
        MessageType::WorldSnapshot
    );
    assert_eq!(
        read_envelope(&mut stream)
            .expect("edited selection")
            .message_type,
        MessageType::SelectionChanged
    );
    assert_eq!(
        read_envelope(&mut stream)
            .expect("edited completed")
            .message_type,
        MessageType::CommandCompleted
    );
    write_envelope(
        &mut stream,
        &Envelope::new("shutdown", MessageType::ServerShutdown, json!({})),
    )
    .expect("shutdown");
    assert_eq!(
        read_envelope(&mut stream)
            .expect("shutdown response")
            .message_type,
        MessageType::ServerShutdown
    );
    drop(stream);
    thread.join().expect("server thread");
}
