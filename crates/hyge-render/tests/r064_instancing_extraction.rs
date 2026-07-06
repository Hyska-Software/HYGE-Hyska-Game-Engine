//! R-064 Instancing extraction — `StaticMesh` path.
//!
//! End-to-end integration test: builds a headless renderer, creates an
//! `AssetServer` backed by the renderer's `BindlessTable`, registers a mesh
//! and a material, spawns 1000 entities all sharing the same `StaticMesh`
//! handles, runs `render_extract`, and verifies that the resulting
//! `FrameSnapshot` contains exactly **one** `DrawCommand` with
//! `instance_count == 1000`.
//!
//! This is the R-064 acceptance #4 test:
//! "test: 1000 entities with same mesh+material yield 1 DrawCommand with
//! instance_count=1000".
//!
//! Additional coverage:
//! - Acceptance #1 ("Query With<StaticMesh> iterates entities") is exercised
//!   by the fact that the 1000 entities are all found and grouped.
//! - Acceptance #2 ("Handle resolution -> mesh_id, material_id from
//!   BindlessTable") is exercised because each `StaticMesh` carries a typed
//!   `Handle<MeshAsset>` / `Handle<MaterialAsset>` that is resolved through
//!   `AssetServer::bindless_for` to the raw bindless slot index.
//! - Acceptance #3 ("Group by (mesh_id, material_id), sort by material_id,
//!   emit one DrawCommand per group") is checked by the multi-group test
//!   below.

use std::sync::Arc;

use bevy_ecs::prelude::*;
use hyge_asset::importer::material::MaterialData;
use hyge_asset::importer::mesh::{MeshData, Vertex};
use hyge_asset::prelude::{
    material_upload_task, mesh_upload_task, Asset as AssetTrait, AssetId, AssetServer, Handle,
    MaterialAsset, MeshAsset,
};
use hyge_runtime_test::TestRenderer;
use hyge_scene::extract::render_extract;
use hyge_scene::prelude::{StaticMesh, WorldTransform};

/// Builds a minimal `MeshData` triangle suitable for registering in the
/// bindless table without exercising the full glTF importer.
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

/// R-064 acceptance #4: 1000 entities with the same mesh + material yields
/// exactly one `DrawCommand` with `instance_count == 1000`.
#[test]
fn thousand_entities_collapse_into_one_draw_command() {
    let Some(test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let bindless: Arc<hyge_render::prelude::BindlessTable> = test_renderer.renderer_bindless_arc();
    let server = AssetServer::new(Arc::clone(&bindless));

    // Register a mesh asset.
    let mesh_data = tiny_mesh();
    let mesh_id = AssetId::from(MeshAsset::hash(&mesh_data));
    let mesh_handle: Handle<MeshAsset> = server.load(mesh_id);
    let mesh_asset = Arc::new(MeshAsset::new(mesh_data.clone()));
    let mesh_task = mesh_upload_task(mesh_id, Arc::clone(&bindless), &mesh_data);
    server
        .register(mesh_id, mesh_asset, mesh_task)
        .expect("mesh register must succeed");

    // Register a material asset.
    let material_data = MaterialData::default();
    let material_id = AssetId::from(MaterialAsset::hash(&material_data));
    let material_handle: Handle<MaterialAsset> = server.load(material_id);
    let material_asset = Arc::new(MaterialAsset::new(material_data.clone()));
    let material_task = material_upload_task(material_id, Arc::clone(&bindless), &material_data);
    server
        .register(material_id, material_asset, material_task)
        .expect("material register must succeed");

    // Sanity: the server now has both assets loaded and resolvable.
    assert!(
        server.bindless_for(mesh_id).is_some(),
        "mesh bindless resolution must succeed after register"
    );
    assert!(
        server.bindless_for(material_id).is_some(),
        "material bindless resolution must succeed after register"
    );

    // Build a world with 1000 entities all sharing the same StaticMesh.
    let mut world = World::new();
    world.insert_resource(server.clone());
    for i in 0..1000_u32 {
        let x = (i % 32) as f32;
        let z = (i / 32) as f32;
        world.spawn((
            StaticMesh::new(mesh_handle, material_handle),
            WorldTransform::from_translation(x, 0.0, z),
        ));
    }

    // Run the extraction.
    let snapshot = render_extract(&mut world);

    // Exactly one (mesh_id, material_id) pair -> one DrawCommand with
    // instance_count == 1000.
    assert_eq!(
        snapshot.draw_count(),
        1,
        "1000 entities sharing one (mesh, material) pair must collapse to 1 DrawCommand \
         (got {} draws)",
        snapshot.draw_count(),
    );
    assert_eq!(
        snapshot.instance_count(),
        1000,
        "all 1000 instances must be present in the instance buffer"
    );
    let dc = &snapshot.draw_commands[0];
    assert_eq!(dc.instance_count, 1000);
    assert_eq!(dc.first_instance, 0);
    // Verify the resolved bindless slot indices match what the server assigned.
    let resolved_mesh = server
        .bindless_for(mesh_id)
        .expect("mesh resolved")
        .slot_index();
    let resolved_material = server
        .bindless_for(material_id)
        .expect("material resolved")
        .slot_index();
    assert_eq!(dc.mesh_id, resolved_mesh);
    assert_eq!(dc.material_id, resolved_material);

    // Every instance must carry the same resolved ids.
    for inst in &snapshot.instances {
        assert_eq!(inst.mesh_id, resolved_mesh);
        assert_eq!(inst.material_id, resolved_material);
    }
}

/// R-064 acceptance #3: distinct (mesh_id, material_id) pairs produce one
/// `DrawCommand` per group, sorted by `material_id`.
///
/// We use a single renderer / server and register two distinct meshes and
/// two distinct materials. We then spawn four groups arranged in an order
/// that is NOT the material-sorted order, and verify the emitted
/// `DrawCommand`s are sorted by `material_id` ascending.
#[test]
fn distinct_groups_sorted_by_material_id() {
    let Some(test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let bindless = test_renderer.renderer_bindless_arc();
    let server = AssetServer::new(Arc::clone(&bindless));

    // Helper that registers an asset and returns its slot index.
    fn register_mesh(
        server: &AssetServer,
        bindless: &Arc<hyge_render::prelude::BindlessTable>,
        tag: &[u8],
    ) -> Handle<MeshAsset> {
        let data = tiny_mesh();
        let id = AssetId::from(blake3::hash(tag));
        let asset = Arc::new(MeshAsset::new(data.clone()));
        let task = mesh_upload_task(id, Arc::clone(bindless), &data);
        server.register(id, asset, task).expect("mesh register");
        server.load(id)
    }
    fn register_material(
        server: &AssetServer,
        bindless: &Arc<hyge_render::prelude::BindlessTable>,
        tag: &[u8],
    ) -> Handle<MaterialAsset> {
        let data = MaterialData::default();
        let id = AssetId::from(blake3::hash(tag));
        let asset = Arc::new(MaterialAsset::new(data.clone()));
        let task = material_upload_task(id, Arc::clone(bindless), &data);
        server.register(id, asset, task).expect("material register");
        server.load(id)
    }

    let mesh_a = register_mesh(&server, &bindless, b"r-064-mesh-a");
    let mesh_b = register_mesh(&server, &bindless, b"r-064-mesh-b");
    let mat_x = register_material(&server, &bindless, b"r-064-mat-x");
    let mat_y = register_material(&server, &bindless, b"r-064-mat-y");

    let mat_x_idx = server
        .bindless_for(mat_x.id())
        .expect("mat_x resolved")
        .slot_index();
    let mat_y_idx = server
        .bindless_for(mat_y.id())
        .expect("mat_y resolved")
        .slot_index();

    // Build a world. We spawn groups in an order that does NOT match the
    // material-sorted order; the extraction must still emit them sorted by
    // material_id.
    //
    // Group layout:
    //   (mesh_b, mat_y) x 3     <- highest material_id if y > x
    //   (mesh_a, mat_x) x 5     <- lowest material_id
    //   (mesh_b, mat_x) x 2     <- same material as group above
    //   (mesh_a, mat_y) x 4     <- same material as first group
    //
    // After grouping by (mesh_id, material_id) there are 4 distinct groups.
    // Sorted by material_id:
    //   (mesh_a, mat_x)  x 5     <- mat_x group 1
    //   (mesh_b, mat_x)  x 2     <- mat_x group 2
    //   (mesh_a, mat_y)  x 4     <- mat_y group 1
    //   (mesh_b, mat_y)  x 3     <- mat_y group 2
    let mut world = World::new();
    world.insert_resource(server.clone());
    for _ in 0..3 {
        world.spawn((StaticMesh::new(mesh_b, mat_y), WorldTransform::identity()));
    }
    for _ in 0..5 {
        world.spawn((StaticMesh::new(mesh_a, mat_x), WorldTransform::identity()));
    }
    for _ in 0..2 {
        world.spawn((StaticMesh::new(mesh_b, mat_x), WorldTransform::identity()));
    }
    for _ in 0..4 {
        world.spawn((StaticMesh::new(mesh_a, mat_y), WorldTransform::identity()));
    }

    let snapshot = render_extract(&mut world);
    assert_eq!(snapshot.draw_count(), 4);
    assert_eq!(snapshot.instance_count(), 14);

    // The draw commands must be sorted by material_id ascending. Within the
    // same material_id, the order is mesh_id ascending (the BTreeMap key is
    // (material_id << 32) | mesh_id).
    let material_ids: Vec<u32> = snapshot
        .draw_commands
        .iter()
        .map(|dc| dc.material_id)
        .collect();
    let mut sorted = material_ids.clone();
    sorted.sort_unstable();
    assert_eq!(
        material_ids, sorted,
        "draw commands must be sorted by material_id ascending"
    );

    // Spot-check the group sizes.
    let counts: Vec<(u32, u32, u32)> = snapshot
        .draw_commands
        .iter()
        .map(|dc| (dc.mesh_id, dc.material_id, dc.instance_count))
        .collect();
    // Resolve mesh ids for assertion.
    let mesh_a_idx = server
        .bindless_for(mesh_a.id())
        .expect("mesh_a resolved")
        .slot_index();
    let mesh_b_idx = server
        .bindless_for(mesh_b.id())
        .expect("mesh_b resolved")
        .slot_index();

    // Expected sorted order: mat_x groups first, then mat_y groups.
    let mat_x_groups: Vec<&(u32, u32, u32)> = counts.iter().filter(|c| c.1 == mat_x_idx).collect();
    let mat_y_groups: Vec<&(u32, u32, u32)> = counts.iter().filter(|c| c.1 == mat_y_idx).collect();
    assert_eq!(mat_x_groups.len(), 2, "two distinct (mesh, mat_x) groups");
    assert_eq!(mat_y_groups.len(), 2, "two distinct (mesh, mat_y) groups");

    // Within mat_x: sorted by mesh_id ascending -> (mesh_a, mat_x) then (mesh_b, mat_x).
    assert_eq!(mat_x_groups[0].0, mesh_a_idx);
    assert_eq!(mat_x_groups[0].2, 5);
    assert_eq!(mat_x_groups[1].0, mesh_b_idx);
    assert_eq!(mat_x_groups[1].2, 2);

    // Within mat_y: (mesh_a, mat_y) then (mesh_b, mat_y).
    assert_eq!(mat_y_groups[0].0, mesh_a_idx);
    assert_eq!(mat_y_groups[0].2, 4);
    assert_eq!(mat_y_groups[1].0, mesh_b_idx);
    assert_eq!(mat_y_groups[1].2, 3);

    // first_instance must be contiguous starting at 0.
    assert_eq!(snapshot.draw_commands[0].first_instance, 0);
    for i in 1..snapshot.draw_commands.len() {
        let prev = &snapshot.draw_commands[i - 1];
        let curr = &snapshot.draw_commands[i];
        assert_eq!(
            curr.first_instance,
            prev.first_instance + prev.instance_count,
            "first_instance must be contiguous"
        );
    }
}

/// R-064 acceptance #2: handle resolution path. Entities whose `StaticMesh`
/// handles have not been registered are skipped (no draw command emitted);
/// once the asset is registered, the same world produces the expected draw.
#[test]
fn unresolved_handles_skipped_then_filled_after_register() {
    let Some(test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let bindless = test_renderer.renderer_bindless_arc();
    let server = AssetServer::new(Arc::clone(&bindless));

    let mesh_data = tiny_mesh();
    let mesh_id = AssetId::from(blake3::hash(b"r-064-late-mesh"));
    let material_data = MaterialData::default();
    let material_id = AssetId::from(blake3::hash(b"r-064-late-mat"));
    let mesh_handle: Handle<MeshAsset> = server.load(mesh_id);
    let material_handle: Handle<MaterialAsset> = server.load(material_id);

    let mut world = World::new();
    world.insert_resource(server.clone());
    world.spawn((
        StaticMesh::new(mesh_handle, material_handle),
        WorldTransform::identity(),
    ));

    // Before register: nothing resolves.
    let snapshot_before = render_extract(&mut world);
    assert_eq!(
        snapshot_before.draw_count(),
        0,
        "unresolved handles must produce no draws"
    );

    // Register both assets.
    let mesh_asset = Arc::new(MeshAsset::new(mesh_data.clone()));
    let mesh_task = mesh_upload_task(mesh_id, Arc::clone(&bindless), &mesh_data);
    server
        .register(mesh_id, mesh_asset, mesh_task)
        .expect("mesh register");
    let material_asset = Arc::new(MaterialAsset::new(material_data.clone()));
    let material_task = material_upload_task(material_id, Arc::clone(&bindless), &material_data);
    server
        .register(material_id, material_asset, material_task)
        .expect("material register");

    // After register: one draw command.
    let snapshot_after = render_extract(&mut world);
    assert_eq!(snapshot_after.draw_count(), 1);
    assert_eq!(snapshot_after.instance_count(), 1);
    assert_eq!(snapshot_after.draw_commands[0].instance_count, 1);
}

/// Regression: the legacy `MeshHandle` / `MaterialHandle` path must still
/// produce draw commands when no `AssetServer` is in the world. This guards
/// the backward-compatibility contract: scenes authored against R-043 keep
/// working without the asset server.
#[test]
fn legacy_path_works_without_asset_server() {
    let mut world = World::new();
    use hyge_scene::prelude::{MaterialHandle, MeshHandle};

    for _ in 0..10 {
        world.spawn((
            MeshHandle(7),
            MaterialHandle(11),
            WorldTransform::identity(),
        ));
    }
    let snapshot = render_extract(&mut world);
    assert_eq!(snapshot.draw_count(), 1);
    assert_eq!(snapshot.instance_count(), 10);
    assert_eq!(snapshot.draw_commands[0].instance_count, 10);
    assert_eq!(snapshot.draw_commands[0].mesh_id, 7);
    assert_eq!(snapshot.draw_commands[0].material_id, 11);
}

/// The legacy path and the `StaticMesh` path emit into the same snapshot
/// with contiguous `first_instance` offsets. The legacy groups are emitted
/// first, then the `StaticMesh` groups.
#[test]
fn legacy_and_static_paths_share_instance_buffer() {
    let Some(test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };

    let bindless = test_renderer.renderer_bindless_arc();
    let server = AssetServer::new(Arc::clone(&bindless));

    let mesh_data = tiny_mesh();
    let mesh_id = AssetId::from(blake3::hash(b"r-064-combo-mesh"));
    let material_data = MaterialData::default();
    let material_id = AssetId::from(blake3::hash(b"r-064-combo-mat"));
    let mesh_handle: Handle<MeshAsset> = server.load(mesh_id);
    let material_handle: Handle<MaterialAsset> = server.load(material_id);
    let mesh_asset = Arc::new(MeshAsset::new(mesh_data.clone()));
    let mesh_task = mesh_upload_task(mesh_id, Arc::clone(&bindless), &mesh_data);
    server
        .register(mesh_id, mesh_asset, mesh_task)
        .expect("mesh register");
    let material_asset = Arc::new(MaterialAsset::new(material_data.clone()));
    let material_task = material_upload_task(material_id, Arc::clone(&bindless), &material_data);
    server
        .register(material_id, material_asset, material_task)
        .expect("material register");

    let mut world = World::new();
    world.insert_resource(server.clone());
    // Legacy: 3 entities with (mesh=42, material=99).
    use hyge_scene::prelude::{MaterialHandle, MeshHandle};
    for _ in 0..3 {
        world.spawn((
            MeshHandle(42),
            MaterialHandle(99),
            WorldTransform::identity(),
        ));
    }
    // StaticMesh: 5 entities sharing one (mesh, material) pair.
    for _ in 0..5 {
        world.spawn((
            StaticMesh::new(mesh_handle, material_handle),
            WorldTransform::identity(),
        ));
    }
    let snapshot = render_extract(&mut world);
    // Two distinct groups: one from the legacy path, one from StaticMesh.
    assert_eq!(snapshot.draw_count(), 2);
    assert_eq!(snapshot.instance_count(), 8);
    // first_instance must be contiguous: legacy group at 0, static group at 3.
    assert_eq!(snapshot.draw_commands[0].first_instance, 0);
    assert_eq!(snapshot.draw_commands[0].instance_count, 3);
    assert_eq!(snapshot.draw_commands[1].first_instance, 3);
    assert_eq!(snapshot.draw_commands[1].instance_count, 5);
}

/// Smoke check: a freshly-constructed headless `Renderer` produces a
/// non-`None` bindless table; `AssetServer::new` accepts it without
/// panicking. This guards the constructor contract used by the tests above.
#[test]
fn asset_server_constructs_from_renderer_bindless() {
    let Some(test_renderer) = TestRenderer::new() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };
    let bindless = test_renderer.renderer_bindless_arc();
    let server = AssetServer::new(bindless);
    assert_eq!(server.loaded_count(), 0);
    let _ = format!("{server:?}");
}
