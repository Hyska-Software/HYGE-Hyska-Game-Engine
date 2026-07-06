use hyge_core::prelude::{Aabb, Mat4, Vec3};
use hyge_render::{cull, meshlet, post, shadow, skinning};

#[test]
fn r050_csm_splits_are_monotonic_and_end_at_far() {
    let splits = shadow::CascadeSplits::lambda_blend(0.1, 100.0, 4, 0.5);
    assert_eq!(splits.distances.len(), 4);
    assert!(splits.distances.windows(2).all(|w| w[0] < w[1]));
    assert!((splits.distances[3] - 100.0).abs() < 0.001);
}

#[test]
fn r051_shadow_atlas_allocates_point_light_faces_contiguously() {
    let mut atlas = shadow::ShadowAtlasAllocator::new(4096, 4096);
    let allocation = atlas
        .allocate_point_light(512)
        .expect("point light must fit");
    assert_eq!(allocation.rects.len(), 6);
    assert!(allocation
        .rects
        .iter()
        .all(|r| r.width == 512 && r.height == 512));
    assert!(allocation.rects.windows(2).all(|w| w[0].x <= w[1].x));
}

#[test]
fn r055_aces_maps_linear_one_to_expected_range() {
    let linear = post::aces_filmic(1.0);
    assert!((0.80..=0.82).contains(&linear), "got {linear}");
}

#[test]
fn r054_bloom_increases_mean_luma_for_bright_pixels() {
    let input = [0.0, 0.0, 8.0, 0.0];
    let output = post::apply_bloom_1d(
        &input,
        post::BloomConfig {
            intensity: 0.5,
            threshold: 1.0,
            levels: 3,
        },
    );
    let before = post::mean_luma(&input);
    let after = post::mean_luma(&output);
    assert!(after > before, "before={before} after={after}");
}

#[test]
fn r052_taa_reduces_temporal_variance_and_resets_on_cut() {
    let mut taa = post::TaaHistory::new(4);
    let frame_a = [0.0, 1.0, 0.0, 1.0];
    let frame_b = [1.0, 0.0, 1.0, 0.0];
    let blended_a = taa.resolve(&frame_a, Mat4::IDENTITY);
    let blended_b = taa.resolve(&frame_b, Mat4::IDENTITY);
    assert!(post::variance(&blended_b) < post::variance(&frame_b));
    let cut = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
    let reset = taa.resolve(&frame_a, cut);
    assert_eq!(reset, frame_a);
    assert_eq!(blended_a, frame_a);
}

#[test]
fn r053_smaa_smooths_hard_edge_without_changing_flat_regions() {
    let input = [0.0, 0.0, 1.0, 1.0];
    let output = post::smaa_smooth_1d(&input, 0.25);
    assert!(output[1] > input[1]);
    assert!(output[2] < input[2]);
    assert_eq!(output[0], 0.0);
    assert_eq!(output[3], 1.0);
}

#[test]
fn r057_cpu_frustum_culls_most_static_instances() {
    let frustum = cull::SimpleFrustum::orthographic(-10.0, 10.0, -10.0, 10.0, -10.0, 10.0);
    let instances: Vec<_> = (0..10_000)
        .map(|i| {
            let x = (i % 100) as f32 - 50.0;
            let y = (i / 100) as f32 - 50.0;
            cull::CullInstance::new(
                Aabb::new(Vec3::splat(-0.25), Vec3::splat(0.25)),
                Mat4::from_translation(Vec3::new(x, y, 0.0)),
            )
        })
        .collect();
    let visible = cull::cull_instances(&frustum, &instances);
    assert!(visible.len() < 1_000, "{} visible", visible.len());
}

#[test]
fn r058_meshlet_cull_selects_lod_and_visible_count() {
    let frustum = cull::SimpleFrustum::orthographic(-5.0, 5.0, -5.0, 5.0, -5.0, 5.0);
    let meshlets: Vec<_> = (0..10_000)
        .map(|i| meshlet::MeshletBounds {
            mesh_id: 0,
            meshlet_id: i,
            bounds: Aabb::new(Vec3::new(-0.1, -0.1, -0.1), Vec3::new(0.1, 0.1, 0.1)),
            transform: Mat4::from_translation(Vec3::new(
                (i % 100) as f32 - 50.0,
                (i / 100) as f32 - 50.0,
                0.0,
            )),
            screen_error: if i % 2 == 0 { 0.25 } else { 2.0 },
        })
        .collect();
    let visible = meshlet::cull_and_select_lod(&frustum, &meshlets, 1.0);
    assert!(visible.len() < 1_000, "{} visible", visible.len());
    assert!(visible.iter().any(|v| v.lod == 0));
    assert!(visible.iter().any(|v| v.lod == 1));
}

#[test]
fn r056_skinning_four_influences_matches_expected_position() {
    let joints = [
        Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0)),
        Mat4::IDENTITY,
    ];
    let vertex = skinning::SkinnedVertex {
        position: Vec3::ZERO,
        normal: Vec3::Y,
        joint_indices: [0, 1, 2, 3],
        joint_weights: [0.25, 0.25, 0.25, 0.25],
    };
    let out = skinning::skin_vertex(&vertex, &joints);
    assert!((out.position - Vec3::new(0.25, 0.5, 0.75)).length() < 0.0001);
}
