//! Environment map (.hdr / .exr) import path (R-041).
//!
//! Reads an equirectangular HDR source, hands it to
//! [`hyge_render::ibl::bake_from_rgbe_hdr`] for the CPU bake
//! (prefilter + irradiance + BRDF LUT), and writes the
//! content-addressed `<blake3>.hyge-env` file to `out_dir`.
//!
//! This is the **offline** bake path that the glTF
//! orchestrator invokes when a sibling `.hdr` is detected
//! (acceptance #3). The same code path is reachable from
//! `hyge-tools import <file.hdr>` for standalone HDRs
//! (R-041 acceptance #3 again, plus a future
//! `hyge-tools cook` recursive pass).
//!
//! EXR support is explicitly out of scope for R-041: the
//! `import_environment` function returns
//! `HygeError::Unsupported` for `.exr` so the dispatch can
//! grow into it without breaking the call site. The
//! `is_environment_source` helper still returns `true` for
//! `.exr` so the dispatch can produce a friendly error
//! rather than treating it as an unknown extension.

use std::fs;
use std::path::{Path, PathBuf};

use blake3::Hasher;

use hyge_core::result::{HygeError, HygeResult};
use hyge_render::ibl;

use crate::asset::AssetId;
use crate::db::AssetDb;

/// The result of a successful environment import. Mirrors the
/// shape of [`crate::importer::import::ImportReport`] for the
/// mesh / texture / material paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentImportReport {
    /// BLAKE3 hex of the original `.hdr` source file.
    pub source_hash: String,
    /// BLAKE3 hex of the cooked `.hyge-env` file (the
    /// content-addressed key recorded in the asset DB).
    pub env_hash: String,
    /// Edge in pixels of the prefiltered cubemap's base mip.
    pub prefilter_size: u32,
    /// Number of mip levels in the prefiltered cubemap.
    pub prefilter_mips: u32,
    /// Edge in pixels of the diffuse irradiance cubemap.
    pub irradiance_size: u32,
    /// Edge in pixels of the BRDF integration LUT.
    pub brdf_lut_size: u32,
}

/// Returns `true` when `path` looks like a standalone
/// environment map source the IBL baker can ingest. Supported
/// extensions: `.hdr` (Radiance RGBE), `.exr` (out of scope
/// for R-041; the dispatch returns `HygeError::Unsupported`
/// when invoked with an `.exr` path).
pub fn is_environment_source(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("hdr") || e.eq_ignore_ascii_case("exr"))
}

/// Bakes an equirectangular HDR source into a
/// content-addressed `.hyge-env` file in `out_dir`.
///
/// # Errors
///
/// - [`HygeError::AssetNotFound`] when `source` does not exist.
/// - [`HygeError::InvalidArgument`] when `source` is not a
///   regular file or has the wrong extension.
/// - [`HygeError::Unsupported`] when `source` is `.exr` (the
///   R-041 MVP only ingests RGBE `.hdr`; EXR is a planned
///   follow-up).
/// - [`HygeError::Parse`] for malformed RGBE input.
/// - [`HygeError::Io`] for filesystem failures during the
///   cook.
pub fn import_environment(source: &Path, out_dir: &Path) -> HygeResult<EnvironmentImportReport> {
    import_environment_with_config(source, out_dir, ibl::BakeConfig::default())
}

/// Like [`import_environment`], but with a caller-supplied
/// [`BakeConfig`]. Used by the test suite to run a small,
/// deterministic bake without waiting for the full production
/// resolution.
///
/// # Errors
///
/// Same as [`import_environment`].
pub fn import_environment_with_config(
    source: &Path,
    out_dir: &Path,
    bake_config: ibl::BakeConfig,
) -> HygeResult<EnvironmentImportReport> {
    import_environment_with_config_and_db(source, out_dir, bake_config, None)
}

/// Like [`import_environment_with_config`], with an optional
/// [`AssetDb`] to record the produced `.hyge-env` asset.
///
/// # Errors
///
/// Same as [`import_environment`], plus [`HygeError::Io`] if the
/// DB write fails.
pub fn import_environment_with_config_and_db(
    source: &Path,
    out_dir: &Path,
    bake_config: ibl::BakeConfig,
    asset_db: Option<&mut AssetDb>,
) -> HygeResult<EnvironmentImportReport> {
    if !source.exists() {
        return Err(HygeError::asset_not_found(format!(
            "environment source does not exist: {}",
            source.display()
        )));
    }
    let meta = fs::metadata(source).map_err(|e| io_with_path(e, "stat", source))?;
    if !meta.is_file() {
        return Err(HygeError::invalid_argument(format!(
            "environment source is not a regular file: {}",
            source.display()
        )));
    }
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "hdr" => {}
        "exr" => {
            return Err(HygeError::Unsupported(format!(
                "EXR environment maps are not yet supported by the R-041 IBL baker \
                 ({}); use a Radiance .hdr source or wait for the EXR follow-up.",
                source.display()
            )));
        }
        other => {
            return Err(HygeError::invalid_argument(format!(
                "unsupported environment source extension: .{other}"
            )));
        }
    }

    fs::create_dir_all(out_dir).map_err(|e| io_with_path(e, "create out_dir", out_dir))?;

    let source_bytes = fs::read(source).map_err(|e| io_with_path(e, "read", source))?;
    let source_hash = hex_lower(Hasher::new().update(&source_bytes).finalize().as_bytes());

    let bake = ibl::bake_from_rgbe_hdr_with_config(&source_bytes, bake_config)?;
    let env_bytes = ibl::encode_for_test(&bake);
    let env_hash = hex_lower(Hasher::new().update(&env_bytes).finalize().as_bytes());
    let env_path = out_dir.join(format!("{env_hash}.hyge-env"));
    fs::write(&env_path, &env_bytes).map_err(|e| io_with_path(e, "write env", &env_path))?;

    if let Some(db) = asset_db {
        let env_id = AssetId::from(blake3::hash(env_hash.as_bytes()));
        db.insert(&env_id, &env_path)?;
    }

    Ok(EnvironmentImportReport {
        source_hash,
        env_hash,
        prefilter_size: bake.prefilter.base_size,
        prefilter_mips: bake.prefilter.mip_count,
        irradiance_size: bake.irradiance.size,
        brdf_lut_size: bake.brdf_lut.size,
    })
}

fn io_with_path(err: std::io::Error, op: &str, path: &Path) -> HygeError {
    HygeError::Io(std::io::Error::other(format!(
        "{op} {}: {err}",
        path.display()
    )))
}

fn hex_lower(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for byte in bytes {
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}

/// Sibling-`.hdr` lookup for the glTF orchestrator: given a
/// glTF source, return the path of a sibling `.hdr` file when
/// one is present. The Khronos glTF-Sample-Environments test
/// corpus uses the convention `<scene>.hdr` next to
/// `<scene>.gltf`; the M3 hook follows the same convention.
///
/// Returns `None` when no sibling `.hdr` is present.
pub fn sibling_hdr(gltf_source: &Path) -> Option<PathBuf> {
    let stem = gltf_source.file_stem()?;
    let parent = gltf_source.parent()?;
    let candidate = parent.join(format!("{}.hdr", stem.to_string_lossy()));
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}
