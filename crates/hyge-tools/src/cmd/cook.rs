//! `hyge-tools cook` — cook every source asset under a project.
//!
//! Walks `<project>/assets/source/` recursively and dispatches
//! [`super::import::run`] per file, writing the sidecar manifests
//! into the cooked-asset cache directory. Format-specific pipelines
//! (mesh bake, KTX2 transcode, meshopt) are layered in by R-034..R-037.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hyge_core::hyge_log;
use hyge_core::result::{HygeError, HygeResult};

use crate::cmd::import;
use crate::cmd::{ASSETS_COOK_DIR, ASSETS_SOURCE_DIR};

/// Per-extension import counts accumulated during a cook run.
///
/// BTreeMap keeps the keys sorted for deterministic test output and
/// stable log lines.
type CountByExt = BTreeMap<String, usize>;

/// Cooks every source asset under `<project>/assets/source/` into
/// the project cache, defaulting to `<project>/assets/cook/` when
/// `out` is `None`.
///
/// Returns a [`CookSummary`] with the total number of files processed
/// and a per-extension breakdown.
///
/// # Errors
///
/// - [`HygeError::InvalidArgument`] when the project directory does
///   not exist, or the canonical `assets/source/` subdirectory is
///   missing.
/// - [`HygeError::Io`] for filesystem failures while walking the
///   source tree.
/// - Any error propagated from [`super::import::run`].
pub fn run(project: &Path, out: Option<&Path>) -> HygeResult<CookSummary> {
    if !project.is_dir() {
        return Err(HygeError::invalid_argument(format!(
            "project path is not a directory: {}",
            project.display()
        )));
    }

    let source_dir = project.join(ASSETS_SOURCE_DIR);
    if !source_dir.is_dir() {
        return Err(HygeError::invalid_argument(format!(
            "project has no {} directory: {}",
            ASSETS_SOURCE_DIR,
            project.display()
        )));
    }

    let out_dir: std::path::PathBuf = match out {
        Some(o) => o.to_path_buf(),
        None => project.join(ASSETS_COOK_DIR),
    };
    fs::create_dir_all(&out_dir).map_err(|e| {
        HygeError::Io(std::io::Error::other(format!(
            "create cook dir {}: {e}",
            out_dir.display()
        )))
    })?;

    hyge_log!(info, "cooking project at {}", project.display());
    hyge_log!(info, "  source: {}", source_dir.display());
    hyge_log!(info, "  output: {}", out_dir.display());

    let mut counts: CountByExt = BTreeMap::new();
    let mut total: usize = 0;

    let entries = collect_files(&source_dir)?;
    for src in entries {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match import::run(&src, &out_dir) {
            Ok(_hash) => {
                *counts.entry(ext).or_insert(0) += 1;
                total += 1;
            }
            Err(e) => {
                hyge_log!(warn, "failed to import {}: {e}", src.display());
            }
        }
    }

    let summary = CookSummary {
        total,
        by_ext: counts,
    };
    hyge_log!(info, "cooked {} asset(s)", summary.total);
    for (ext, n) in &summary.by_ext {
        let label = if ext.is_empty() {
            "<no-ext>"
        } else {
            ext.as_str()
        };
        hyge_log!(info, "  {label}: {n}");
    }
    Ok(summary)
}

fn collect_files(root: &Path) -> HygeResult<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| {
            HygeError::Io(std::io::Error::other(format!(
                "read_dir {}: {e}",
                dir.display()
            )))
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                HygeError::Io(std::io::Error::other(format!(
                    "read_dir entry in {}: {e}",
                    dir.display()
                )))
            })?;
            let p = entry.path();
            let ft = entry.file_type().map_err(|e| {
                HygeError::Io(std::io::Error::other(format!(
                    "file_type for {}: {e}",
                    p.display()
                )))
            })?;
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Aggregate result of a [`run`] invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookSummary {
    /// Total number of source files successfully imported.
    pub total: usize,
    /// Per-extension (lowercased) count of files imported.
    pub by_ext: CountByExt,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use super::*;

    fn write_file(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        let mut f = fs::File::create(path).expect("create");
        f.write_all(bytes).expect("write");
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let unique = format!(
            "hyge-tools-r033-cook-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let p = base.join(unique);
        fs::create_dir_all(&p).expect("create tempdir");
        p
    }

    #[test]
    fn cook_walks_assets_source_and_writes_manifests() {
        // Non-glTF sources go through the passthrough path
        // (the glTF path is covered by hyge-asset's golden
        // tests; see crates/hyge-asset/src/importer/golden.rs).
        let project = tempdir();
        let src_dir = project.join(ASSETS_SOURCE_DIR);
        write_file(&src_dir.join("scene.lua"), b"lua-scene");
        write_file(&src_dir.join("nested/anim.lua"), b"lua-anim");
        write_file(&src_dir.join("readme.txt"), b"text");

        let summary = run(&project, None).expect("cook must succeed");
        assert_eq!(summary.total, 3);
        assert_eq!(summary.by_ext.get("lua"), Some(&2));
        assert_eq!(summary.by_ext.get("txt"), Some(&1));

        let cook_dir = project.join(ASSETS_COOK_DIR);
        let manifests: Vec<_> = fs::read_dir(&cook_dir)
            .expect("cook dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".source-path"))
            .collect();
        assert_eq!(manifests.len(), 3, "one manifest per imported file");
    }

    #[test]
    fn cook_rejects_project_without_assets_source() {
        let project = tempdir();
        let err = run(&project, None).expect_err("missing source dir must error");
        assert!(matches!(err, HygeError::InvalidArgument(_)), "got {err:?}");
    }

    #[test]
    fn cook_rejects_non_directory_project() {
        let dir = tempdir();
        let not_a_dir = dir.join("file.txt");
        write_file(&not_a_dir, b"hi");
        let err = run(&not_a_dir, None).expect_err("file as project must error");
        assert!(matches!(err, HygeError::InvalidArgument(_)), "got {err:?}");
    }

    #[test]
    fn cook_respects_out_override() {
        let project = tempdir();
        let src_dir = project.join(ASSETS_SOURCE_DIR);
        write_file(&src_dir.join("a.lua"), b"lua");

        let out = tempdir().join("custom-cook");
        let summary = run(&project, Some(&out)).expect("cook with --out must succeed");
        assert_eq!(summary.total, 1);
        assert!(out.is_dir(), "custom output dir must be created");
    }
}
