//! Asset import pipeline.
//!
//! The current phase (R-034..R-036) covers the glTF 2.0
//! importer, the `meshopt`-baked meshlet pipeline (R-035),
//! and the KTX2 transcode (R-036). The module is organised so
//! future items (R-037 bindless table) can drop into the
//! existing [`crate::importer::gltf::GltfScene`] intermediate
//! representation without changing the public orchestrator in
//! [`import_gltf`].
//!
//! See `docs/architecture.md` §9 for the importer's high-level
//! contract and `docs/roadmap.toml` R-034..R-036 for these
//! milestones.

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

pub use import::{import_gltf, ImportError, ImportOptions, ImportReport};
