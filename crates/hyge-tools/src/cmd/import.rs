//! `hyge-tools import` — cook a single source asset into the project cache.
//!
//! Three code paths share this entry point:
//!
//! - `.gltf` / `.glb` sources go through the full
//!   [`hyge_asset::importer::import_gltf`] pipeline (R-034 + R-036):
//!   glTF 2.0 is parsed, the mesh / material outputs are written
//!   to the cache, and every texture is **transcoded to KTX2**
//!   (real container, full mip chain) by the R-036 pipeline.
//! - `.png` / `.jpg` / `.jpeg` / `.exr` standalone texture
//!   sources are decoded by the [`image`] crate and fed
//!   directly to the KTX2 transcoder (R-036). EXR is opt-in via
//!   the `texture-exr` feature on the `image` crate.
//! - Any other extension falls through to a minimal
//!   hash-and-sidecar-manifest path so `hyge-tools cook` continues
//!   to work for scripts / audio / other formats that the
//!   importer does not yet cover.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use blake3::Hasher;

use hyge_asset::importer::texture::TextureFormat;
use hyge_asset::importer::transcode::{self, CompressionMode, TargetFormat, TranscodeResult};
use hyge_asset::importer::ImportOptions;
use hyge_asset::prelude::import_gltf;
use hyge_core::hyge_log;
use hyge_core::result::{HygeError, HygeResult};

use crate::cmd::ASSETS_COOK_DIR;

/// Returns true when `path` looks like a glTF 2.0 source (`.gltf`
/// or `.glb`). Case-insensitive.
pub fn is_gltf_source(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gltf") || e.eq_ignore_ascii_case("glb"))
}

/// Returns `true` when the path looks like a standalone texture
/// source the KTX2 transcoder can ingest directly. Supported
/// extensions: `png`, `jpg`, `jpeg`, `exr` (case-insensitive).
pub fn is_texture_source(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let e = e.to_ascii_lowercase();
        matches!(e.as_str(), "png" | "jpg" | "jpeg" | "exr")
    })
}

/// Hashes the file at `path` with BLAKE3 and returns the
/// 64-character lowercase hex digest.
///
/// Reads the file in 64 KiB chunks so very large source files do
/// not pin multi-gigabyte allocations.
pub fn hash_file(path: &Path) -> HygeResult<String> {
    let mut file = fs::File::open(path).map_err(|e| io_with_path(e, "open", path))?;
    let mut hasher = Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| io_with_path(e, "read", path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize().into()))
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

/// Imports `path` into the project cache under `out`.
///
/// See module docs for the dispatch rules. The returned `String` is
/// the BLAKE3 hash of the *top-level* asset written to the cache
/// (the mesh hash for glTF, the KTX2 hash for a standalone
/// texture, the source hash for everything else).
///
/// # Errors
///
/// - [`HygeError::AssetNotFound`] when `path` does not exist.
/// - [`HygeError::InvalidArgument`] when `path` is not a regular
///   file.
/// - [`HygeError::Parse`] for malformed glTF / GLB / image input.
/// - [`HygeError::Io`] for any filesystem failure during the
///   cook.
pub fn run(path: &Path, out: &Path) -> HygeResult<String> {
    if !path.exists() {
        return Err(HygeError::asset_not_found(format!(
            "source asset does not exist: {}",
            path.display()
        )));
    }
    let meta = fs::metadata(path).map_err(|e| io_with_path(e, "stat", path))?;
    if meta.is_dir() {
        return Err(HygeError::invalid_argument(format!(
            "source path is a directory, expected a file: {}",
            path.display()
        )));
    }

    fs::create_dir_all(out).map_err(|e| io_with_path(e, "create", out))?;

    if is_gltf_source(path) {
        run_gltf(path, out)
    } else if is_texture_source(path) {
        run_texture(path, out)
    } else if hyge_asset::importer::is_environment_source(path) {
        run_environment(path, out)
    } else {
        run_passthrough(path, out)
    }
}

fn run_gltf(path: &Path, out: &Path) -> HygeResult<String> {
    let db_path = out.join(".hyge.db");
    let opts = ImportOptions {
        source: path.to_path_buf(),
        out_dir: out.to_path_buf(),
        asset_db: Some(db_path),
        compression_mode: CompressionMode::Auto,
        target_format: TargetFormat::Bc7,
        toktx_path: None,
    };
    hyge_log!(info, "importing glTF {}", path.display());
    let report = import_gltf(&opts).map_err(|e| e.0)?;
    hyge_log!(
        info,
        "imported {} -> mesh {}, {} material(s), {} texture(s), {} light(s)",
        path.display(),
        report.mesh_hash,
        report.material_hashes.len(),
        report.texture_hashes.len(),
        report.light_count
    );
    if report.transcode_pending {
        hyge_log!(
            info,
            "  R-036 will transcode {} non-KTX2 texture(s) in a follow-up pass",
            report.texture_hashes.len()
        );
    }
    Ok(report.mesh_hash)
}

fn run_texture(path: &Path, out: &Path) -> HygeResult<String> {
    hyge_log!(info, "importing texture {}", path.display());
    let (width, height, format, pixels) = decode_texture_source(path)?;
    let res: TranscodeResult = transcode::transcode(
        width,
        height,
        format,
        &pixels,
        texture_mime(path),
        out,
        CompressionMode::Auto,
        TargetFormat::Bc7,
        None,
    )?;
    let (vk_format, level_count) = match res.kind {
        hyge_asset::importer::texture::TextureKind::Ktx2 {
            vk_format,
            level_count,
            ..
        } => (vk_format, level_count),
    };
    hyge_log!(
        info,
        "transcoded {} -> {} ({}x{}, vkFormat {}, {} mip levels, used_toktx={})",
        path.display(),
        res.path.display(),
        width,
        height,
        vk_format,
        level_count,
        res.used_toktx
    );
    Ok(res.hash)
}

fn run_environment(path: &Path, out: &Path) -> HygeResult<String> {
    hyge_log!(info, "importing environment {}", path.display());
    let report = hyge_asset::importer::import_environment(path, out)?;
    hyge_log!(
        info,
        "baked {} -> {} (prefilter {}x{} mips {}, irradiance {}x6, brdf_lut {} squared)",
        path.display(),
        report.env_hash,
        report.prefilter_size,
        report.prefilter_size,
        report.prefilter_mips,
        report.irradiance_size,
        report.brdf_lut_size,
    );
    Ok(report.env_hash)
}

fn decode_texture_source(path: &Path) -> HygeResult<(u32, u32, TextureFormat, Vec<u8>)> {
    let bytes = std::fs::read(path).map_err(|e| io_with_path(e, "read", path))?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| HygeError::parse(format!("decode {}: {e}", path.display())))?;
    let rgba8 = img.to_rgba8();
    let (w, h) = rgba8.dimensions();
    Ok((w, h, TextureFormat::R8G8B8A8, rgba8.into_raw()))
}

fn texture_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("exr") => "image/exr",
        _ => "image/png",
    }
}

fn run_passthrough(path: &Path, out: &Path) -> HygeResult<String> {
    hyge_log!(info, "importing (passthrough) {}", path.display());
    let hash = hash_file(path)?;
    let manifest = manifest_path(out, &hash);
    let manifest_contents = format!("hash: {hash}\nsource: {}\n", path.display());
    fs::write(&manifest, manifest_contents).map_err(|e| io_with_path(e, "write", &manifest))?;
    hyge_log!(info, "imported {} -> {}", path.display(), hash);
    Ok(hash)
}

/// Returns the sidecar manifest path for a given BLAKE3 hash under
/// `out`. Used by the passthrough code path.
pub fn manifest_path(out: &Path, hash: &str) -> PathBuf {
    out.join(format!("{hash}.source-path"))
}

/// Returns the canonical cooked-assets directory under a project
/// root.
pub fn default_cook_dir(project: &Path) -> PathBuf {
    project.join(ASSETS_COOK_DIR)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "hyge-tools-r036-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&p).expect("create tempdir");
        p
    }

    fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = fs::File::create(&p).expect("create");
        f.write_all(bytes).expect("write");
        p
    }

    #[test]
    fn is_gltf_source_recognises_gltf_and_glb() {
        assert!(is_gltf_source(Path::new("foo.gltf")));
        assert!(is_gltf_source(Path::new("FOO.GLTF")));
        assert!(is_gltf_source(Path::new("bar.glb")));
        assert!(!is_gltf_source(Path::new("foo.png")));
        assert!(!is_gltf_source(Path::new("foo")));
    }

    #[test]
    fn is_texture_source_recognises_png_jpg_jpeg_exr() {
        assert!(is_texture_source(Path::new("foo.png")));
        assert!(is_texture_source(Path::new("FOO.PNG")));
        assert!(is_texture_source(Path::new("bar.jpg")));
        assert!(is_texture_source(Path::new("bar.jpeg")));
        assert!(is_texture_source(Path::new("hdr.exr")));
        assert!(!is_texture_source(Path::new("foo.gltf")));
        assert!(!is_texture_source(Path::new("foo.lua")));
        assert!(!is_texture_source(Path::new("foo")));
    }

    #[test]
    fn run_rejects_missing_source() {
        let dir = tempdir("miss");
        let out = dir.join("cook");
        let err = run(&dir.join("nope.gltf"), &out).expect_err("must error");
        assert!(matches!(err, HygeError::AssetNotFound(_)), "got {err:?}");
    }

    #[test]
    fn run_rejects_directory_source() {
        let dir = tempdir("dir");
        let sub = dir.join("a-dir");
        fs::create_dir(&sub).unwrap();
        let out = dir.join("cook");
        let err = run(&sub, &out).expect_err("must error");
        assert!(matches!(err, HygeError::InvalidArgument(_)), "got {err:?}");
    }

    #[test]
    fn run_passthrough_writes_manifest_for_unknown_extension() {
        let dir = tempdir("passthru");
        let out = dir.join("cook");
        let src = write_file(&dir, "scene.lua", b"print('hi')");
        let hash = run(&src, &out).expect("passthrough must succeed");
        let body = fs::read_to_string(manifest_path(&out, &hash)).expect("manifest readable");
        assert!(body.contains(&format!("hash: {hash}")));
        assert!(body.contains("source:"));
        assert!(body.contains("scene.lua"));
    }

    #[test]
    fn run_texture_writes_real_ktx2_from_png_source() {
        // Build a tiny 4x4 RGBA PNG in memory: red, green, blue, white.
        // We use the `image` crate's RgbaImage + save() round-trip to
        // produce a real PNG on disk that the import path can decode.
        let img = image::RgbaImage::from_fn(4, 4, |x, y| match (x, y) {
            (0, 0) => image::Rgba([255, 0, 0, 255]),
            (1, 0) => image::Rgba([0, 255, 0, 255]),
            (0, 1) => image::Rgba([0, 0, 255, 255]),
            _ => image::Rgba([255, 255, 255, 255]),
        });
        let dir = tempdir("png");
        let src = dir.join("rgb.png");
        img.save(&src).expect("save png");
        let out = dir.join("cook");
        let hash = run(&src, &out).expect("PNG import must succeed");
        // The cache contains a real KTX2 file with the content-
        // addressed filename.
        let ktx2_path = out.join(format!("{hash}.ktx2"));
        assert!(
            ktx2_path.is_file(),
            "KTX2 file missing: {}",
            ktx2_path.display()
        );
        let raw = std::fs::read(&ktx2_path).unwrap();
        // KTX2 magic.
        assert_eq!(
            &raw[0..12],
            &hyge_asset::importer::texture::KTX2_MAGIC,
            "imported file must be a real KTX2 container"
        );
        // 4x4 -> 3 levels (4, 2, 1).
        let level_count = u32::from_le_bytes(raw[24..28].try_into().unwrap());
        assert_eq!(level_count, 3);
    }
}
