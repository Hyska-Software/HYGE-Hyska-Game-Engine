//! R-063 acceptance test: "round-trip a known scene".
//!
//! Builds a representative `WorldDocument`, serializes it to msgpack, parses
//! it back, and asserts byte-for-byte equality. The second half of the test
//! drives the [`WorldLoader`] end-to-end: it resolves a prefab, instantiates
//! it into a fresh ECS world, and verifies the resulting hierarchy plus the
//! applied per-instance overrides.

use bevy_ecs::prelude::{Entity, World};
use bevy_reflect::TypeRegistry;

use hyge_asset::{Asset, LoadContext};
use hyge_ecs::AppTypeRegistry;
use hyge_scene::components::{Name, PointLight, Transform};
use hyge_scene::env::{AmbientParams, Environment, FogParams, PostProcessProfile};
use hyge_scene::prefab::{Prefab, PrefabAssets, PrefabNode, SerializedComponentOverride};
use hyge_scene::prefab_id::PrefabId;
use hyge_scene::world::{PrefabInstance, WorldDocument, WorldLoader};
use hyge_scene::world_asset::WorldAsset;

/// Builds the canonical TypeRegistry used by both the loader and the
/// serialized overrides.
fn test_registry() -> TypeRegistry {
    use hyge_scene::plugin::build_scene_type_registry;
    build_scene_type_registry()
}

/// A known prefab (root + single child) used as the resolved prefab by the
/// loader test below.
fn sample_prefab(registry: &TypeRegistry) -> Prefab {
    let mut root = PrefabNode::named("root");
    root.components.push(
        SerializedComponentOverride::new(
            "hyge_scene::components::Name",
            &Name::new("scene-root"),
            registry,
        )
        .expect("name serializes"),
    );
    let mut child = PrefabNode::named("child");
    child.persist = true;
    child.components.push(
        SerializedComponentOverride::new(
            "hyge_scene::components::Name",
            &Name::new("scene-child"),
            registry,
        )
        .expect("name serializes"),
    );
    root.children.push(child);

    Prefab::new(
        "sample",
        root,
        PrefabAssets {
            meshes: vec![blake3::hash(b"mesh").into()],
            materials: vec![blake3::hash(b"mat").into()],
            scripts: Vec::new(),
        },
    )
}

/// A known scene: ambient sky, foggy atmosphere, sun, two prefab instances
/// (the second parented to the first), one with a runtime override that
/// inserts a `PointLight` on top of the prefab's defaults.
fn sample_world_document(registry: &TypeRegistry, prefab_id: PrefabId) -> WorldDocument {
    let instance_a = PrefabInstance::new(prefab_id, Transform::identity());
    let instance_b = PrefabInstance::new(prefab_id, Transform::identity())
        .with_parent(0)
        .with_override(
            SerializedComponentOverride::new(
                "hyge_scene::components::PointLight",
                &PointLight {
                    color: [0.2, 0.4, 0.8],
                    intensity: 33.0,
                    range: 12.0,
                },
                registry,
            )
            .expect("override serializes"),
        );

    WorldDocument {
        env: Environment {
            skybox: Some(blake3::hash(b"skybox").into()),
            sun: Some(hyge_scene::components::DirectionalLight {
                direction: [-0.3, -0.8, 0.1],
                color: [1.0, 0.95, 0.85],
                illuminance: 65_000.0,
            }),
            fog: Some(FogParams {
                color: [0.6, 0.7, 0.85],
                density: 0.04,
                height_start: -2.0,
                height_falloff: 8.0,
            }),
            ambient: AmbientParams {
                color: [0.15, 0.18, 0.22],
                intensity: 0.45,
            },
        },
        root_prefab_instances: vec![instance_a, instance_b],
        post_process: PostProcessProfile {
            taa: true,
            smaa: false,
            bloom_intensity: 0.35,
            bloom_threshold: 1.2,
            bloom_levels: 4,
            exposure: 1.05,
        },
        editor_layer: None,
    }
}

#[test]
fn round_trip_known_scene() {
    let registry = test_registry();
    let prefab = sample_prefab(&registry);
    let original = sample_world_document(&registry, prefab.prefab_id);

    // Serializing the document should yield determinstic msgpack bytes.
    let bytes = original.to_bytes().expect("serialize");
    let restored = WorldDocument::from_bytes(&bytes).expect("deserialize");

    assert_eq!(original, restored);

    // The asset layer should load the same document from those bytes.
    let mut ctx = LoadContext::default();
    let loaded = WorldAsset::load(&bytes, &mut ctx).expect("asset load");
    assert_eq!(loaded, original);

    // And the asset hash must match the BLAKE3 of the serialized bytes, so
    // the asset DB stays consistent if the scene is cooked to disk.
    assert_eq!(WorldAsset::hash(&loaded), blake3::hash(&bytes));

    // Two re-serializations of the same value must be byte-identical, since
    // msgpack encoding is canonical for our structures.
    let bytes_again = original.to_bytes().expect("serialize again");
    assert_eq!(bytes, bytes_again);
}

#[test]
fn world_loader_into_ecs_with_parent_and_override() {
    let registry = test_registry();
    let prefab = sample_prefab(&registry);

    let doc = sample_world_document(&registry, prefab.prefab_id);

    // Build an ECS world wired with the scene's TypeRegistry.
    let mut world = World::new();
    let app_registry = AppTypeRegistry::default();
    *app_registry.write() = test_registry();
    world.insert_resource(app_registry);

    let resolver = move |_id: &PrefabId| Ok(prefab.clone());
    let roots = WorldLoader::load(&doc, &mut world, resolver).expect("load into ECS");

    // Two root prefab instances expected.
    assert_eq!(roots.len(), 2);

    // Each prefab contributes one root + one child, totaling four entities.
    let entity_count = world.query::<Entity>().iter(&world).count();
    assert_eq!(entity_count, 4);

    // Root #0 has no Parent; root #1 is parented to root #0.
    assert!(world
        .get::<hyge_scene::components::Parent>(roots[0])
        .is_none());
    let parent_of_1 = world
        .get::<hyge_scene::components::Parent>(roots[1])
        .expect("root #1 has Parent");
    assert_eq!(parent_of_1.0, roots[0]);

    // The second root should have the PointLight override applied.
    let light = world
        .get::<PointLight>(roots[1])
        .expect("override PointLight applied");
    assert_eq!(light.color, [0.2, 0.4, 0.8]);
    assert_eq!(light.intensity, 33.0);

    // The first root should *not* have a PointLight (no override).
    assert!(world.get::<PointLight>(roots[0]).is_none());

    // Parent's Children must list root #1.
    let children_of_a = world
        .get::<hyge_scene::components::Children>(roots[0])
        .expect("root #0 has Children");
    assert!(children_of_a.0.contains(&roots[1]));
}

#[test]
fn world_loader_rejects_out_of_range_parent() {
    let registry = test_registry();
    let prefab = sample_prefab(&registry);

    // A scene with a single instance claiming parent index 99 — clearly out
    // of range for a one-element slice.
    let doc = WorldDocument {
        env: Environment::empty(),
        root_prefab_instances: vec![
            PrefabInstance::new(prefab.prefab_id, Transform::identity()).with_parent(99)
        ],
        post_process: PostProcessProfile::default(),
        editor_layer: None,
    };

    let mut world = World::new();
    let app_registry = AppTypeRegistry::default();
    *app_registry.write() = test_registry();
    world.insert_resource(app_registry);

    let resolver = move |_id: &PrefabId| Ok(prefab.clone());
    let err = WorldLoader::load(&doc, &mut world, resolver).unwrap_err();
    assert!(matches!(
        err,
        hyge_core::result::HygeError::InvalidArgument(_)
    ));
}

#[test]
fn empty_world_round_trips() {
    let doc = WorldDocument::empty();
    let bytes = doc.to_bytes().expect("serialize");
    let restored = WorldDocument::from_bytes(&bytes).expect("deserialize");
    assert_eq!(doc, restored);
}
