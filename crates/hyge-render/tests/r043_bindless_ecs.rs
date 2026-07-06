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
use hyge_render::prelude::pod_collect_to_vec;
use hyge_render::renderer::Renderer;
use hyge_runtime_test::TestRenderer;
use hyge_scene::extract::render_extract;
use hyge_scene::prelude::{LightComponent, MaterialHandle, MeshHandle, WorldTransform};

fn build_snapshot_world() -> World {
    let mut world = World::new();

    // Sun.
    world.spawn(LightComponent::sun([0.0, -1.0, 0.0], [1.0, 0.95, 0.9], 1.0));

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
    // The world has 3 renderable entities across 2 unique
    // (mesh_id, material_id) pairs: (0, 0) x 2 and (1, 1) x
    // 1. R-043 acceptance #3 groups by (mesh, material)
    // -> 2 draw commands with instance_count {2, 1}.
    assert_eq!(snapshot.draw_count(), 2);
    assert_eq!(snapshot.instance_count(), 3);
    assert_eq!(snapshot.light_count(), 2);
    // Sanity: the two draw commands reflect the grouping.
    let counts: Vec<u32> = snapshot
        .draw_commands
        .iter()
        .map(|dc| dc.instance_count)
        .collect();
    assert!(counts.contains(&2) && counts.contains(&1));
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
    let mut renderer = Renderer::new_headless(&config).expect("headless renderer must construct");

    let mut world = build_snapshot_world();
    let snapshot = render_extract(&mut world);

    // The scene-side and render-side PODs have identical
    // memory layouts (asserted by the size-of tests above).
    // `pod_collect_to_vec` is the safe ABI bridge that
    // replaces the unsafe `from_raw_parts` cast.
    let instances: Vec<Instance> = pod_collect_to_vec(&snapshot.instances);
    let draw_commands: Vec<DrawCommand> = pod_collect_to_vec(&snapshot.draw_commands);
    let lights: Vec<Light> = pod_collect_to_vec(&snapshot.lights);

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

/// R-041 / R-042 acceptance: `Renderer::set_environment`
/// must take effect *after* the first `render_frame` call.
/// Previously the IBL was captured in the lazy
/// `ClusteredForwardPass` constructor and any subsequent
/// `set_environment` was silently ignored.
#[test]
fn set_environment_after_first_render_is_applied() {
    let Some(_test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let config = RendererConfig::default();
    let mut renderer = Renderer::new_headless(&config).expect("headless renderer must construct");

    let device: *const wgpu::Device = renderer.bindless().device();
    let mut world = build_snapshot_world();
    let snapshot = render_extract(&mut world);

    let instances: Vec<Instance> = pod_collect_to_vec(&snapshot.instances);
    let draw_commands: Vec<DrawCommand> = pod_collect_to_vec(&snapshot.draw_commands);
    let lights: Vec<Light> = pod_collect_to_vec(&snapshot.lights);

    let target = unsafe { &*device }.create_texture(&wgpu::TextureDescriptor {
        label: Some("r-043-env-test-target"),
        size: wgpu::Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    // Bake a minimal IBL so set_environment has something
    // to upload. The 8x8 sizes keep the bake in milliseconds.
    let bytes = {
        let mut out = Vec::new();
        out.extend_from_slice(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n");
        out.extend_from_slice(b"-Y 2 +X 4\n");
        for _ in 0..8 {
            out.extend_from_slice(&[128, 128, 128, 128]);
        }
        out
    };
    let cfg = hyge_render::ibl::BakeConfig {
        prefilter_size: 8,
        irradiance_size: 4,
        brdf_lut_size: 4,
        sample_count: 16,
    };
    let bake = hyge_render::ibl::bake_from_rgbe_hdr_with_config(&bytes, cfg)
        .expect("minimal bake must succeed");

    // First render (IBL = None).
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
            &FrameData::default_looking_at_origin(),
            &instances,
            &draw_commands,
            &lights,
        )
        .expect("first render_frame must succeed");

    // Now set the environment. The next render_frame must
    // bind the new IBL resources.
    renderer
        .set_environment(&bake)
        .expect("set_environment must succeed");

    // After set_environment, the renderer's IBL handle
    // must be Some.
    assert!(
        renderer.ibl().is_some(),
        "Renderer::ibl() must be Some after set_environment"
    );

    // Second render must also succeed (regression: the
    // bind-group rebuild could panic on the second frame).
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
            &FrameData::default_looking_at_origin(),
            &instances,
            &draw_commands,
            &lights,
        )
        .expect("second render_frame must succeed after set_environment");
}
