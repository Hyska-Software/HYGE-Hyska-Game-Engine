//! R-065 M6 prep smoke: 5 prefab instance scene renders correctly.

use std::sync::Arc;

use bevy_ecs::prelude::World;
use hyge_asset::importer::material::MaterialData;
use hyge_asset::importer::mesh::{MeshData, Vertex};
use hyge_asset::prelude::{
    material_upload_task, mesh_upload_task, AssetId, AssetServer, MaterialAsset, MeshAsset,
};
use hyge_ecs::AppTypeRegistry;
use hyge_render::bindless::{DrawCommand, Instance, Light};
use hyge_render::clustered_forward::FrameData;
use hyge_render::prelude::pod_collect_to_vec;
use hyge_render::renderer::Renderer;
use hyge_runtime_test::{capture_frame, TestRenderer};
use hyge_scene::prelude::{
    build_scene_type_registry, load_world_document_from_bytes, resolve_static_mesh_asset_refs,
    LoadedSceneState, PostProcessProfile, Prefab, PrefabAssets, PrefabInstance, PrefabLibrary,
    PrefabNode, SceneEnvironmentState, ScenePostProcessState, SerializedComponentOverride,
    StaticMesh, StaticMeshAssetRefs, Transform, WorldDocument,
};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 96;

fn tiny_mesh() -> MeshData {
    MeshData::from_triangle_list(
        vec![
            Vertex {
                position: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            },
            Vertex {
                position: [1.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
            },
            Vertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 1.0],
            },
        ],
        vec![0, 1, 2],
    )
}

fn sample_prefab(mesh: AssetId, material: AssetId) -> Prefab {
    let registry = build_scene_type_registry();
    let mut root = PrefabNode::named("scifi-helmet-root");
    root.components.push(
        SerializedComponentOverride::new(
            "hyge_scene::components::StaticMeshAssetRefs",
            &StaticMeshAssetRefs::new(mesh, material),
            &registry,
        )
        .expect("static mesh refs serialize"),
    );
    Prefab::new(
        "scifi-helmet",
        root,
        PrefabAssets {
            meshes: vec![mesh],
            materials: vec![material],
            scripts: Vec::new(),
        },
    )
}

fn sample_world(prefab_id: hyge_scene::PrefabId, skybox: AssetId) -> WorldDocument {
    WorldDocument {
        env: hyge_scene::Environment {
            skybox: Some(skybox),
            sun: Some(hyge_scene::DirectionalLight {
                direction: [0.25, -1.0, 0.15],
                color: [1.0, 0.95, 0.85],
                illuminance: 70_000.0,
            }),
            fog: None,
            ambient: hyge_scene::AmbientParams {
                color: [0.15, 0.16, 0.2],
                intensity: 0.4,
            },
        },
        root_prefab_instances: (0..5)
            .map(|i| {
                PrefabInstance::new(
                    prefab_id,
                    Transform {
                        translation: [i as f32 * 1.2, 0.0, (i % 2) as f32 * 0.5],
                        ..Transform::identity()
                    },
                )
            })
            .collect(),
        post_process: PostProcessProfile {
            exposure: 1.1,
            bloom_intensity: 0.3,
            ..PostProcessProfile::default()
        },
    }
}

#[test]
fn five_prefab_instance_world_loads_and_renders() {
    let Some(_renderer_guard) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let mut renderer =
        Renderer::new_headless(&hyge_render::config::RendererConfig::default()).expect("renderer");
    let bindless = renderer.bindless_arc();
    let server = AssetServer::new(Arc::clone(&bindless));

    let mesh_id = AssetId::from(blake3::hash(b"r065-mesh"));
    let material_id = AssetId::from(blake3::hash(b"r065-material"));
    let skybox_id = AssetId::from(blake3::hash(b"r065-skybox"));

    let mesh_data = tiny_mesh();
    let mesh_asset = Arc::new(MeshAsset::new(mesh_data.clone()));
    let mesh_task = mesh_upload_task(mesh_id, Arc::clone(&bindless), &mesh_data);
    server
        .register(mesh_id, mesh_asset, mesh_task)
        .expect("mesh registers");

    let material_data = MaterialData::default();
    let material_asset = Arc::new(MaterialAsset::new(material_data.clone()));
    let material_task = material_upload_task(material_id, Arc::clone(&bindless), &material_data);
    server
        .register(material_id, material_asset, material_task)
        .expect("material registers");

    // Minimal non-uniform HDR bake that the scene's skybox id stands for.
    let hdr_bytes = {
        let mut out = Vec::new();
        out.extend_from_slice(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n");
        out.extend_from_slice(b"-Y 2 +X 4\n");
        out.extend_from_slice(&[
            255, 180, 80, 129, 80, 160, 255, 129, 128, 128, 128, 128, 64, 64, 64, 128, 255, 220,
            200, 128, 50, 50, 120, 128, 120, 200, 90, 128, 32, 32, 32, 128,
        ]);
        out
    };
    let bake = hyge_render::ibl::bake_from_rgbe_hdr_with_config(
        &hdr_bytes,
        hyge_render::ibl::BakeConfig {
            prefilter_size: 8,
            irradiance_size: 4,
            brdf_lut_size: 4,
            sample_count: 16,
        },
    )
    .expect("minimal ibl bake");

    let prefab = sample_prefab(mesh_id, material_id);
    let doc = sample_world(prefab.prefab_id, skybox_id);

    let mut world = World::new();
    let type_registry = AppTypeRegistry::default();
    *type_registry.write() = build_scene_type_registry();
    world.insert_resource(type_registry);
    world.insert_resource(server.clone());
    let mut prefabs = PrefabLibrary::default();
    prefabs.insert(prefab);
    world.insert_resource(prefabs);

    let bytes = doc.to_bytes().expect("world bytes");
    let roots = load_world_document_from_bytes(&mut world, &bytes).expect("scene loads");
    assert_eq!(roots.len(), 5, "five prefab instances must load");
    resolve_static_mesh_asset_refs(&mut world);

    assert_eq!(world.query::<&StaticMesh>().iter(&world).count(), 5);
    assert_eq!(
        world
            .get_resource::<ScenePostProcessState>()
            .expect("post state")
            .profile,
        doc.post_process
    );
    assert_eq!(
        world
            .get_resource::<SceneEnvironmentState>()
            .expect("env state")
            .environment,
        doc.env
    );
    assert_eq!(
        world
            .get_resource::<LoadedSceneState>()
            .expect("loaded state")
            .root_entities
            .len(),
        5
    );

    // Use the scene environment state to drive the renderer's IBL upload.
    let env_state = world
        .get_resource::<SceneEnvironmentState>()
        .expect("env state");
    assert_eq!(env_state.environment.skybox, Some(skybox_id));
    renderer.set_environment(&bake).expect("ibl uploads");
    assert!(
        renderer.ibl().is_some(),
        "renderer should hold IBL after scene apply"
    );

    let snapshot = hyge_scene::extract::render_extract(&mut world);
    assert_eq!(
        snapshot.draw_count(),
        1,
        "five identical prefab instances should instance"
    );
    assert_eq!(snapshot.instance_count(), 5);
    assert_eq!(snapshot.draw_commands[0].instance_count, 5);

    let instances: Vec<Instance> = pod_collect_to_vec(&snapshot.instances);
    let draw_commands: Vec<DrawCommand> = pod_collect_to_vec(&snapshot.draw_commands);
    let lights: Vec<Light> = pod_collect_to_vec(&snapshot.lights);

    let target = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("r065-scene-target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    renderer
        .render_frame(
            &target,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::Color {
                r: 0.05,
                g: 0.05,
                b: 0.07,
                a: 1.0,
            },
            &FrameData::default_looking_at_origin(),
            &instances,
            &draw_commands,
            &lights,
        )
        .expect("scene render succeeds");

    let pixels = capture_frame(renderer.device(), renderer.queue(), &target);
    let drawn_pixels = pixels
        .chunks_exact(4)
        .filter(|px| {
            let r = px[0] as f32 / 255.0;
            let g = px[1] as f32 / 255.0;
            let b = px[2] as f32 / 255.0;
            (r - 0.05).abs() > 0.01 || (g - 0.05).abs() > 0.01 || (b - 0.07).abs() > 0.01
        })
        .count();
    assert!(
        drawn_pixels > 0,
        "scene smoke frame must contain non-clear pixels"
    );
}
