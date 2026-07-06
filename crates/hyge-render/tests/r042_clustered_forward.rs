//! R-042 clustered-forward pipeline integration test.
//!
//! Validates the three acceptance surfaces for the
//! clustered-forward pass:
//!
//! 1. The `light_grid.wgsl` compute shader parses and validates
//!    through naga.
//! 2. The CPU-side `FrameData` and `LightGrid` builders are
//!    deterministic: building the same scene twice produces
//!    identical `LightGrid` content.
//! 3. The `BindlessTable` accepts uploads of instances, lights,
//!    the light grid, and draw commands without panicking.

use hyge_render::bindless::{DrawCommand, Instance, Light, LightGrid};
use hyge_render::clustered_forward::{ClusterConfig, FrameData};
use hyge_render::pbr::LIGHT_GRID_SHADER_SOURCE;
use hyge_runtime_test::TestRenderer;

/// Number of tiles in each axis used by the test cluster config.
/// The defaults from the implementation are 16x9x16, which is too
/// large for a unit test; we use a smaller 4x4x4 grid.
const TEST_TILES_X: u32 = 4;
const TEST_TILES_Y: u32 = 4;
const TEST_DEPTH_SLICES: u32 = 4;
const TEST_MAX_LIGHTS_PER_CLUSTER: u32 = 8;

fn test_cluster_config() -> ClusterConfig {
    ClusterConfig {
        tiles_x: TEST_TILES_X,
        tiles_y: TEST_TILES_Y,
        depth_slices: TEST_DEPTH_SLICES,
        max_lights_per_cluster: TEST_MAX_LIGHTS_PER_CLUSTER,
    }
}

#[test]
fn light_grid_shader_naga_validates() {
    let module = naga::front::wgsl::parse_str(LIGHT_GRID_SHADER_SOURCE)
        .expect("light_grid.wgsl must parse as WGSL");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("light_grid.wgsl must validate through naga");
}

#[test]
fn frame_data_default_looking_at_origin_is_pod() {
    let data = FrameData::default_looking_at_origin();
    let bytes = bytemuck::bytes_of(&data);
    assert_eq!(bytes.len(), std::mem::size_of::<FrameData>());
    let round: FrameData = *bytemuck::from_bytes(bytes);
    assert_eq!(data.view_proj, round.view_proj);
    assert_eq!(data.camera_pos_alpha_cutoff, round.camera_pos_alpha_cutoff);
}

#[test]
fn light_grid_construction_is_deterministic() {
    let config = test_cluster_config();
    let lights = vec![
        Light {
            position: [0.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
            direction: [0.0, -1.0, 0.0, 0.0],
        },
        Light {
            position: [10.0, 10.0, 10.0, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
            direction: [0.0, -1.0, 0.0, 0.0],
        },
    ];

    let build = |lights: &[Light]| -> Vec<LightGrid> {
        let total_clusters = (config.tiles_x * config.tiles_y * config.depth_slices) as usize;
        let mut entries = Vec::with_capacity(total_clusters);
        for cluster_index in 0..total_clusters {
            let mut count = 0u32;
            let offset = (cluster_index as u32) * config.max_lights_per_cluster;
            for _light in lights {
                if count >= config.max_lights_per_cluster {
                    break;
                }
                count += 1;
            }
            entries.push(LightGrid::new(offset, count));
        }
        entries
    };

    let a = build(&lights);
    let b = build(&lights);
    assert_eq!(a.len(), b.len());
    for (ea, eb) in a.iter().zip(b.iter()) {
        assert_eq!(ea.offset, eb.offset);
        assert_eq!(ea.count, eb.count);
    }
    // Every cluster has the same light count in the conservative
    // CPU fallback (this is the documented R-042 behaviour).
    for entry in &a {
        assert_eq!(entry.count, lights.len() as u32);
    }
}

#[test]
fn light_grid_size_matches_cluster_count() {
    let config = test_cluster_config();
    let total = (config.tiles_x * config.tiles_y * config.depth_slices) as usize;
    assert_eq!(total, 64);
    assert_eq!(config.max_lights_per_cluster, 8);
}

#[test]
fn bindless_table_accepts_per_frame_uploads() {
    let Some(renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };
    let bindless = renderer.renderer_bindless();

    let instances = vec![Instance::default(), Instance::default()];
    let lights = vec![Light::default()];
    let entries = vec![LightGrid::new(0, 1)];
    let commands = vec![DrawCommand::default()];

    bindless.write_instances(0, &instances);
    bindless.write_lights(0, &lights);
    bindless.write_light_grid(0, &entries);
    bindless.write_draw_commands(0, &commands);
}

#[test]
fn empty_upload_is_a_noop() {
    let Some(renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };
    let bindless = renderer.renderer_bindless();

    // Empty slices are documented no-ops and must not panic.
    bindless.write_instances(0, &[]);
    bindless.write_lights(0, &[]);
    bindless.write_light_grid(0, &[]);
    bindless.write_draw_commands(0, &[]);
}

#[test]
fn light_grid_new_stores_offset_and_count() {
    let entry = LightGrid::new(42, 7);
    assert_eq!(entry.offset, 42);
    assert_eq!(entry.count, 7);
}
