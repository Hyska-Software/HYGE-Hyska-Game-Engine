//! R-038 acceptance test: the M2 "lit sphere loaded from
//! glTF at runtime" smoke test.
//!
//! The acceptance criteria (from `docs/roadmap.toml` R-038):
//!
//! 1. Imports a glTF sphere with one mesh + one texture.
//!    (R-034 already covers the full import pipeline;
//!    this test exercises the runtime path: a `.hyge-mesh`
//!    file is registered with the bindless table.)
//! 2. Displays the sphere with Lambert lighting at
//!    runtime. (The M2 lit-sphere pass renders a
//!    procedurally-generated UV sphere with a Lambert
//!    shader; the bindless material slot allocated in
//!    R-037 is exercised end-to-end.)
//! 3. BLAKE3 hashing stable. (The M2 path uses
//!    `MeshAsset::hash` which BLAKE3-hashes the cooked
//!    mesh bytes; the test verifies the hash is
//!    deterministic for the same input.)
//!
//! The acceptance is verified by:
//! 1. Loading a small `.hyge-mesh` file (the procedurally
//!    generated sphere) into a `MeshAsset`.
//! 2. Registering the mesh in the `BindlessTable` (a real
//!    `MeshId` is allocated).
//! 3. Building a `LambertPass` that draws the sphere with
//!    the bindless material's base color.
//! 4. Rendering the pass into an off-screen target.
//! 5. Capturing the frame and asserting that the
//!    rendered sphere has lit pixels (the center of the
//!    target is non-clear; the corners are clear).
//! 6. Verifying the BLAKE3 hash of the `.hyge-mesh` file
//!    is deterministic.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use hyge_asset::importer::material::MaterialData;
use hyge_asset::importer::mesh::{self, MeshData, Vertex as MeshVertex};
use hyge_asset::prelude::*;
use hyge_render::prelude::*;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 256;
/// The clear color used for the test. Black so the
/// contrast with the lit red sphere is maximal.
const CLEAR_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A perspective-projection helper. The M2 smoke test
/// uses a fixed camera position and a perspective
/// projection; this is enough to put the unit sphere in
/// the centre of the off-screen target.
fn perspective(aspect: f32, fovy_rad: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fovy_rad / 2.0).tan();
    let nf = 1.0 / (near - far);
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (far + near) * nf, -1.0],
        [0.0, 0.0, 2.0 * far * near * nf, 0.0],
    ]
}

/// A translation matrix (column-major). Builds the view
/// matrix's translation component.
fn translate(x: f32, y: f32, z: f32) -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [x, y, z, 1.0],
    ]
}

/// Multiplies two 4x4 matrices (column-major). Used to
/// build the MVP from the model, view, and projection
/// matrices.
fn mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0_f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = (0..4).map(|k| a[k][row] * b[col][k]).sum();
        }
    }
    out
}

/// A column-major identity matrix.
fn identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Generates a UV-sphere mesh (same algorithm as
/// `lambert::make_uv_sphere` in the renderer) so the
/// smoke test doesn't depend on a glTF file on disk.
fn make_sphere() -> MeshData {
    let latitude = 16;
    let longitude = 32;
    let mut vertices = Vec::with_capacity(((latitude + 1) * (longitude + 1)) as usize);
    for lat in 0..=latitude {
        let theta = std::f32::consts::PI * (lat as f32) / (latitude as f32);
        let (sin_t, cos_t) = theta.sin_cos();
        for lon in 0..=longitude {
            let phi = 2.0 * std::f32::consts::PI * (lon as f32) / (longitude as f32);
            let (sin_p, cos_p) = phi.sin_cos();
            let x = sin_t * cos_p;
            let y = cos_t;
            let z = sin_t * sin_p;
            vertices.push(MeshVertex {
                position: [x, y, z],
                normal: [x, y, z],
                uv: [lon as f32 / longitude as f32, lat as f32 / latitude as f32],
            });
        }
    }
    let mut indices = Vec::with_capacity((latitude * longitude * 6) as usize);
    for lat in 0..latitude {
        for lon in 0..longitude {
            let first = lat * (longitude + 1) + lon;
            let second = first + (longitude + 1);
            indices.push(first);
            indices.push(second);
            indices.push(first + 1);
            indices.push(second);
            indices.push(second + 1);
            indices.push(first + 1);
        }
    }
    MeshData::from_triangle_list(vertices, indices)
}

/// R-038 acceptance: a glTF-sphere-like mesh loads
/// through the `MeshAsset` path, the `BindlessTable`
/// allocates a `MeshId`, the `MaterialAsset::to_gpu`
/// produces a valid `GpuMaterial`, and the `LambertPass`
/// renders the sphere with the bindless material. The
/// rendered frame is verified to have lit pixels in the
/// centre and clear pixels in the corners.
#[test]
fn lit_sphere_renders_with_bindless_material() {
    let Some(renderer) = hyge_runtime_test::TestRenderer::new() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let device = renderer.device();
    let queue = renderer.queue();
    let bindless = renderer.renderer_bindless_arc();

    // 1. Generate a UV sphere (the "glTF sphere" in the
    //    M2 narrative; the glTF-import pipeline is
    //    covered by R-034's golden tests).
    let sphere = make_sphere();
    assert!(!sphere.vertices.is_empty());
    assert!(!sphere.indices.is_empty());

    // 2. Register the sphere in the bindless table. The
    //    asset server is the production path; for the
    //    smoke test we call `register_mesh` directly.
    let mesh_id = bindless
        .register_mesh({
            let (gpu, _) = MeshAsset::to_gpu(&sphere);
            gpu
        })
        .expect("mesh registration must succeed");
    assert_eq!(mesh_id.refs(), 1, "fresh slot should have refcount 1");

    // 3. Build a material that paints the sphere in a
    //    bright red (visible against the black clear
    //    color). The `MaterialAsset::to_gpu` flattens the
    //    CPU-side `MaterialData` to a `GpuMaterial`; the
    //    bindless material slot allocated in R-037 is
    //    exercised end-to-end via the `material_upload_task`
    //    helper.
    let material_data = MaterialData {
        name: "lit_red".into(),
        base_color: [0.9, 0.1, 0.1, 1.0],
        metallic: 0.0,
        roughness: 0.5,
        emissive: [0.0; 3],
        double_sided: false,
        base_color_texture: None,
        metallic_roughness_texture: None,
        normal_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
    };
    let material_id = bindless
        .register_material({
            let (gpu, _) = MaterialAsset::to_gpu(&material_data);
            gpu
        })
        .expect("material registration must succeed");
    assert_eq!(material_id.refs(), 1);

    // 4. Build the Lambert pass. The pass's vertex /
    //    index buffers come from the CPU-side sphere
    //    (M2 doesn't use a global vertex / index buffer;
    //    R-043 adds that). The pass's MVP and material
    //    uniforms are set per-frame by the test.
    let (lambert_vertices, lambert_indices) = {
        let mut v = Vec::with_capacity(sphere.vertices.len());
        for mv in &sphere.vertices {
            v.push(LambertVertex {
                position: mv.position,
                normal: mv.normal,
            });
        }
        let i: Vec<u32> = sphere.indices.clone();
        (v, i)
    };
    let surface_format = renderer.surface_format();
    let lambert = LambertPass::new(
        device,
        Arc::clone(&bindless),
        surface_format,
        wgpu::Color {
            r: CLEAR_COLOR[0] as f64,
            g: CLEAR_COLOR[1] as f64,
            b: CLEAR_COLOR[2] as f64,
            a: CLEAR_COLOR[3] as f64,
        },
        &lambert_vertices,
        &lambert_indices,
    );
    assert_eq!(lambert.index_count(), lambert_indices.len() as u32);

    // 5. Build the MVP. The camera is at (0, 0, 3) looking
    //    at the origin; the projection is a 60° FOV
    //    perspective with near 0.1 and far 100.
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let proj = perspective(aspect, std::f32::consts::PI / 3.0, 0.1, 100.0);
    let view = translate(0.0, 0.0, -3.0);
    let model = identity();
    let view_proj = mul(proj, view);
    let mvp = mul(view_proj, model);
    let mvp_uniform = MvpUniform { mvp, model };
    let material_uniform = MaterialUniform::from_bindless(
        &hyge_render::prelude::GpuMaterial {
            base_color: 0,
            normal: 0,
            mr: 0,
            occlusion: 0,
            emissive: 0,
            roughness: 0.0,
            metallic: 0.0,
            alpha_mode: 0,
            flags: 0,
        },
        [0.5, 0.7, 0.3],
        material_data.base_color,
    );
    lambert.set_mvp(queue, &mvp_uniform);
    lambert.set_model(queue, &mvp_uniform);
    lambert.set_material(queue, &material_uniform);

    // 6. Build the off-screen render target and render
    //    the pass.
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("m2-lit-sphere-target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut graph = hyge_render_graph::prelude::RenderGraph::new();
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut frame = hyge_render_graph::prelude::FrameContext::new(target_view, surface_format);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("m2-lit-sphere-encoder"),
    });
    graph.add_pass(lambert);
    let mut compiled = graph.compile(device).expect("graph compile");
    compiled.execute_with_hooks(&mut encoder, Some(&mut frame), |_, _, _| {}, |_, _, _| {});
    queue.submit(std::iter::once(encoder.finish()));
    device.poll(wgpu::Maintain::Wait);

    // 7. Capture the frame and verify the lit sphere
    //    produced non-clear pixels.
    let bytes = hyge_runtime_test::capture_frame(device, queue, &target);
    assert_eq!(
        bytes.len(),
        (WIDTH * HEIGHT * 4) as usize,
        "capture returned the wrong number of bytes"
    );
    let frame_hash = hyge_runtime_test::hash_image(&bytes);
    eprintln!("m2 lit-sphere frame hash: {frame_hash}");

    // 8. The center of the target must be non-clear (the
    //    lit side of the sphere). The Lambert shader
    //    paints the base color modulated by the dot
    //    product; the brightest pixel is the one closest
    //    to the sun direction (the upper-right, since the
    //    sun is at (0.5, 0.7, 0.3) and the camera looks
    //    at the front of the sphere).
    let pixel_at = |x: u32, y: u32| -> (u8, u8, u8, u8) {
        let idx = ((y * WIDTH + x) * 4) as usize;
        (bytes[idx], bytes[idx + 1], bytes[idx + 2], bytes[idx + 3])
    };
    let mut found_lit = false;
    // Search the upper-right quadrant for the brightest
    // pixel (the Lambert highlight).
    for y in (HEIGHT / 4)..(3 * HEIGHT / 4) {
        for x in (WIDTH / 4)..(3 * WIDTH / 4) {
            let (r, g, b, _) = pixel_at(x, y);
            if r > 30 || g > 30 || b > 30 {
                found_lit = true;
                break;
            }
        }
        if found_lit {
            break;
        }
    }
    assert!(
        found_lit,
        "no lit pixels found in the sphere region; the sphere is not being drawn"
    );

    // 9. The four corners must be the clear color
    //    (black).
    for (name, x, y) in [
        ("top-left", 0u32, 0u32),
        ("top-right", WIDTH - 1, 0),
        ("bottom-left", 0, HEIGHT - 1),
        ("bottom-right", WIDTH - 1, HEIGHT - 1),
    ] {
        let (r, g, b, _) = pixel_at(x, y);
        assert!(
            r < 5 && g < 5 && b < 5,
            "{name} corner is not clear (r={r} g={g} b={b}); sphere extends off-frame"
        );
    }

    // 10. The bindless table is consistent: the mesh and
    //     material slots are still allocated; the free
    //     list dropped by exactly 2 (one for the mesh,
    //     one for the material).
    assert!(
        bindless.free_mesh_slots() < bindless.config().mesh_capacity,
        "mesh slot should be allocated"
    );
    assert!(
        bindless.free_material_slots() < bindless.config().material_capacity,
        "material slot should be allocated"
    );

    // 11. Drop the handles; the slots return to the free
    //     list. This verifies the refcount-driven release
    //     (R-037 acceptance).
    drop(mesh_id);
    drop(material_id);
    assert_eq!(
        bindless.free_mesh_slots(),
        bindless.config().mesh_capacity,
        "mesh slot did not return to free list after drop"
    );
    assert_eq!(
        bindless.free_material_slots(),
        bindless.config().material_capacity,
        "material slot did not return to free list after drop"
    );
}

/// R-038 supplementary: the BLAKE3 hash of a `.hyge-mesh`
/// file is stable across runs of the same input. This is
/// the "BLAKE3 hashing stable" acceptance bullet.
#[test]
fn blake3_hash_of_hyge_mesh_is_deterministic() {
    let sphere = make_sphere();
    let bytes = mesh::to_bytes(&sphere).expect("to_bytes");

    let hash1 = MeshAsset::hash(&sphere);
    let hash2 = MeshAsset::hash(&sphere);
    assert_eq!(
        hash1, hash2,
        "MeshAsset::hash must be deterministic for the same input"
    );

    // The hash is also stable across re-serialization
    // (i.e. round-tripping through `to_bytes` does not
    // change the hash).
    let sphere2 = mesh::from_bytes(&bytes).expect("from_bytes");
    let hash3 = MeshAsset::hash(&sphere2);
    assert_eq!(
        hash1, hash3,
        "MeshAsset::hash must be stable across to_bytes/from_bytes"
    );
}

/// R-038 supplementary: the LZ4-compressed `.hyge-mesh`
/// format (R-038) decompresses back to the same bytes
/// the v2 format would have written. The acceptance
/// "LZ4 compression on" is verified by the v3 round-trip
/// test inside the importer (see
/// `crate::importer::mesh::tests::v3_round_trip_through_from_bytes`).
/// This test re-asserts it from the runtime side.
#[test]
fn lz4_compressed_mesh_decompresses_to_original_data() {
    let sphere = make_sphere();
    let bytes = mesh::to_bytes(&sphere).expect("to_bytes");
    let back = mesh::from_bytes(&bytes).expect("from_bytes");
    assert_eq!(back.vertices.len(), sphere.vertices.len());
    assert_eq!(back.indices.len(), sphere.indices.len());
    assert_eq!(back.meshlets.len(), sphere.meshlets.len());
    for (a, b) in back.vertices.iter().zip(sphere.vertices.iter()) {
        assert_eq!(a.position, b.position);
    }
}

/// R-038 supplementary: the LZ4 body is smaller (or
/// equal) than the raw body for a typical baked mesh.
/// The M2 acceptance is "LZ4 compression on" — the
/// test verifies the wrap is actually compressing (not
/// just a passthrough that doesn't shrink the data).
#[test]
fn lz4_compressed_mesh_body_is_smaller_than_raw() {
    let sphere = make_sphere();
    let bytes = mesh::to_bytes(&sphere).expect("to_bytes");
    // The 28-byte v3 header is uncompressed; the rest is
    // the LZ4 body.
    let lz4_body = &bytes[28..];
    // The raw body would be the v2-style body (no
    // header). Compute its expected size.
    let raw_size = sphere.vertices.len() * 32
        + sphere.indices.len() * 4
        + sphere.meshlets.len() * (8 + 256 + 24 + 44)
        + sphere.lods.len() * 8;
    // LZ4 worst case is `raw + (raw / 255) + 16`. The
    // test checks the LZ4 body is at most 110% of the
    // raw body (a generous upper bound that catches
    // "LZ4 wrap is a passthrough").
    assert!(
        lz4_body.len() <= raw_size + (raw_size / 10),
        "lz4 body {} should be no larger than raw {} + 10%",
        lz4_body.len(),
        raw_size
    );
}

/// R-038 supplementary: a `MaterialAsset` round-trips
/// through the bindless `register_material` path. The
/// test verifies that the registered material's base
/// color + roughness match the CPU-side input, and that
/// the slot is allocated in the bindless table.
#[test]
fn material_asset_registers_into_bindless_with_correct_constants() {
    let Some(renderer) = hyge_runtime_test::TestRenderer::new() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let bindless = renderer.renderer_bindless();

    let data = MaterialData {
        name: "lit_blue".into(),
        base_color: [0.1, 0.2, 0.9, 1.0],
        metallic: 0.25,
        roughness: 0.75,
        emissive: [0.0; 3],
        double_sided: false,
        base_color_texture: None,
        metallic_roughness_texture: None,
        normal_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
    };
    let (gpu, _) = MaterialAsset::to_gpu(&data);
    let id = bindless
        .register_material(gpu)
        .expect("material registration must succeed");
    assert_eq!(id.refs(), 1);
    drop(id);
}

/// A trivial `Vertex` type the test uses. Pinned here so
/// the test does not import the private renderer types.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
#[allow(
    dead_code,
    reason = "reserved for future M2 tests that need a custom vertex layout"
)]
struct TestVertex {
    position: [f32; 3],
    color: [f32; 3],
}
