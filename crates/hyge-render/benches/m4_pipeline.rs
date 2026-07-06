use criterion::{black_box, criterion_group, criterion_main, Criterion};
use hyge_core::prelude::{Aabb, Mat4, Vec3};
use hyge_render::{cull, meshlet, post, shadow, skinning};

fn bench_shadows(c: &mut Criterion) {
    c.bench_function("m4/shadow_cascades_and_atlas", |b| {
        b.iter(|| {
            let splits = shadow::CascadeSplits::lambda_blend(0.1, 1_000.0, 4, 0.5);
            let cascades = shadow::build_cascade_data(&splits, Vec3::new(-0.3, -1.0, -0.2));
            let mut atlas = shadow::ShadowAtlasAllocator::new(4096, 4096);
            for _ in 0..16 {
                black_box(atlas.allocate_point_light(256));
            }
            black_box(cascades);
        });
    });
}

fn bench_post(c: &mut Criterion) {
    let input: Vec<f32> = (0..1920)
        .map(|i| if i % 64 == 0 { 8.0 } else { 0.25 })
        .collect();
    c.bench_function("m4/post_taa_smaa_bloom_aces", |b| {
        b.iter(|| {
            let bloom = post::apply_bloom_1d(
                black_box(&input),
                post::BloomConfig {
                    intensity: 0.2,
                    threshold: 1.0,
                    levels: 5,
                },
            );
            let smaa = post::smaa_smooth_1d(&bloom, 0.25);
            let mapped: Vec<f32> = smaa.iter().map(|x| post::aces_filmic(*x)).collect();
            black_box(mapped);
        });
    });
}

fn bench_culling(c: &mut Criterion) {
    let frustum = cull::SimpleFrustum::orthographic(-10.0, 10.0, -10.0, 10.0, -10.0, 10.0);
    let instances: Vec<_> = (0..10_000)
        .map(|i| {
            cull::CullInstance::new(
                Aabb::new(Vec3::splat(-0.25), Vec3::splat(0.25)),
                Mat4::from_translation(Vec3::new(
                    (i % 100) as f32 - 50.0,
                    (i / 100) as f32 - 50.0,
                    0.0,
                )),
            )
        })
        .collect();
    c.bench_function("m4/cpu_frustum_cull_10k", |b| {
        b.iter(|| black_box(cull::cull_instances(&frustum, black_box(&instances))));
    });
}

fn bench_meshlets(c: &mut Criterion) {
    let frustum = cull::SimpleFrustum::orthographic(-5.0, 5.0, -5.0, 5.0, -5.0, 5.0);
    let meshlets: Vec<_> = (0..10_000)
        .map(|i| meshlet::MeshletBounds {
            mesh_id: 0,
            meshlet_id: i,
            bounds: Aabb::new(Vec3::splat(-0.1), Vec3::splat(0.1)),
            transform: Mat4::from_translation(Vec3::new(
                (i % 100) as f32 - 50.0,
                (i / 100) as f32 - 50.0,
                0.0,
            )),
            screen_error: if i % 2 == 0 { 0.25 } else { 2.0 },
        })
        .collect();
    c.bench_function("m4/meshlet_cull_lod_10k", |b| {
        b.iter(|| {
            black_box(meshlet::cull_and_select_lod(
                &frustum,
                black_box(&meshlets),
                1.0,
            ))
        });
    });
}

fn bench_skinning(c: &mut Criterion) {
    let joints = [
        Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0)),
        Mat4::IDENTITY,
    ];
    let vertices = vec![
        skinning::SkinnedVertex {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            joint_indices: [0, 1, 2, 3],
            joint_weights: [0.25, 0.25, 0.25, 0.25],
        };
        10_000
    ];
    c.bench_function("m4/skinning_10k", |b| {
        b.iter(|| {
            black_box(skinning::skin_vertices(
                black_box(&vertices),
                black_box(&joints),
            ))
        });
    });
}

criterion_group!(
    m4_pipeline,
    bench_shadows,
    bench_post,
    bench_culling,
    bench_meshlets,
    bench_skinning
);
criterion_main!(m4_pipeline);
