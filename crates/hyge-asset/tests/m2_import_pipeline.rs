//! R-038 M2 import-pipeline integration test.
//!
//! The M2 acceptance includes "BLAKE3 hashing stable, LZ4
//! compression on, SQLite DB created and queried". This
//! test verifies the full import → DB → runtime path:
//!
//! 1. A small glTF sphere is imported via `import_gltf`.
//!    The output files (`.hyge-mesh`, `.hyge-mat`,
//!    `.ktx2`, `.hyge-meta.json`) are content-addressed
//!    by BLAKE3 hashes.
//! 2. The on-disk `.hyge-mesh` is LZ4-compressed (R-038).
//! 3. The `AssetDb` is created, the import is recorded,
//!    and the assets are queryable by hash.
//! 4. The same import is re-run with the same input; the
//!    output hashes are stable (BLAKE3 determinism).
//! 5. A modified glTF sphere produces a different hash
//!    (so the DB can detect a real change).

use std::path::PathBuf;

use hyge_asset::importer::mesh;
use hyge_asset::importer::transcode::{CompressionMode, TargetFormat};
use hyge_asset::prelude::*;
use hyge_asset::AssetDb;
use serial_test::serial;

/// Constructs a unique temp dir for the test. The dir
/// is created on disk so the importer + DB can write
/// into it.
fn unique_temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hyge-asset-m2-smoke-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir should be creatable");
    dir
}

/// Builds the minimum valid GLB the importer accepts.
/// The exact layout matches the one used by the R-034
/// golden tests (see
/// `crates/hyge-asset/src/importer/golden.rs`).
fn build_test_glb() -> Vec<u8> {
    let json = r#"{
  "asset": { "version": "2.0" },
  "scene": 0,
  "scenes": [ { "nodes": [0] } ],
  "nodes":  [ { "mesh": 0 } ],
  "meshes": [ {
    "primitives": [ {
      "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
      "indices": 3,
      "material": 0
    } ]
  } ],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "max": [1.0, 1.0, 0.0], "min": [0.0, 0.0, 0.0] },
    { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
    { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
    { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }
  ],
  "materials": [ {
    "name": "Golden",
    "pbrMetallicRoughness": {
      "baseColorFactor": [0.8, 0.1, 0.1, 1.0],
      "metallicFactor": 0.0,
      "roughnessFactor": 0.5
    }
  } ],
  "buffers": [ { "byteLength": 96 } ],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0,  "byteLength": 36 },
    { "buffer": 0, "byteOffset": 36, "byteLength": 36 },
    { "buffer": 0, "byteOffset": 72, "byteLength": 24 },
    { "buffer": 0, "byteOffset": 96, "byteLength": 12 }
  ]
}
"#;
    let mut bin: Vec<u8> = Vec::with_capacity(96);
    // positions (3 * vec3)
    for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&f.to_le_bytes());
    }
    // normals
    for f in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0] {
        bin.extend_from_slice(&f.to_le_bytes());
    }
    // uvs
    for f in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
        bin.extend_from_slice(&f.to_le_bytes());
    }
    // indices
    for w in [0u16, 1, 2] {
        bin.extend_from_slice(&w.to_le_bytes());
    }
    while bin.len() < 96 {
        bin.push(0);
    }

    let json_bytes = json.as_bytes();
    let json_padded = (json_bytes.len() + 3) & !3;
    let bin_padded = (bin.len() + 3) & !3;
    let total = 12 + 8 + json_padded + 8 + bin_padded;
    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    // JSON chunk
    out.extend_from_slice(&(json_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(json_bytes);
    out.resize(12 + 8 + json_padded, 0x20);
    // BIN chunk
    out.extend_from_slice(&(bin_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out.resize(total, 0);
    out
}

/// R-038 acceptance: a glTF import produces LZ4-compressed
/// `.hyge-mesh` files (R-038) that are recorded in a
/// SQLite `AssetDb` and queryable by BLAKE3 hash.
#[test]
#[serial]
fn m2_glb_import_produces_lz4_mesh_and_sqlite_db_is_queryable() {
    let temp = unique_temp_dir("import");
    let src = temp.join("sphere.glb");
    std::fs::write(&src, build_test_glb()).expect("glb should be writable");
    let out = temp.join("cook");
    let db_path = out.join(".hyge.db");

    let opts = ImportOptions {
        source: src.clone(),
        out_dir: out.clone(),
        asset_db: Some(db_path.clone()),
        compression_mode: CompressionMode::Uncompressed,
        target_format: TargetFormat::Bc7,
        toktx_path: None,
    };
    let report = import_gltf(&opts).expect("import must succeed");
    assert!(
        !report.mesh_hash.is_empty(),
        "import report must contain a mesh hash"
    );

    // 1. The on-disk `.hyge-mesh` is the LZ4-compressed
    //    v3 format. Read the first 28 bytes and verify
    //    the header.
    let mesh_path = out.join(format!("{}.hyge-mesh", report.mesh_hash));
    let mesh_bytes = std::fs::read(&mesh_path).expect("mesh file readable");
    assert!(
        mesh_bytes.len() > 28,
        "mesh file must have v3 header + body"
    );
    let header: [u32; 7] = bytemuck::cast_slice(&mesh_bytes[0..28]).try_into().unwrap();
    assert_eq!(header[0], 0x484D_4548, "magic preserved");
    assert_eq!(header[1], 3, "v3 LZ4 format");
    assert_eq!(header[2], 0x1, "FLAG_LZ4 set");
    assert!(header[3] >= 1, "at least 1 meshlet");

    // 2. The mesh bytes round-trip through `from_bytes`.
    let _back = mesh::from_bytes(&mesh_bytes).expect("mesh must parse");

    // 3. The SQLite DB is created and queryable.
    let db = AssetDb::open(&db_path).expect("DB should be re-openable");
    let mesh_id = AssetId::from(blake3::hash(report.mesh_hash.as_bytes()));
    let mesh_path_in_db = db.lookup(&mesh_id);
    assert_eq!(
        mesh_path_in_db.as_deref(),
        Some(mesh_path.as_path()),
        "DB must record the mesh path"
    );

    // 4. Re-importing the same glTF produces the same
    //    hashes. BLAKE3 is deterministic.
    let out_b = temp.join("cook-b");
    std::fs::create_dir_all(&out_b).unwrap();
    let opts_b = ImportOptions {
        source: src.clone(),
        out_dir: out_b.clone(),
        asset_db: None,
        compression_mode: CompressionMode::Uncompressed,
        target_format: TargetFormat::Bc7,
        toktx_path: None,
    };
    let report_b = import_gltf(&opts_b).expect("re-import must succeed");
    assert_eq!(
        report.mesh_hash, report_b.mesh_hash,
        "BLAKE3 hash must be stable across re-imports"
    );

    // 5. A modified glTF produces a different hash.
    let mut modified = build_test_glb();
    // The GLB header + JSON chunk header are 20 bytes;
    // the JSON is padded to 4-byte alignment. Modify a
    // byte deep in the BIN section (the first position
    // vertex is at offset 1060 in the standard layout).
    let bin_offset = 12 + 8 + 1024 + 8 + 8;
    if modified.len() > bin_offset {
        modified[bin_offset] = modified[bin_offset].wrapping_add(1);
    }
    let src_b = temp.join("sphere-mod.glb");
    std::fs::write(&src_b, &modified).expect("modified glb should be writable");
    let opts_c = ImportOptions {
        source: src_b,
        out_dir: temp.join("cook-c"),
        asset_db: None,
        compression_mode: CompressionMode::Uncompressed,
        target_format: TargetFormat::Bc7,
        toktx_path: None,
    };
    let report_c = import_gltf(&opts_c).expect("modified import must succeed");
    assert_ne!(
        report.mesh_hash, report_c.mesh_hash,
        "modified glTF must produce a different BLAKE3 hash"
    );
}

/// R-038 acceptance: LZ4 compression is enabled by
/// default. The mesh body's LZ4-compressed size is
/// smaller (or at most +10% overhead) than the raw
/// body, so the wrap is actually compressing.
#[test]
fn lz4_mesh_body_is_compressed_by_default() {
    use hyge_asset::importer::mesh::MeshData;
    use hyge_asset::importer::mesh::Vertex as MeshVertex;
    let mut vertices = Vec::new();
    // A 4x4 grid of triangles = 9 vertices + 8 triangles
    // = 24 indices. Enough geometry for LZ4 to find
    // repetition.
    for i in 0..9 {
        for j in 0..9 {
            vertices.push(MeshVertex {
                position: [i as f32, j as f32, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [i as f32 / 8.0, j as f32 / 8.0],
            });
        }
    }
    let mut indices = Vec::new();
    for i in 0..8 {
        for j in 0..8 {
            let first = (i * 9 + j) as u32;
            indices.push(first);
            indices.push(first + 9);
            indices.push(first + 1);
            indices.push(first + 9);
            indices.push(first + 10);
            indices.push(first + 1);
        }
    }
    let mesh = MeshData::from_triangle_list(vertices, indices);
    let bytes = mesh::to_bytes(&mesh).expect("to_bytes");
    // The 28-byte v3 header is uncompressed; the rest is
    // the LZ4 body.
    let lz4_body = &bytes[28..];
    let raw_size = mesh.vertices.len() * 32
        + mesh.indices.len() * 4
        + mesh.meshlets.len() * (8 + 256 + 24 + 44)
        + mesh.lods.len() * 8;
    assert!(
        lz4_body.len() <= raw_size,
        "lz4 body {} should be no larger than raw {} (LZ4 should compress the regular geometry)",
        lz4_body.len(),
        raw_size
    );
}

/// R-038 acceptance: the SQLite DB is created on import
/// and is re-openable. The DB stores hash → path
/// mappings and dependency edges.
#[test]
#[serial]
fn sqlite_db_is_created_on_import_and_reopenable() {
    let temp = unique_temp_dir("db");
    let src = temp.join("sphere.glb");
    std::fs::write(&src, build_test_glb()).expect("glb should be writable");
    let out = temp.join("cook");
    let db_path = out.join(".hyge.db");

    let opts = ImportOptions {
        source: src.clone(),
        out_dir: out.clone(),
        asset_db: Some(db_path.clone()),
        compression_mode: CompressionMode::Uncompressed,
        target_format: TargetFormat::Bc7,
        toktx_path: None,
    };
    let _ = import_gltf(&opts).expect("import must succeed");

    // The DB file is created on disk.
    assert!(db_path.exists(), ".hyge.db should be created");
    let db = AssetDb::open(&db_path).expect("DB should re-open");
    let mesh_id = AssetId::from(blake3::hash(_mesh_hash_for_report(&opts).as_bytes()));
    // The DB's lookup returns the on-disk path; the path
    // resolves to a file that exists.
    let path = db.lookup(&mesh_id);
    assert!(
        path.is_some(),
        "DB should contain an entry for the imported mesh"
    );
    let path = path.unwrap();
    assert!(
        path.is_file(),
        "DB-resolved path should point at a real file"
    );
}

/// Helper: re-imports the same glTF and returns the mesh
/// hash. The smoke tests in this file use the path to
/// look up the entry in the DB.
fn _mesh_hash_for_report(opts: &ImportOptions) -> String {
    import_gltf(opts).expect("re-import").mesh_hash
}
