//! Loading context shared by asset loaders.

use std::path::{Path, PathBuf};

use crate::asset::AssetId;

/// Per-load context passed to [`crate::asset::Asset::load`].
///
/// The R-030 skeleton only tracks declared dependencies. Future roadmap items
/// extend this type with DB lookup, importer options, and diagnostic spans.
#[derive(Clone, Debug, Default)]
pub struct LoadContext {
    dependencies: Vec<AssetId>,
    source_path: Option<PathBuf>,
}

impl LoadContext {
    /// Creates a context for a source path.
    pub fn with_source_path(path: impl Into<PathBuf>) -> Self {
        Self {
            dependencies: Vec::new(),
            source_path: Some(path.into()),
        }
    }

    /// Records that the asset currently being loaded depends on `id`.
    pub fn add_dependency(&mut self, id: AssetId) {
        self.dependencies.push(id);
    }

    /// Returns dependencies declared by the loader.
    pub fn dependencies(&self) -> &[AssetId] {
        &self.dependencies
    }

    /// Returns the source path associated with this load, when known.
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_tracks_source_path_and_dependencies() {
        let id = AssetId::from(blake3::hash(b"dependency"));
        let mut ctx = LoadContext::with_source_path("models/cube.gltf");
        ctx.add_dependency(id);

        assert_eq!(ctx.source_path(), Some(Path::new("models/cube.gltf")));
        assert_eq!(ctx.dependencies(), &[id]);
    }
}
