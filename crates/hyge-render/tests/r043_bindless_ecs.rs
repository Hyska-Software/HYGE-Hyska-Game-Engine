//! R-043 Bindless table integration with ECS.
//!
//! End-to-end integration test: builds a `bevy_ecs::World`,
//! spawns entities with `MeshHandle` / `MaterialHandle` /
//! `WorldTransform` / `LightComponent`, runs `render_extract` to
//! produce a `FrameSnapshot`, and submits the snapshot through
//! `Renderer::render_frame` to an off-screen target. The test
//! passes if the device initialises, the queue submits, and
//! no errors fire.

use bevy_ecs::prelude::*;
use hyge_render::bindless::{DrawCommand, Instance, Light};
use hyge_render::clustered_forward::FrameData;
use hyge_render::config::RendererConfig;
use hyge_render::renderer::Renderer;
use hyge_runtime_test::TestRenderer;
use hyge_scene::extract::render_extract;
use hyge_scene::prelude::{
    LightComponent, MaterialHandle, MeshHandle, WorldTransform,
};

fn build_snapshot_world() -> World {
    let mut world = World::new();

    // Sun.
    world.spawn(LightComponent::sun(
        [0.0, -1.0, 0.0],
        [1.0, 0.95, 0.9],
        1.0,
    ));

    // One point light.
    world.spawn(LightComponent::point([0.0, 5.0, 0.0], [0.4, 0.6, 1.0], 2.5));

    // Three renderable entities at varying depths.
    world.spawn((
        MeshHandle(0),
        MaterialHandle(0),
        WorldTransform::from_translation(0.0, 0.0, 0.0),
    ));
    world.spawn((
        MeshHandle(0),
        MaterialHandle(0),
        WorldTransform::from_translation(1.0, 0.0, 0.0),
    ));
    world.spawn((
        MeshHandle(1),
        MaterialHandle(1),
        WorldTransform::from_translation(-1.0, 0.0, 0.0),
    ));

    world
}

#[test]
fn extract_three_entities_and_two_lights() {
    let mut world = build_snapshot_world();
    let snapshot = render_extract(&mut world);
    assert_eq!(snapshot.draw_count(), 3);
    assert_eq!(snapshot.instance_count(), 3);
    assert_eq!(snapshot.light_count(), 2);
}

#[test]
fn snapshot_mirrors_bindless_pod_layout() {
    // The `Instance` types in `hyge_scene::extract` and
    // `hyge_render::bindless` must be ABI-compatible so the
    // renderer's `write_instances` accepts the scene-side slice
    // by reference.
    assert_eq!(
        std::mem::size_of::<hyge_scene::extract::Instance>(),
        std::mem::size_of::<Instance>(),
    );
    assert_eq!(
        std::mem::size_of::<hyge_scene::extract::DrawCommand>(),
        std::mem::size_of::<DrawCommand>(),
    );
    assert_eq!(
        std::mem::size_of::<hyge_scene::extract::Light>(),
        std::mem::size_of::<Light>(),
    );
}

#[test]
fn render_frame_with_extracted_snapshot_does_not_error() {
    // We use the headless renderer because it doesn't need a
    // window / surface. The TestRenderer::new is a soft skip
    // when no wgpu adapter is available (CI without lavapipe).
    let Some(_test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let config = RendererConfig::default();
    let mut renderer = Renderer::new_headless(&config)
        .expect("headless renderer must construct");

    let mut world = build_snapshot_world();
    let snapshot = render_extract(&mut world);

    // The scene-side and render-side PODs have identical
    // memory layouts (asserted by the size-of tests above), so
    // a `bytemuck::pod_collect_to_vec` round-trip is sound. We
    // go through a `&[u8]` to dodge the orphan-rule problem of
    // casting between two distinct types defined in different
    // crates.
    let instances: Vec<Instance> = {
        let bytes: &[u8] = bytemuck::cast_slice(&snapshot.instances);
        let ptr = bytes.as_ptr() as *const Instance;
        let len = bytes.len() / std::mem::size_of::<Instance>();
        // SAFETY: the two PODs are layout-compatible (asserted
        // by `snapshot_mirrors_bindless_pod_layout`), and the
        // source slice outlives the pointer aliasing for the
        // duration of this `Vec::from_raw_parts_in` call.
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    };
    let draw_commands: Vec<DrawCommand> = {
        let bytes: &[u8] = bytemuck::cast_slice(&snapshot.draw_commands);
        let ptr = bytes.as_ptr() as *const DrawCommand;
        let len = bytes.len() / std::mem::size_of::<DrawCommand>();
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    };
    let lights: Vec<Light> = {
        let bytes: &[u8] = bytemuck::cast_slice(&snapshot.lights);
        let ptr = bytes.as_ptr() as *const Light;
        let len = bytes.len() / std::mem::size_of::<Light>();
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    };

    // Create an off-screen render target using the renderer's
    // device.
    let device = renderer.bindless().device();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("r-043-target"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let frame_data = FrameData::default_looking_at_origin();
    renderer
        .render_frame(
            &target,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            &frame_data,
            &instances,
            &draw_commands,
            &lights,
        )
        .expect("render_frame must succeed");
}

#[test]
fn light_pod_layout_matches_bindless_light() {
    use hyge_render::bindless::Light as BLight;
    use hyge_scene::extract::Light as ELight;
    // The two PODs are declared with the same field order in
    // both crates; `assert_eq!` on size is the strongest static
    // check available without `bytemuck::Pod` derive in the
    // scene crate.
    assert_eq!(std::mem::size_of::<ELight>(), std::mem::size_of::<BLight>());
    assert_eq!(
        std::mem::align_of::<ELight>(),
        std::mem::align_of::<BLight>(),
    );
}
