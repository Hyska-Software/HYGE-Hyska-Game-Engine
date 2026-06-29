//! Asset import pipeline.
//!
//! The current phase (R-034) covers the glTF 2.0 importer. The module is
//! organised so future items (R-035 meshlet bake, R-036 KTX2 transcode,
//! R-037 bindless table) can drop into the existing [`GltfScene`]
//! intermediate representation without changing the public orchestrator
//! in [`import_gltf`].
//!
//! See `docs/architecture.md` §9 for the importer's high-level
//! contract and `docs/roadmap.toml` R-034 for this milestone.

pub mod gltf;
pub mod import;
pub mod material;
pub mod mesh;
pub mod meta;
pub mod texture;

#[cfg(test)]
mod golden;

pub use import::{import_gltf, ImportError, ImportOptions, ImportReport};
