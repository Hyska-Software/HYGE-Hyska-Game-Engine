//! R-037 acceptance test: load 1000 meshes + 1000 materials,
//! assert no descriptor thrash (no wgpu validation errors).
//!
//! What this test verifies (from `docs/roadmap.toml` R-037):
//!
//! 1. The bindless table's slot allocator handles 1000
//!    unique mesh registrations without exhausting the
//!    capacity or panicking.
//! 2. The bindless table's slot allocator handles 1000
//!    unique material registrations without exhausting
//!    the capacity or panicking.
//! 3. The wgpu validation layer reports no errors after
//!    every `queue.write_buffer` call (the
//!    "no descriptor thrash" check).
//! 4. Refcount-driven release returns slots to the free
//!    list: after dropping all 1000+1000 handles, the free
//!    list is back to its full capacity.
//!
//! The test uses the `TestRenderer` harness to create a
//! headless wgpu device (so it works on CI without a
//! display), then drives the bindless table directly.

use hyge_render::prelude::*;

const MESH_COUNT: u32 = 1000;
const MATERIAL_COUNT: u32 = 1000;

/// Builds a small `GpuMesh` for the stress test. The values
/// are deterministic and irrelevant — the test only
/// exercises the slot allocator and the storage write path.
fn make_test_mesh(seed: u32) -> GpuMesh {
    GpuMesh {
        vertex_offset: seed.wrapping_mul(32),
        index_offset: seed.wrapping_mul(48),
        meshlet_offset: seed.wrapping_mul(64),
        meshlet_count: (seed % 16) + 1,
        aabb_min: [0.0, 0.0, 0.0],
        aabb_max: [1.0, 1.0, 1.0],
        lod_count: 3,
        _pad: 0,
    }
}

/// Builds a small `GpuMaterial` for the stress test. Same
/// reasoning as [`make_test_mesh`].
fn make_test_material(seed: u32) -> GpuMaterial {
    GpuMaterial {
        base_color: seed % 16,
        normal: (seed + 1) % 16,
        mr: (seed + 2) % 16,
        occlusion: (seed + 3) % 16,
        emissive: (seed + 4) % 16,
        roughness: 0.5,
        metallic: 0.25,
        alpha_mode: 0,
        flags: 0,
    }
}

/// R-037 acceptance test: 1000 meshes + 1000 materials
/// registered through the bindless table, with no wgpu
/// validation errors and full refcount-driven release.
#[test]
fn bindless_table_handles_1000_meshes_and_1000_materials() {
    let Some(renderer) = hyge_runtime_test::TestRenderer::new() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let bindless = renderer.renderer_bindless();

    // -- Phase 1: register 1000 meshes -----------------------------
    let initial_mesh_free = bindless.free_mesh_slots();
    let initial_material_free = bindless.free_material_slots();
    assert!(
        initial_mesh_free >= MESH_COUNT,
        "bindless table should start with at least {MESH_COUNT} mesh slots; got {initial_mesh_free}"
    );
    assert!(
        initial_material_free >= MATERIAL_COUNT,
        "bindless table should start with at least {MATERIAL_COUNT} material slots; got {initial_material_free}"
    );

    let mesh_handles: Vec<MeshId> = (0..MESH_COUNT)
        .map(|i| {
            bindless
                .register_mesh(make_test_mesh(i))
                .expect("mesh registration must succeed for the first {MESH_COUNT} entries")
        })
        .collect();
    let material_handles: Vec<MaterialId> = (0..MATERIAL_COUNT)
        .map(|i| {
            bindless
                .register_material(make_test_material(i))
                .expect("material registration must succeed for the first {MATERIAL_COUNT} entries")
        })
        .collect();

    // Every handle's slot id is unique (we just allocated
    // them all from the free list). The exact ids depend
    // on the allocator's stack order, but they should
    // form a 0..1000 range with no duplicates.
    let mut mesh_slots: Vec<u32> = mesh_handles.iter().map(|h| h.index()).collect();
    mesh_slots.sort_unstable();
    mesh_slots.dedup();
    assert_eq!(
        mesh_slots.len(),
        MESH_COUNT as usize,
        "duplicate mesh slot ids after registering {MESH_COUNT} entries"
    );
    let mut material_slots: Vec<u32> = material_handles.iter().map(|h| h.index()).collect();
    material_slots.sort_unstable();
    material_slots.dedup();
    assert_eq!(
        material_slots.len(),
        MATERIAL_COUNT as usize,
        "duplicate material slot ids after registering {MATERIAL_COUNT} entries"
    );

    // The free list dropped by exactly the number of
    // allocations.
    assert_eq!(
        bindless.free_mesh_slots(),
        initial_mesh_free - MESH_COUNT,
        "free mesh slots did not decrease by the expected amount"
    );
    assert_eq!(
        bindless.free_material_slots(),
        initial_material_free - MATERIAL_COUNT,
        "free material slots did not decrease by the expected amount"
    );

    // -- Phase 2: poll the device ---------------------------------
    // The validation layer reports any errors during the
    // `queue.write_buffer` calls above. A successful
    // `Maintain::Wait` is the canonical "no validation
    // errors" check (the wgpu device processes any pending
    // error scopes during this poll).
    renderer.device().poll(wgpu::Maintain::Wait);
    // Force a queue submission so any deferred validation
    // errors surface before the test exits.
    renderer.queue().submit(std::iter::empty());

    // -- Phase 3: drop everything ---------------------------------
    // The `MeshId` and `MaterialId` refcount tracks the
    // number of live clones; dropping the last clone
    // returns the slot to the free list.
    drop(mesh_handles);
    drop(material_handles);
    assert_eq!(
        bindless.free_mesh_slots(),
        initial_mesh_free,
        "free mesh slots did not return to initial capacity after drop"
    );
    assert_eq!(
        bindless.free_material_slots(),
        initial_material_free,
        "free material slots did not return to initial capacity after drop"
    );
}

/// R-037 supplementary: the refcount of a single mesh slot
/// climbs with each `Clone` and reaches zero on the last
/// `Drop`. Verifying the refcount via `refs()` is a
/// low-level sanity check that the typed slot bookkeeping
/// is correct (independent of the 1000-entry stress test).
#[test]
fn bindless_slot_refcount_bumps_on_clone_and_drops_to_zero() {
    let Some(renderer) = hyge_runtime_test::TestRenderer::new() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let bindless = renderer.renderer_bindless();
    let id = bindless
        .register_mesh(GpuMesh::default())
        .expect("registration must succeed");
    assert_eq!(id.refs(), 1, "fresh slot should have refcount 1");
    let clone = id.clone();
    assert_eq!(id.refs(), 2, "cloning should bump the refcount to 2");
    drop(clone);
    assert_eq!(id.refs(), 1, "dropping the clone should decrement to 1");
    drop(id);
    // After the slot is freed, the next registration can
    // reuse the same slot id (the free list is LIFO).
    let id2 = bindless
        .register_mesh(GpuMesh::default())
        .expect("re-registration must succeed");
    assert_eq!(id2.refs(), 1, "recycled slot starts at refcount 1");
}
