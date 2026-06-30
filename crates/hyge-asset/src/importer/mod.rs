//! Asset import pipeline.
//!
//! The current phase (R-034..R-036, R-041) covers the glTF 2.0
//! importer, the `meshopt`-baked meshlet pipeline (R-035),
//! the KTX2 transcode (R-036), and the IBL environment bake
//! (R-041). The module is organised so future items (R-037
//! bindless table) can drop into the existing
//! [`crate::importer::gltf::GltfScene`] intermediate
//! representation without changing the public orchestrator in
//! [`import_gltf`].
//!
//! See `docs/architecture.md` §9 for the importer's high-level
//! contract and `docs/roadmap.toml` R-034..R-036 + R-041 for
//! these milestones.

pub mod environment;
pub mod gltf;
pub mod import;
pub mod material;
pub mod mesh;
pub mod meshlet;
pub mod meta;
pub mod texture;
pub mod transcode;

#[cfg(test)]
mod golden;

pub use environment::{import_environment, is_environment_source, EnvironmentImportReport};
pub use import::{import_gltf, ImportError, ImportOptions, ImportReport};
