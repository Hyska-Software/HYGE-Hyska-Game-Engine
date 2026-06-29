//! `hyge-tools import` — cook a single source asset into the project cache.
//!
//! Stage 1 of the import pipeline (R-033). The detailed format-specific
//! conversion (glTF → `.hyge-mesh`, textures → `.ktx2`, meshlet bake)
//! is implemented in R-034..R-037. This module handles the parts that
//! do not depend on a specific asset format: hashing the source,
//! creating the output directory, and writing a sidecar manifest that
//! later cook steps can read.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use blake3::Hasher;

use hyge_core::hyge_log;
use hyge_core::result::{HygeError, HygeResult};

use crate::cmd::ASSETS_COOK_DIR;

/// Hashes the file at `path` with BLAKE3 and returns the 64-character
/// lowercase hex digest.
///
/// Reads the file in 64 KiB chunks so very large source files do not
/// blow the stack or pin a multi-gigabyte allocation in memory.
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

/// Cooks a single source asset at `path` into the project cache under
/// `out`.
///
/// Concretely, this:
///
/// 1. Verifies the source path exists and is a regular file
///    (otherwise [`HygeError::AssetNotFound`]).
/// 2. Hashes the file contents with BLAKE3.
/// 3. Ensures the output directory exists, creating it if missing.
/// 4. Writes a sidecar `<hash>.source-path` manifest under
///    `<out>/<hash>.source-path` containing the source path. This is
///    the deterministic, hash-named sidecar that later cook steps
///    (R-034..R-037) will read to decide which format-specific
///    pipeline to run.
///
/// The hex hash is also returned from this function so callers
/// (notably [`crate::cmd::cook::run`]) can log a summary without
/// re-reading the file.
///
/// # Errors
///
/// - [`HygeError::AssetNotFound`] when `path` does not exist.
/// - [`HygeError::InvalidArgument`] when `path` exists but is not a
///   regular file.
/// - [`HygeError::Io`] for any filesystem failure (open, read, mkdir,
///   write).
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

    hyge_log!(info, "importing {}", path.display());

    let hash = hash_file(path)?;
    fs::create_dir_all(out).map_err(|e| io_with_path(e, "create", out))?;

    let manifest = manifest_path(out, &hash);
    let manifest_contents = format!("hash: {hash}\nsource: {}\n", path.display());
    fs::write(&manifest, manifest_contents).map_err(|e| io_with_path(e, "write", &manifest))?;

    hyge_log!(info, "imported {} -> {}", path.display(), hash);
    Ok(hash)
}

/// Returns the sidecar manifest path for a given BLAKE3 hash under
/// `out`.
///
/// The convention is `<out>/<hash>.source-path`. The hash is a
/// 64-character lowercase hex string; see [`hash_file`].
pub fn manifest_path(out: &Path, hash: &str) -> PathBuf {
    out.join(format!("{hash}.source-path"))
}

/// Returns the canonical cooked-assets directory under a project root.
///
/// Exposed so callers can avoid re-typing the constant from
/// [`crate::cmd`].
pub fn default_cook_dir(project: &Path) -> PathBuf {
    project.join(ASSETS_COOK_DIR)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn write_source(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = fs::File::create(&p).expect("create source");
        f.write_all(bytes).expect("write source");
        p
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let unique = format!(
            "hyge-tools-r033-{}-{}",
            std::process::id(),
            hex_lower(
                &blake3::hash(format!("{:?}-{:?}", std::time::SystemTime::now(), base).as_bytes())
                    .into()
            )
        );
        let p = base.join(unique);
        fs::create_dir_all(&p).expect("create tempdir");
        p
    }

    #[test]
    fn hash_file_is_deterministic_and_hex() {
        let dir = tempdir();
        let p = write_source(&dir, "a.bin", b"hello world");
        let h1 = hash_file(&p).expect("hash");
        let h2 = hash_file(&p).expect("hash");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64, "BLAKE3 hex digest is 64 chars");
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_file_matches_known_blake3_of_hello_world() {
        // blake3("hello world") =
        //   d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24
        let dir = tempdir();
        let p = write_source(&dir, "a.bin", b"hello world");
        let h = hash_file(&p).expect("hash");
        assert_eq!(
            h,
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
    }

    #[test]
    fn run_writes_sidecar_manifest_for_existing_file() {
        let dir = tempdir();
        let out = dir.join("cook");
        let src = write_source(&dir, "cube.gltf", b"glTF-binary-stub");

        let hash = run(&src, &out).expect("import must succeed");

        let manifest = manifest_path(&out, &hash);
        let body = fs::read_to_string(&manifest).expect("manifest readable");
        assert!(body.contains(&format!("hash: {hash}")));
        assert!(body.contains("source:"));
        assert!(body.contains("cube.gltf"));
    }

    #[test]
    fn run_rejects_missing_source() {
        let dir = tempdir();
        let out = dir.join("cook");
        let missing = dir.join("does-not-exist.gltf");
        let err = run(&missing, &out).expect_err("missing source must error");
        assert!(matches!(err, HygeError::AssetNotFound(_)), "got {err:?}");
    }

    #[test]
    fn run_rejects_directory_source() {
        let dir = tempdir();
        let sub = dir.join("a-directory");
        fs::create_dir(&sub).expect("mkdir");
        let out = dir.join("cook");
        let err = run(&sub, &out).expect_err("directory source must error");
        assert!(matches!(err, HygeError::InvalidArgument(_)), "got {err:?}");
    }
}
