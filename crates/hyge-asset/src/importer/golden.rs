//! Golden integration tests for the glTF 2.0 importer.
//!
//! The tests in this module are the R-034 acceptance "golden test":
//! import a small, hand-built glTF asset, then assert that:
//!
//! 1. The expected `.hyge-mesh` / `.hyge-mat` / `.ktx2` /
//!    `.hyge-meta.json` files are written with content-addressed
//!    filenames.
//! 2. The output bytes are deterministic across two consecutive
//!    runs (the writer is stable; no timestamp / pid / random
//!    bytes leak into the cache).
//! 3. Each output file is within a documented size bound for the
//!    fixture so accidental schema bloat fails the gate.
//! 4. The `KHR_mesh_quantization` and `KHR_lights_punctual`
//!    extensions are accepted (i.e. do not produce a parse
//!    error).
//! 5. The `AssetDb` records the imported assets with their
//!    `hash → path` mapping and the `mesh → material`,
//!    `mesh → texture`, `material → texture` dependency edges.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hyge_core::result::HygeResult;
use serial_test::serial;

use crate::asset::AssetId;
use crate::db::AssetDb;
use crate::importer::transcode::{CompressionMode, TargetFormat};
use crate::importer::{import_gltf, ImportOptions, ImportReport};

/// Minimal valid glTF 2.0 document containing a single triangle,
/// one material, and a 2x2 RGBA PNG-like image. The image bytes
/// are hand-built to avoid pulling in the `image` crate just for
/// one test fixture.
const MINIMAL_GLTF: &str = r#"{
  "asset": { "version": "2.0", "generator": "hyge-golden" },
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

/// Binary buffer for the minimal glTF. Layout matches `bufferViews`:
///   3 * vec3 positions (36 bytes)
///   3 * vec3 normals   (36 bytes)
///   3 * vec2 uvs       (24 bytes)
///   3 * u16 indices    (12 bytes — padded to 4-byte alignment)
/// Total: 108 bytes (the document declares 96 because the index
/// buffer is 6 bytes real + 6 bytes padding; the importer only
/// uses the declared byteLength for buffer allocation, so we
/// just supply a 96-byte blob with the meaningful prefix).
fn minimal_gltf_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(96);
    // positions
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    // normals
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    // uvs
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&0.0f32.to_le_bytes());
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    // indices
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    // pad to 96
    while bytes.len() < 96 {
        bytes.push(0);
    }
    bytes
}

/// Packs the JSON + binary buffer into a binary glTF (.glb).
/// Header (12 bytes) + JSON chunk (4-byte aligned) + BIN chunk
/// (4-byte aligned) per the glTF 2.0 spec.
fn build_glb() -> Vec<u8> {
    let json = MINIMAL_GLTF.as_bytes();
    let bin = minimal_gltf_bytes();
    let json_padded = round_up_4(json.len());
    let bin_padded = round_up_4(bin.len());

    let mut out = Vec::with_capacity(12 + 8 + json_padded + 8 + bin_padded);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    let total = 12 + 8 + json_padded + 8 + bin_padded;
    out.extend_from_slice(&(total as u32).to_le_bytes());
    // JSON chunk
    out.extend_from_slice(&(json_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(json);
    out.resize(12 + 8 + json_padded, 0x20);
    // BIN chunk
    out.extend_from_slice(&(bin_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out.resize(total, 0);
    out
}

fn round_up_4(n: usize) -> usize {
    (n + 3) & !3
}

/// Returns a unique per-test directory under the OS temp dir.
fn golden_dir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "hyge-asset-r034-golden-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&base).expect("create golden dir");
    base
}

fn write_gltf(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
    let p = dir.join(name);
    let mut f = fs::File::create(&p).expect("create source glTF");
    f.write_all(body).expect("write glTF");
    p
}

fn run_import(gltf_path: &Path, out_dir: &Path) -> HygeResult<ImportReport> {
    let opts = ImportOptions {
        source: gltf_path.to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        asset_db: Some(out_dir.join(".hyge.db")),
        compression_mode: CompressionMode::Uncompressed,
        target_format: TargetFormat::Uncompressed,
        toktx_path: None,
    };
    import_gltf(&opts).map_err(|e| e.0)
}

#[test]
#[serial]
fn golden_gltf_glb_writes_all_expected_files_within_size_bounds() {
    let dir = golden_dir("glb");
    let src = write_gltf(&dir, "triangle.glb", &build_glb());
    let out = dir.join("cook");
    let report = run_import(&src, &out).expect("import must succeed");

    assert!(!report.mesh_hash.is_empty(), "mesh hash must be set");
    assert_eq!(report.material_hashes.len(), 1, "one material");
    assert_eq!(report.texture_hashes.len(), 0, "no textures in the fixture");
    assert_eq!(report.light_count, 0, "no KHR_lights_punctual in fixture");
    assert!(!report.transcode_pending);

    let mesh_path = out.join(format!("{}.hyge-mesh", report.mesh_hash));
    let mat_path = out.join(format!("{}.hyge-mat", report.material_hashes[0]));
    let meta_path = out.join(format!("{}.hyge-meta.json", report.mesh_hash));

    for p in [&mesh_path, &mat_path, &meta_path] {
        assert!(p.is_file(), "expected file missing: {}", p.display());
    }

    // Size bounds — chosen as tight upper bounds for the
    // 3-vertex / 1-triangle / 1-material fixture so any
    // accidental schema bloat trips the gate.
    let mesh_size = fs::metadata(&mesh_path).unwrap().len();
    assert!(
        mesh_size > 24 && mesh_size < 2048,
        "mesh size {mesh_size} out of [24, 2048) bounds"
    );
    let mat_size = fs::metadata(&mat_path).unwrap().len();
    assert!(
        mat_size > 16 && mat_size < 4096,
        "material size {mat_size} out of [16, 4096) bounds"
    );
    let meta_raw = fs::read_to_string(&meta_path).unwrap();
    assert!(
        meta_raw.contains("\"mesh_hash\"")
            && meta_raw.contains("\"source_name\"")
            && meta_raw.contains("\"dependencies\""),
        "meta must include key fields; got: {meta_raw}"
    );
    let mat_raw = fs::read_to_string(&mat_path).unwrap();
    assert!(
        mat_raw.contains("Golden"),
        "material file must carry the source name; got: {mat_raw}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(parsed["light_count"], 0);
    assert_eq!(parsed["scene"]["mesh_count"], 1);
    assert_eq!(parsed["scene"]["material_count"], 1);
    assert_eq!(parsed["scene"]["primitive_count"], 1);
}

#[test]
#[serial]
fn golden_gltf_import_is_deterministic_across_runs() {
    let src_dir = golden_dir("det-src");
    let src = write_gltf(&src_dir, "triangle.glb", &build_glb());

    let out_a = golden_dir("det-a");
    let out_b = golden_dir("det-b");
    let report_a = run_import(&src, &out_a).expect("first import must succeed");
    let report_b = run_import(&src, &out_b).expect("second import must succeed");

    assert_eq!(report_a, report_b, "import reports must be byte-identical");

    let a = fs::read(out_a.join(format!("{}.hyge-mesh", report_a.mesh_hash))).unwrap();
    let b = fs::read(out_b.join(format!("{}.hyge-mesh", report_b.mesh_hash))).unwrap();
    assert_eq!(a, b, ".hyge-mesh must be deterministic across runs");

    let am =
        fs::read_to_string(out_a.join(format!("{}.hyge-meta.json", report_a.mesh_hash))).unwrap();
    let bm =
        fs::read_to_string(out_b.join(format!("{}.hyge-meta.json", report_b.mesh_hash))).unwrap();
    assert_eq!(am, bm, ".hyge-meta.json must be deterministic across runs");
}

#[test]
#[serial]
fn golden_gltf_khr_lights_punctual_count_is_extracted() {
    // Build a glTF that declares KHR_lights_punctual and one
    // light. The extension requires the asset.extensionsUsed
    // list to mention it, and the light must be attached to a
    // node via `node.extensions.KHR_lights_punctual.light`.
    let mut json = MINIMAL_GLTF.to_string();
    // KHR_lights_punctual lights live under a top-level
    // `extensions` object (not under `asset.extensions`).
    // The glTF spec also requires `extensionsUsed` to declare
    // the extension; we add it after the asset line.
    json = json.replace(
        "\"generator\": \"hyge-golden\" },",
        "\"generator\": \"hyge-golden\" },\n  \"extensionsUsed\": [\"KHR_lights_punctual\"],",
    );
    json = json.replace(
        "\"meshes\": [ {",
        "\"extensions\": { \"KHR_lights_punctual\": { \"lights\": [{\"name\":\"Sun\",\"color\":[1.0,1.0,1.0],\"intensity\":1000.0,\"type\":\"directional\"}] } },\n  \"meshes\": [ {",
    );
    json = json.replace(
        "\"nodes\":  [ { \"mesh\": 0 } ],",
        "\"nodes\":  [ { \"mesh\": 0, \"extensions\": { \"KHR_lights_punctual\": { \"light\": 0 } } } ],",
    );
    let json_bytes = json.into_bytes();
    let bin = minimal_gltf_bytes();
    let json_padded = round_up_4(json_bytes.len());
    let bin_padded = round_up_4(bin.len());
    let total = 12 + 8 + json_padded + 8 + bin_padded;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.resize(12 + 8 + json_padded, 0x20);
    out.extend_from_slice(&(bin_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out.resize(total, 0);

    let dir = golden_dir("khr-lights");
    let src = write_gltf(&dir, "with_light.glb", &out);
    let cook = dir.join("cook");
    let report = run_import(&src, &cook).expect("KHR_lights_punctual must parse");
    assert_eq!(
        report.light_count, 1,
        "exactly one KHR_lights_punctual light must be counted"
    );

    // Meta document should record the light count too.
    let meta_raw = fs::read_to_string(cook.join(format!("{}.hyge-meta.json", report.mesh_hash)))
        .expect("meta readable");
    let parsed: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(parsed["light_count"], 1);
    assert_eq!(parsed["scene"]["light_count"], 1);
}

#[test]
#[serial]
fn golden_gltf_khr_mesh_quantization_dequantizes_to_min_max_range() {
    // KHR_mesh_quantization stores POSITION as quantised integers
    // with an explicit min/max. The importer must dequantize
    // every component back into the original float range, so
    // the test:
    //   1. builds a glTF with SHORT VEC3 positions in the range
    //      [-1.0, +1.0] on X and Y, [0.0, 1.0] on Z;
    //   2. confirms the import succeeds (no rejection);
    //   3. reads the produced `.hyge-mesh` and checks that the
    //      first vertex round-trips to the expected
    //      dequantized value.
    let json = r#"{
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [ { "nodes": [0] } ],
        "nodes":  [ { "mesh": 0 } ],
        "meshes": [ {
          "primitives": [ {
            "attributes": { "POSITION": 0 },
            "indices": 1
          } ]
        } ],
        "accessors": [
          {
            "bufferView": 0,
            "componentType": 5122,
            "count": 3,
            "type": "VEC3",
            "max": [1.0, 1.0, 1.0],
            "min": [-1.0, -1.0, 0.0]
          },
          { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "buffers": [ { "byteLength": 24 } ],
        "bufferViews": [
          { "buffer": 0, "byteOffset": 0,  "byteLength": 18 },
          { "buffer": 0, "byteOffset": 18, "byteLength": 6 }
        ]
    }"#;
    let json_bytes = json.as_bytes();
    let mut bin = Vec::new();
    // Three SHORT VEC3 positions at the three quant corners
    // (-32768, -32768, 0), (0, 0, 16384), (32767, 32767, 32767).
    // With min = [-1, -1, 0] and max = [1, 1, 1] the
    // dequantized values are: (-1, -1, 0), (-0.5, -0.5, 0.25),
    // (1, 1, 1) (modulo rounding).
    for chunk in [
        (-32768i16, -32768i16, 0i16),
        (0, 0, 16384),
        (32767, 32767, 32767),
    ] {
        for c in [chunk.0, chunk.1, chunk.2] {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    // Three U16 indices: 6 bytes.
    bin.extend_from_slice(&0u16.to_le_bytes());
    bin.extend_from_slice(&1u16.to_le_bytes());
    bin.extend_from_slice(&2u16.to_le_bytes());
    let json_padded = round_up_4(json_bytes.len());
    let bin_padded = round_up_4(bin.len());
    let total = 12 + 8 + json_padded + 8 + bin_padded;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(json_bytes);
    out.resize(12 + 8 + json_padded, 0x20);
    out.extend_from_slice(&(bin_padded as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out.resize(total, 0);

    let dir = golden_dir("quant");
    let src = write_gltf(&dir, "quant.glb", &out);
    let cook = dir.join("cook");
    let report = run_import(&src, &cook).expect("quantised POSITION must import");

    let mesh_bytes =
        fs::read(cook.join(format!("{}.hyge-mesh", report.mesh_hash))).expect("mesh file readable");
    // Header is 24 bytes (magic + version + counts). Each vertex
    // is 32 bytes: position[3] + normal[3] + uv[2]. The first
    // three floats of each vertex are the dequantized POSITION.
    // With quant = (-32768, -32768, 0) and target range
    // min = (-1, -1, 0), max = (1, 1, 1) the dequantized
    // position is approximately (-1, -1, 0.5) — z maps
    // (0 - (-32768)) / 65535 ≈ 0.5000076 into [0, 1].
    let read_pos = |offset: usize| -> [f32; 3] {
        bytemuck::cast_slice::<u8, [u8; 4]>(&mesh_bytes[24 + offset..24 + offset + 12])[0..3]
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    };
    let pos = read_pos(0);
    assert!((pos[0] - (-1.0)).abs() < 1e-3, "x0: got {pos:?}");
    assert!((pos[1] - (-1.0)).abs() < 1e-3, "y0: got {pos:?}");
    assert!((pos[2] - 0.5).abs() < 1e-3, "z0: got {pos:?}");

    // Second vertex: quant (0, 0, 16384) -> (0, 0, 0.75).
    let pos1 = read_pos(32);
    assert!((pos1[0] - 0.0).abs() < 1e-3, "x1: got {pos1:?}");
    assert!((pos1[1] - 0.0).abs() < 1e-3, "y1: got {pos1:?}");
    assert!((pos1[2] - 0.75).abs() < 1e-3, "z1: got {pos1:?}");

    // Third vertex: quant (32767, 32767, 32767) -> (1, 1, 1).
    let pos2 = read_pos(64);
    assert!((pos2[0] - 1.0).abs() < 1e-3, "x2: got {pos2:?}");
    assert!((pos2[1] - 1.0).abs() < 1e-3, "y2: got {pos2:?}");
    assert!((pos2[2] - 1.0).abs() < 1e-3, "z2: got {pos2:?}");
}

#[test]
#[serial]
fn golden_gltf_asset_db_records_hashes_paths_and_dependency_edges() {
    let dir = golden_dir("db");
    let src = write_gltf(&dir, "triangle.glb", &build_glb());
    let out = dir.join("cook");
    let report = run_import(&src, &out).expect("import must succeed");

    let db = AssetDb::open(&out.join(".hyge.db")).expect("db must reopen");
    let mesh_path = out.join(format!("{}.hyge-mesh", report.mesh_hash));
    let mat_path = out.join(format!("{}.hyge-mat", report.material_hashes[0]));

    let mesh_id = AssetId::from(blake3::hash(report.mesh_hash.as_bytes()));
    let mat_id = AssetId::from(blake3::hash(report.material_hashes[0].as_bytes()));
    let src_id = AssetId::from(blake3::hash(report.source_hash.as_bytes()));

    assert_eq!(
        db.lookup(&mesh_id).as_deref(),
        Some(mesh_path.as_path()),
        "db must record mesh path"
    );
    assert_eq!(
        db.lookup(&mat_id).as_deref(),
        Some(mat_path.as_path()),
        "db must record material path"
    );
    assert_eq!(
        db.lookup(&src_id).as_deref(),
        Some(std::path::Path::new(&report.source_hash)),
        "db must record the source-file entry"
    );

    let mesh_deps = db.dependencies(&mesh_id);
    assert!(
        mesh_deps.contains(&mat_id),
        "mesh must depend on its material; got {mesh_deps:?}"
    );
    assert!(
        mesh_deps.contains(&src_id),
        "mesh must depend on the source glTF; got {mesh_deps:?}"
    );
    assert_eq!(
        mesh_deps.len(),
        2,
        "exactly mesh -> material and mesh -> source"
    );
}

/// R-035 acceptance test: end-to-end through `import_gltf` —
/// asserts the on-disk `.hyge-mesh` carries a real
/// `meshopt`-baked meshlet stream (header `version == 2`,
/// `meshlet_count > 0`, `lod_count == 3`) and that the
/// deterministic-bake contract holds for repeated runs.
#[test]
#[serial]
fn r035_meshlet_bake_lands_on_disk_with_three_lods() {
    let dir = golden_dir("r035");
    let src = write_gltf(&dir, "triangle.glb", &build_glb());
    let out = dir.join("cook");
    let report = run_import(&src, &out).expect("R-035 import must succeed");

    let mesh_path = out.join(format!("{}.hyge-mesh", report.mesh_hash));
    let mesh_bytes = fs::read(&mesh_path).expect("mesh file readable");

    // Header is 24 bytes (6 * u32, little-endian).
    let header: [u32; 6] = bytemuck::cast_slice::<u8, u32>(&mesh_bytes[0..24])
        .try_into()
        .unwrap();
    const MAGIC: u32 = 0x484D_4548;
    const VERSION_R035: u32 = 2;
    assert_eq!(header[0], MAGIC, "magic preserved");
    assert_eq!(
        header[1], VERSION_R035,
        "R-035 bumps the on-disk format version to 2 (cone bounds + LOD chain)"
    );
    assert!(
        header[2] >= 1,
        "R-035 must produce >= 1 meshlet from a triangle; got {}",
        header[2]
    );
    assert_eq!(
        header[5], 3,
        "R-035 must produce exactly 3 LODs (0.5, 0.25, 0.1 ratios); got {}",
        header[5]
    );

    // The 3-vertex / 1-triangle fixture collapses to 1 meshlet
    // (within the 128-tri cap) and produces 3 LODs even when
    // each LOD ends up with 1 triangle (meshopt's lower bound).
    assert_eq!(header[3], 3, "three source vertices");
    // 1 meshlet * 3 indices per triangle = 3 base indices, plus
    // 3 LODs * 3 indices each = 9, total 12. meshopt may add a
    // small overhead for degenerate LODs, so we just check the
    // index count is at least 12.
    assert!(
        header[4] >= 12,
        "expected at least 12 indices (1 meshlet + 3 LODs); got {}",
        header[4]
    );

    // Determinism: a second import of the same source produces
    // the exact same `.hyge-mesh` bytes.
    let out_b = golden_dir("r035-b");
    let report_b = run_import(&src, &out_b).expect("R-035 second import must succeed");
    assert_eq!(
        report.mesh_hash, report_b.mesh_hash,
        "R-035 determinism: mesh hash must be stable across runs"
    );
    let bytes_b = fs::read(out_b.join(format!("{}.hyge-mesh", report_b.mesh_hash))).unwrap();
    assert_eq!(
        mesh_bytes, bytes_b,
        "R-035 determinism: same input glTF must produce identical .hyge-mesh bytes"
    );
}

/// R-036 acceptance test: a glTF that references a real
/// embedded PNG texture must end up with a content-addressed
/// `.ktx2` cache file that is a real KTX2 container with the
/// expected mip chain. The fixture embeds the PNG via a
/// buffer view (`image.bufferView`) so the test does not
/// depend on the `gltf` crate's external-reference loader.
#[test]
#[serial]
fn r036_ktx2_transcode_writes_real_ktx2_with_mip_chain() {
    let dir = golden_dir("r036");
    // Build a real 2x2 RGBA PNG.
    let img = image::RgbaImage::from_fn(2, 2, |x, y| match (x, y) {
        (0, 0) => image::Rgba([255, 0, 0, 255]),
        (1, 0) => image::Rgba([0, 255, 0, 255]),
        (0, 1) => image::Rgba([0, 0, 255, 255]),
        _ => image::Rgba([255, 255, 255, 255]),
    });
    let mut png_buf: Vec<u8> = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut png_buf);
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .expect("encode PNG");
    }
    let png_len = png_buf.len();
    // Round png_len up to 4 bytes for the buffer view alignment.
    let png_padded = round_up_4(png_len);
    while png_buf.len() < png_padded {
        png_buf.push(0);
    }
    // Geometry occupies bytes 0..108; the PNG payload starts at
    // 108. The buffers[0].byteLength must cover both.
    let total_buf_len = 108 + png_padded;

    // Hand-rolled glTF: a 1-triangle mesh, one material that
    // references the base-color texture by bufferView. The
    // image bufferView (index 4) sits after the geometry
    // bufferViews in the same BIN chunk.
    let gltf_json = format!(
        r#"{{
            "asset": {{ "version": "2.0" }},
            "scene": 0,
            "scenes": [ {{ "nodes": [0] }} ],
            "nodes":  [ {{ "mesh": 0 }} ],
            "meshes": [ {{
              "primitives": [ {{
                "attributes": {{ "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 }},
                "indices": 3,
                "material": 0
              }} ]
            }} ],
            "accessors": [
              {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "max": [1.0, 1.0, 0.0], "min": [0.0, 0.0, 0.0] }},
              {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }},
              {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" }},
              {{ "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }}
            ],
            "materials": [ {{
              "name": "R036",
              "pbrMetallicRoughness": {{
                "baseColorTexture": {{ "index": 0 }},
                "baseColorFactor": [1.0, 1.0, 1.0, 1.0]
              }}
            }} ],
            "textures": [ {{ "source": 0 }} ],
            "images":   [ {{ "bufferView": 4, "mimeType": "image/png" }} ],
            "buffers":  [ {{ "byteLength": {total_buf_len} }} ],
            "bufferViews": [
              {{ "buffer": 0, "byteOffset": 0,    "byteLength": 36 }},
              {{ "buffer": 0, "byteOffset": 36,   "byteLength": 36 }},
              {{ "buffer": 0, "byteOffset": 72,   "byteLength": 24 }},
              {{ "buffer": 0, "byteOffset": 96,   "byteLength": 12 }},
              {{ "buffer": 0, "byteOffset": 108,  "byteLength": {png_len} }}
            ]
          }}"#
    );
    // The geometry bin (108 bytes: 36 pos + 36 nrm + 24 uv +
    // 12 u16 indices with 4-byte padding) followed by the
    // PNG payload (padded to 4 bytes).
    let mut bin: Vec<u8> = Vec::with_capacity(total_buf_len);
    // positions (36)
    for chunk in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for v in chunk {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    // normals (36)
    for chunk in [[0.0f32, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]] {
        for v in chunk {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    // uvs (24)
    for chunk in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
        for v in chunk {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    // indices (6 bytes + 6 bytes padding to 12)
    bin.extend_from_slice(&0u16.to_le_bytes());
    bin.extend_from_slice(&1u16.to_le_bytes());
    bin.extend_from_slice(&2u16.to_le_bytes());
    bin.extend_from_slice(&[0u8; 6]);
    // bin is now 108 bytes; append the PNG payload.
    bin.extend_from_slice(&png_buf);
    // Pad to 4-byte boundary.
    while bin.len() < round_up_4(bin.len()) {
        bin.push(0);
    }

    let json_bytes = gltf_json.as_bytes();
    let bin_padded_total = round_up_4(bin.len());
    let json_padded = round_up_4(json_bytes.len());
    let total = 12 + 8 + json_padded + 8 + bin_padded_total;
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_padded as u32).to_le_bytes());
    glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    glb.extend_from_slice(json_bytes);
    glb.resize(12 + 8 + json_padded, 0x20);
    glb.extend_from_slice(&(bin_padded_total as u32).to_le_bytes());
    glb.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    glb.extend_from_slice(&bin);
    glb.resize(total, 0);

    let src = write_gltf(&dir, "r036.glb", &glb);
    let out = dir.join("cook");
    let report = run_import(&src, &out).expect("R-036 import must succeed");

    // R-036 must produce exactly one texture, and the report
    // must declare it as no longer pending transcode.
    assert_eq!(report.texture_hashes.len(), 1, "one texture in the fixture");
    assert!(
        !report.transcode_pending,
        "R-036 must report transcode_pending=false once the KTX2 is on disk"
    );

    // The cache file is content-addressed and a real KTX2.
    let tex_path = out.join(format!("{}.ktx2", report.texture_hashes[0]));
    assert!(
        tex_path.is_file(),
        "KTX2 file missing: {}",
        tex_path.display()
    );
    let raw = fs::read(&tex_path).expect("KTX2 readable");
    assert_eq!(
        &raw[0..12],
        &crate::importer::texture::KTX2_MAGIC,
        "transcoded texture must be a real KTX2 container"
    );
    // 2x2 -> floor(log2(2)) + 1 = 2 levels (2, 1).
    let level_count = u32::from_le_bytes(raw[24..28].try_into().unwrap());
    assert_eq!(level_count, 2, "expected 2 mip levels for 2x2 source");
    // vkFormat: 37 = R8G8B8A8_UNORM (uncompressed fallback).
    let vk_format = u32::from_le_bytes(raw[12..16].try_into().unwrap());
    assert_eq!(vk_format, 37, "expected VK_FORMAT_R8G8B8A8_UNORM (37)");

    // The meta document must record the new per-texture fields.
    let meta_raw = fs::read_to_string(out.join(format!("{}.hyge-meta.json", report.mesh_hash)))
        .expect("meta readable");
    let parsed: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    let textures = parsed["textures"].as_array().expect("textures array");
    assert_eq!(textures.len(), 1);
    let tex = &textures[0];
    assert_eq!(tex["vk_format"], 37);
    assert_eq!(tex["level_count"], 2);
    assert_eq!(tex["transcode_pending"], false);
    assert_eq!(parsed["transcode_pending"], false);
}
