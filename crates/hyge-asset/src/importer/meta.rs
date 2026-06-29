//! `.hyge-meta.json` writer for imported glTF assets.
//!
//! The meta file is the per-import sidecar that records everything a
//! downstream consumer needs without having to re-parse the source
//! glTF: human-readable name, the source path, the BLAKE3 hashes of
//! every sub-asset the import produced, the dependency graph, and
//! whether any texture is still waiting for the R-036 KTX2 transcode.

use std::fs;
use std::path::Path;

use hyge_core::result::HygeResult;
use serde::{Deserialize, Serialize};

use crate::importer::gltf::SceneSummary;
use crate::importer::texture::TextureFormat;

/// A single node in the dependency graph, expressed as a `parent ->
/// child` edge. Stored as a flat list of edges for easy JSON
/// round-tripping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// BLAKE3 hex of the dependent asset.
    pub parent: String,
    /// BLAKE3 hex of the asset `parent` depends on.
    pub child: String,
}

/// Top-level meta document written to `<hash>.hyge-meta.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaDocument {
    /// Schema version. Bump on incompatible layout changes.
    pub schema_version: u32,
    /// Source glTF name (the `asset.name` field, when present).
    pub source_name: String,
    /// Source glTF path on disk, as the user passed it to the
    /// importer.
    pub source_path: String,
    /// Hash of the source glTF file.
    pub source_hash: String,
    /// BLAKE3 hash of the top-level mesh asset.
    pub mesh_hash: String,
    /// BLAKE3 hashes of the material assets produced.
    pub material_hashes: Vec<String>,
    /// BLAKE3 hashes of the texture assets produced.
    pub texture_hashes: Vec<String>,
    /// Number of lights extracted from `KHR_lights_punctual` (0
    /// when the extension is not used).
    pub light_count: u32,
    /// `true` if any non-KTX2 source textures were written as
    /// passthrough and still need R-036 transcode.
    pub transcode_pending: bool,
    /// Detailed per-texture record, so the inspector command and
    /// R-036 can introspect each one without re-parsing the
    /// source.
    pub textures: Vec<TextureRecord>,
    /// Flat list of dependency edges. The runtime re-loads these
    /// into the [`crate::AssetDb`] for hot-reload invalidation.
    pub dependencies: Vec<DependencyEdge>,
    /// High-level scene counts (meshes, primitives, materials,
    /// textures, lights) for the inspector command.
    pub scene: SceneSummary,
}

/// Per-texture record stored in the meta document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureRecord {
    /// BLAKE3 hash of the texture payload.
    pub hash: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel format (the integer encoding of
    /// [`crate::importer::texture::TextureFormat`]).
    pub format: u8,
    /// `true` for passthrough bytes; `false` once R-036 has
    /// transcoded the file to a real KTX2 container.
    pub transcode_pending: bool,
    /// Mime type of the source image.
    pub source_mime: String,
}

const SCHEMA_VERSION: u32 = 1;

/// One texture record the orchestrator passes in.
#[derive(Debug, Clone)]
pub struct TextureInfo {
    /// BLAKE3 hash of the written `.ktx2` file.
    pub hash: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel format.
    pub format: TextureFormat,
    /// `true` for passthrough bytes; `false` for an already-valid
    /// KTX2 container.
    pub transcode_pending: bool,
    /// Mime type of the source image.
    pub source_mime: String,
}

/// Builds a [`MetaDocument`] from the pieces the importer collects
/// during a glTF run. The summary plus the dependency edges are the
/// only inputs the meta writer actually cares about; everything else
/// is mirrored verbatim.
#[allow(clippy::too_many_arguments)]
pub fn build(
    source_name: &str,
    source_path: &Path,
    source_hash: &str,
    mesh_hash: &str,
    material_hashes: &[String],
    textures: &[TextureInfo],
    summary: &SceneSummary,
    dependencies: &[DependencyEdge],
) -> MetaDocument {
    let transcode_pending = textures.iter().any(|t| t.transcode_pending);
    let texture_records = textures
        .iter()
        .map(|t| TextureRecord {
            hash: t.hash.clone(),
            width: t.width,
            height: t.height,
            format: t.format as u8,
            transcode_pending: t.transcode_pending,
            source_mime: t.source_mime.clone(),
        })
        .collect();
    let texture_hashes = textures.iter().map(|t| t.hash.clone()).collect();
    MetaDocument {
        schema_version: SCHEMA_VERSION,
        source_name: source_name.to_string(),
        source_path: source_path.display().to_string(),
        source_hash: source_hash.to_string(),
        mesh_hash: mesh_hash.to_string(),
        material_hashes: material_hashes.to_vec(),
        texture_hashes,
        light_count: summary.light_count,
        transcode_pending,
        textures: texture_records,
        dependencies: dependencies.to_vec(),
        scene: summary.clone(),
    }
}

/// Writes `doc` to `path` as pretty-printed JSON.
///
/// # Errors
///
/// Returns [`hyge_core::result::HygeError::Io`] on filesystem
/// failure or [`hyge_core::result::HygeError::Parse`] on JSON
/// serialization failure.
pub fn write(path: &Path, doc: &MetaDocument) -> HygeResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create meta parent dir"))?;
    }
    let bytes = serde_json::to_vec_pretty(doc)
        .map_err(|e| hyge_core::result::HygeError::parse(format!("meta json: {e}")))?;
    fs::write(path, bytes).map_err(io_error("write meta file"))?;
    Ok(())
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> hyge_core::result::HygeError {
    move |e| hyge_core::result::HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::texture::TextureFormat;

    #[test]
    fn build_marks_transcode_pending_when_any_texture_is_ktx1_passthrough() {
        let textures = vec![
            TextureInfo {
                hash: "t1".into(),
                width: 16,
                height: 16,
                format: TextureFormat::R8G8B8A8,
                transcode_pending: true,
                source_mime: "image/png".into(),
            },
            TextureInfo {
                hash: "t2".into(),
                width: 32,
                height: 32,
                format: TextureFormat::R8G8B8A8,
                transcode_pending: true,
                source_mime: "image/ktx2".into(),
            },
        ];
        let doc = build(
            "Box",
            std::path::Path::new("box.gltf"),
            "sourcehash",
            "meshhash",
            &["m1".into()],
            &textures,
            &SceneSummary {
                mesh_count: 1,
                primitive_count: 1,
                material_count: 1,
                texture_count: 2,
                light_count: 0,
            },
            &[DependencyEdge {
                parent: "m1".into(),
                child: "t1".into(),
            }],
        );
        assert!(doc.transcode_pending);
        assert_eq!(doc.textures.len(), 2);
        assert_eq!(doc.textures[0].width, 16);
        assert_eq!(doc.textures[0].format, TextureFormat::R8G8B8A8 as u8);
        assert_eq!(doc.dependencies.len(), 1);
    }

    #[test]
    fn build_no_transcode_when_all_textures_are_real_ktx2() {
        let textures = vec![TextureInfo {
            hash: "t".into(),
            width: 8,
            height: 8,
            format: TextureFormat::R8G8B8A8,
            transcode_pending: false,
            source_mime: "image/ktx2".into(),
        }];
        let doc = build(
            "Ktx",
            std::path::Path::new("k.gltf"),
            "sh",
            "mh",
            &[],
            &textures,
            &SceneSummary::default(),
            &[],
        );
        assert!(!doc.transcode_pending);
    }
}
