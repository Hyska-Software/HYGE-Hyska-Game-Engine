//! Hyge scene: glTF loader, prefab system, instancing extraction, and the
//! canonical ECS component catalog.
//!
//! Owns the `Prefab` / `PrefabNode` / `ComponentOverride` types and the
//! `.hyge-prefab` / `.hyge-world` serialization formats. Performs meshlet
//! bake via `meshopt` during import (delegated to the importer in `hyge-tools`).
//!
//! See `docs/architecture.md` §6.6 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-035, R-060..R-064.
