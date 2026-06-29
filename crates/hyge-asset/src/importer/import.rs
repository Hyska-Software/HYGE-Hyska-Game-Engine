//! Top-level glTF import orchestrator.
//!
//! Given a source glTF 2.0 file, this module:
//!
//! 1. Reads and BLAKE3-hashes the source.
//! 2. Parses the file with [`crate::importer::gltf::parse`].
//! 3. Writes `.hyge-mesh` / `.hyge-mat` / `.ktx2` /
//!    `.hyge-meta.json` into the configured cook directory, with
//!    each output content-addressed by its BLAKE3 hash.
//! 4. Records every hash → path mapping plus the
//!    `mesh → material`, `mesh → texture`, `material → texture`
//!    dependency edges into the [`AssetDb`].

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use blake3::Hasher;

use hyge_core::result::{HygeError, HygeResult};

use crate::asset::AssetId;
use crate::db::AssetDb;
use crate::importer::gltf::{self, GltfScene};
use crate::importer::material;
use crate::importer::mesh;
use crate::importer::meta::{self, DependencyEdge, TextureInfo};
use crate::importer::texture;

/// Options that control a single [`import_gltf`] call.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Path to the source `.gltf` or `.glb` file on disk.
    pub source: PathBuf,
    /// Output directory for the cooked cache. Created if it does
    /// not exist.
    pub out_dir: PathBuf,
    /// Optional asset database to record the import into. When
    /// `None`, the import still writes the cache files but skips
    /// the DB write — useful for unit tests.
    pub asset_db: Option<PathBuf>,
}

impl ImportOptions {
    /// Convenience constructor for the common case (no DB write).
    pub fn without_db(source: impl Into<PathBuf>, out_dir: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            out_dir: out_dir.into(),
            asset_db: None,
        }
    }
}

/// Aggregate result of a successful import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// BLAKE3 hex of the source file.
    pub source_hash: String,
    /// BLAKE3 hex of the top-level mesh asset.
    pub mesh_hash: String,
    /// BLAKE3 hexes of the material assets produced.
    pub material_hashes: Vec<String>,
    /// BLAKE3 hexes of the texture assets produced.
    pub texture_hashes: Vec<String>,
    /// Number of `KHR_lights_punctual` lights, 0 when absent.
    pub light_count: u32,
    /// `true` when any non-KTX2 source texture is waiting for
    /// R-036 to transcode it.
    pub transcode_pending: bool,
}

/// Specialised error type for the import orchestrator. Thin
/// newtype around [`HygeError`].
#[derive(Debug)]
pub struct ImportError(pub HygeError);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ImportError {}

impl From<HygeError> for ImportError {
    fn from(e: HygeError) -> Self {
        Self(e)
    }
}

/// Runs a full glTF import end-to-end. See module docs for the
/// pipeline.
///
/// # Errors
///
/// Returns [`ImportError`] wrapping any of:
///
/// - [`HygeError::AssetNotFound`] when `options.source` does not
///   exist.
/// - [`HygeError::Parse`] for malformed glTF / GLB input.
/// - [`HygeError::Io`] for any filesystem failure during the
///   cook.
/// - [`HygeError::InvalidArgument`] when an input file is the
///   wrong kind (e.g. a directory passed where a file is
///   expected).
pub fn import_gltf(options: &ImportOptions) -> Result<ImportReport, ImportError> {
    let ImportOptions {
        source,
        out_dir,
        asset_db,
    } = options;

    if !source.exists() {
        return Err(HygeError::asset_not_found(format!(
            "source glTF does not exist: {}",
            source.display()
        ))
        .into());
    }
    let meta = fs::metadata(source).map_err(|e| io_with_path(e, "stat", source))?;
    if !meta.is_file() {
        return Err(HygeError::invalid_argument(format!(
            "source path is not a regular file: {}",
            source.display()
        ))
        .into());
    }

    fs::create_dir_all(out_dir).map_err(|e| io_with_path(e, "create out_dir", out_dir))?;

    let source_bytes = read_file(source)?;
    let source_hash = hash_hex(&source_bytes);

    let scene = gltf::parse(&source_bytes, source)?;
    let (report, dependencies) = write_outputs(source, out_dir, &source_hash, &scene)?;

    if let Some(db_path) = asset_db {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_with_path(e, "create db parent", parent))?;
        }
        let mut db = AssetDb::open(db_path)?;
        record_in_db(&mut db, out_dir, &report, &dependencies)?;
    }

    Ok(report)
}

fn write_outputs(
    source: &Path,
    out_dir: &Path,
    source_hash: &str,
    scene: &GltfScene,
) -> Result<(ImportReport, Vec<DependencyEdge>), ImportError> {
    // -- mesh ---------------------------------------------------------
    let mesh_bytes = serialize_mesh(&scene.mesh)?;
    let mesh_hash = hash_hex(&mesh_bytes);
    let mesh_path = out_dir.join(format!("{mesh_hash}.hyge-mesh"));
    fs::write(&mesh_path, &mesh_bytes).map_err(|e| io_with_path(e, "write mesh", &mesh_path))?;

    // -- materials ----------------------------------------------------
    let mut material_hashes: Vec<String> = Vec::with_capacity(scene.materials.len());
    for m in &scene.materials {
        let bytes =
            serde_json::to_vec(m).map_err(|e| HygeError::parse(format!("material encode: {e}")))?;
        let hash = hash_hex(&bytes);
        let path = out_dir.join(format!("{hash}.hyge-mat"));
        material::write(&path, m)?;
        material_hashes.push(hash);
    }

    // -- textures -----------------------------------------------------
    // R-034 writes real KTX1 containers with the `.ktx2`
    // extension. R-036 will rewrite them in place with a real
    // KTX2 (BasisU) container, so every texture is marked
    // transcode-pending in the meta document.
    let mut texture_records: Vec<TextureInfo> = Vec::with_capacity(scene.images.len());
    let mut texture_hashes: Vec<String> = Vec::with_capacity(scene.images.len());
    for img in &scene.images {
        let bytes = build_ktx1_bytes(img.width, img.height, img.format, &img.pixels);
        let hash = hash_hex(&bytes);
        let path = out_dir.join(format!("{hash}.ktx2"));
        fs::write(&path, &bytes).map_err(|e| io_with_path(e, "write texture", &path))?;
        texture_records.push(TextureInfo {
            hash: hash.clone(),
            width: img.width,
            height: img.height,
            format: img.format,
            transcode_pending: true,
            source_mime: img.mime.to_string(),
        });
        texture_hashes.push(hash);
    }

    // -- dependencies -------------------------------------------------
    let mut dependencies: Vec<DependencyEdge> = Vec::new();
    for mhash in &material_hashes {
        dependencies.push(DependencyEdge {
            parent: mesh_hash.clone(),
            child: mhash.clone(),
        });
    }
    for thash in &texture_hashes {
        dependencies.push(DependencyEdge {
            parent: mesh_hash.clone(),
            child: thash.clone(),
        });
    }
    dependencies.push(DependencyEdge {
        parent: mesh_hash.clone(),
        child: source_hash.to_string(),
    });

    // -- meta ---------------------------------------------------------
    let report = ImportReport {
        source_hash: source_hash.to_string(),
        mesh_hash: mesh_hash.clone(),
        material_hashes: material_hashes.clone(),
        texture_hashes: texture_hashes.clone(),
        light_count: scene.light_count,
        transcode_pending: texture_records.iter().any(|t| t.transcode_pending),
    };
    let doc = meta::build(
        source
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("gltf"),
        source,
        source_hash,
        &mesh_hash,
        &material_hashes,
        &texture_records,
        &scene.summary,
        &dependencies,
    );
    let meta_path = out_dir.join(format!("{mesh_hash}.hyge-meta.json"));
    meta::write(&meta_path, &doc)?;

    Ok((report, dependencies))
}

fn record_in_db(
    db: &mut AssetDb,
    out_dir: &Path,
    report: &ImportReport,
    dependencies: &[DependencyEdge],
) -> HygeResult<()> {
    let mut assets: Vec<(AssetId, PathBuf)> = Vec::new();
    let mesh_id = id_from_hex(&report.mesh_hash);
    assets.push((
        mesh_id,
        out_dir.join(format!("{}.hyge-mesh", report.mesh_hash)),
    ));
    for h in &report.material_hashes {
        assets.push((id_from_hex(h), out_dir.join(format!("{h}.hyge-mat"))));
    }
    for h in &report.texture_hashes {
        assets.push((id_from_hex(h), out_dir.join(format!("{h}.ktx2"))));
    }
    let source_id = id_from_hex(&report.source_hash);
    assets.push((source_id, PathBuf::from(&report.source_hash)));

    let mut edges: Vec<(AssetId, AssetId)> = Vec::new();
    for edge in dependencies {
        let parent = id_from_hex(&edge.parent);
        let child = id_from_hex(&edge.child);
        if !edges.contains(&(parent, child)) {
            edges.push((parent, child));
        }
    }
    db.record_with_dependencies(assets, edges)
}

fn id_from_hex(hex: &str) -> AssetId {
    let bytes = blake3::hash(hex.as_bytes());
    AssetId::from(bytes)
}

fn read_file(path: &Path) -> HygeResult<Vec<u8>> {
    let mut f = fs::File::open(path).map_err(|e| io_with_path(e, "open", path))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .map_err(|e| io_with_path(e, "read", path))?;
    Ok(buf)
}

fn serialize_mesh(m: &mesh::MeshData) -> HygeResult<Vec<u8>> {
    // The mesh writer is a thin pass-through, so serializing once
    // to a temp file is cheap (the largest mesh a single import
    // can produce in R-034 is bounded by `len(vertices) * 32`).
    let dir = std::env::temp_dir().join(format!(
        "hyge-asset-meshbuf-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).map_err(io_error("create tempdir"))?;
    let p = dir.join("m.hyge-mesh");
    mesh::write(&p, m)?;
    let bytes = fs::read(&p).map_err(io_error("read temp mesh"))?;
    let _ = fs::remove_dir_all(&dir);
    Ok(bytes)
}

fn build_ktx1_bytes(
    width: u32,
    height: u32,
    format: texture::TextureFormat,
    pixels: &[u8],
) -> Vec<u8> {
    // KTX1 layout: 64-byte header + tightly-packed pixel data.
    // The orchestrator writes the bytes directly so the
    // content-addressed filename uses the hash of the actual
    // KTX1 container that ends up on disk.
    let desc = ktx1_descriptor_for(format);
    let mut buf = Vec::with_capacity(64 + pixels.len());
    buf.extend_from_slice(&texture::KTX1_MAGIC);
    buf.extend_from_slice(&texture::KTX1_ENDIANNESS.to_le_bytes());
    buf.extend_from_slice(&desc.0.to_le_bytes()); // glType
    buf.extend_from_slice(&desc.1.to_le_bytes()); // glTypeSize
    buf.extend_from_slice(&desc.2.to_le_bytes()); // glFormat
    buf.extend_from_slice(&desc.3.to_le_bytes()); // glInternalFormat
    buf.extend_from_slice(&desc.4.to_le_bytes()); // glBaseInternalFormat
    buf.extend_from_slice(&width.to_le_bytes());
    buf.extend_from_slice(&height.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // pixelDepth
    buf.extend_from_slice(&0u32.to_le_bytes()); // numberOfArrayElements
    buf.extend_from_slice(&0u32.to_le_bytes()); // numberOfFaces
    buf.extend_from_slice(&1u32.to_le_bytes()); // numberOfMipmapLevels
    buf.extend_from_slice(&0u32.to_le_bytes()); // bytesOfKeyValueData
    buf.extend_from_slice(pixels);
    buf
}

/// `(glType, glTypeSize, glFormat, glInternalFormat, glBaseInternalFormat)`
/// for a given [`TextureFormat`].
fn ktx1_descriptor_for(format: texture::TextureFormat) -> (u32, u32, u32, u32, u32) {
    use texture::TextureFormat::*;
    match format {
        R8 => (0x1401, 1, 0x1903, 0x8229, 0x1903), // GL_UNSIGNED_BYTE, GL_RED, GL_R8
        R8G8 => (0x1401, 1, 0x8227, 0x822B, 0x8227), // GL_RG, GL_RG8
        R8G8B8 => (0x1401, 1, 0x1907, 0x8051, 0x1907), // GL_RGB, GL_RGB8
        R8G8B8A8 => (0x1401, 1, 0x1908, 0x8058, 0x1908), // GL_RGBA, GL_RGBA8
        R16 => (0x1403, 2, 0x1903, 0x822A, 0x1903), // GL_UNSIGNED_SHORT, GL_R16
        R16G16 => (0x1403, 2, 0x8227, 0x822C, 0x8227), // GL_RG16
        R16G16B16A16 => (0x1403, 2, 0x1908, 0x805B, 0x1908), // GL_RGBA16
        R32G32B32A32FLOAT => (0x1406, 4, 0x1908, 0x8814, 0x1908), // GL_FLOAT, GL_RGBA32F
    }
}

fn hash_hex(bytes: &[u8]) -> String {
    let h = Hasher::new().update(bytes).finalize();
    let mut s = String::with_capacity(64);
    for byte in h.as_bytes() {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}

fn io_with_path(err: std::io::Error, op: &str, path: &Path) -> HygeError {
    HygeError::Io(std::io::Error::other(format!(
        "{op} {}: {err}",
        path.display()
    )))
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> HygeError {
    move |e| HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_rejects_missing_source() {
        let dir = std::env::temp_dir().join(format!(
            "hyge-asset-imp-miss-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let opts = ImportOptions::without_db(dir.join("missing.gltf"), dir.join("out"));
        let err = import_gltf(&opts).expect_err("must reject missing source");
        assert!(matches!(err.0, HygeError::AssetNotFound(_)), "got {err:?}");
    }
}
