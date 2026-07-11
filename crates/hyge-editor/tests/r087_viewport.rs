//! R-087 editor viewport camera and render-bridge evidence.

use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::world::World;
use hyge_editor::{EditorCameraState, EditorRenderBridge, EditorSessionRuntime};
use hyge_render::prelude::{RenderView, RendererConfig};
use hyge_scene::extract::render_extract;
use hyge_scene::prelude::{LightComponent, MaterialHandle, MeshHandle, WorldTransform};

fn known_scene() -> hyge_scene::extract::FrameSnapshot {
    let mut world = World::new();
    world.spawn(LightComponent::sun([0.0, -1.0, 0.0], [1.0, 0.95, 0.9], 1.0));
    world.spawn((
        MeshHandle(0),
        MaterialHandle(0),
        WorldTransform::from_translation(0.0, 0.0, 0.0),
    ));
    render_extract(&mut world)
}

#[test]
fn editor_camera_is_session_owned_and_validated() {
    let mut runtime = EditorSessionRuntime::new();
    let camera = EditorCameraState {
        position: [2.0, 1.0, 6.0],
        ..EditorCameraState::default()
    };
    runtime.set_editor_camera(camera).expect("valid camera");
    assert_eq!(runtime.editor_camera().position, [2.0, 1.0, 6.0]);
    assert_eq!(runtime.viewport_state().camera_revision, 2);
    assert!(runtime
        .set_editor_camera(EditorCameraState {
            fov_degrees: f32::NAN,
            ..camera
        })
        .is_err());
    assert_eq!(runtime.editor_camera().position, [2.0, 1.0, 6.0]);
}

#[test]
fn viewport_resize_clamps_and_invalidates_frame() {
    let mut runtime = EditorSessionRuntime::new();
    runtime.set_viewport_size(0, 0);
    let viewport = runtime.viewport_state();
    assert_eq!((viewport.width, viewport.height), (1, 1));
    assert_eq!(viewport.last_frame_revision, None);
}

#[test]
fn known_scene_renders_deterministic_editor_frame_and_resizes() {
    let bridge = match EditorRenderBridge::new(RendererConfig::default()) {
        Ok(bridge) => bridge,
        Err(error) if error.contains("no wgpu adapter") => {
            eprintln!("skipping: no wgpu adapter available");
            return;
        }
        Err(error) => panic!("editor renderer failed to start: {error}"),
    };
    let snapshot = known_scene();
    let view = RenderView::editor_default(32, 24);
    bridge
        .submit(7, view, &snapshot)
        .expect("submit first frame");
    let deadline = Instant::now() + Duration::from_secs(10);
    let first = loop {
        if let Some(frame) = bridge.try_receive() {
            break frame.expect("first editor frame");
        }
        assert!(Instant::now() < deadline, "editor frame timed out");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((first.width, first.height, first.revision), (32, 24, 7));
    assert_eq!(first.pixels.len(), 32 * 24 * 4);
    assert!(!first.hash.is_empty());

    bridge
        .submit(7, view, &snapshot)
        .expect("submit repeated frame");
    let deadline = Instant::now() + Duration::from_secs(10);
    let repeated = loop {
        if let Some(frame) = bridge.try_receive() {
            break frame.expect("repeated editor frame");
        }
        assert!(Instant::now() < deadline, "repeated editor frame timed out");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        repeated.hash, first.hash,
        "editor frame must be deterministic"
    );

    bridge
        .submit(8, RenderView::editor_default(16, 12), &snapshot)
        .expect("submit resized frame");
    let deadline = Instant::now() + Duration::from_secs(10);
    let second = loop {
        if let Some(frame) = bridge.try_receive() {
            break frame.expect("resized editor frame");
        }
        assert!(Instant::now() < deadline, "resized editor frame timed out");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((second.width, second.height, second.revision), (16, 12, 8));
    assert_eq!(second.pixels.len(), 16 * 12 * 4);
}
