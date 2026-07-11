//! Project discovery and path validation.

use std::fs;
use std::path::{Path, PathBuf};

use hyge_asset::Asset;
use hyge_core::result::{HygeError, HygeResult};
use hyge_scene::PrefabAsset;

use crate::lock::ProjectLock;

/// Canonical project root and its exclusive editor lock.
#[derive(Debug)]
pub struct Project {
    /// Canonical project directory.
    pub root: PathBuf,
    /// Lock held while the project is open.
    _lock: ProjectLock,
    /// Non-fatal project diagnostics.
    pub diagnostics: Vec<String>,
}

impl Project {
    /// Opens and locks a project directory.
    pub fn open(path: &Path) -> HygeResult<Self> {
        let root = path
            .canonicalize()
            .map_err(|error| HygeError::invalid_argument(format!("project path: {error}")))?;
        if !root.is_dir() {
            return Err(HygeError::invalid_argument(
                "project path is not a directory",
            ));
        }
        let lock = ProjectLock::acquire(&root)
            .map_err(|error| HygeError::invalid_argument(format!("project lock: {error}")))?;
        let mut diagnostics = Vec::new();
        if !root.join("assets").is_dir() {
            diagnostics.push("assets directory is missing".to_owned());
        }
        Ok(Self {
            root,
            _lock: lock,
            diagnostics,
        })
    }

    /// Resolves a scene path and rejects paths outside the project.
    pub fn scene_path(&self, path: &Path) -> HygeResult<PathBuf> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|error| HygeError::invalid_argument(format!("scene path: {error}")))?;
        if !canonical.starts_with(&self.root)
            || canonical.extension().and_then(|e| e.to_str()) != Some("hyge-world")
        {
            return Err(HygeError::invalid_argument(
                "scene must be a .hyge-world inside the project",
            ));
        }
        Ok(canonical)
    }

    /// Loads every project prefab into the supplied scene library.
    pub fn load_prefabs(&self, library: &mut hyge_scene::PrefabLibrary) -> HygeResult<usize> {
        let mut count = 0;
        visit_files(&self.root, &mut |path| {
            if path.extension().and_then(|e| e.to_str()) != Some("hyge-prefab") {
                return Ok(());
            }
            let bytes = fs::read(path)?;
            let prefab = PrefabAsset::load(&bytes, &mut hyge_asset::LoadContext::default())?;
            library.insert(prefab);
            count += 1;
            Ok(())
        })?;
        Ok(count)
    }
}

fn visit_files(root: &Path, callback: &mut impl FnMut(&Path) -> HygeResult<()>) -> HygeResult<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(".hyge") {
            continue;
        }
        if path.is_dir() {
            visit_files(&path, callback)?;
        } else {
            callback(&path)?;
        }
    }
    Ok(())
}
