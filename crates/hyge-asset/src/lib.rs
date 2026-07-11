//! Hyge asset: the content-addressed asset DB, loader registry, and hot-reload
//! pipeline.
//!
//! Every asset is identified by its BLAKE3 hash (`AssetId = [u8; 32]`). The
//! `AssetDb` (SQLite) maps hashes to on-disk cache paths and records the
//! dependency graph. The `FileWatcher` (via `notify`) detects FS events and
//! triggers re-imports on `AsyncComputeTaskPool`.
//!
//! R-037 adds the asset server and the GPU upload path:
//! - [`AssetServer`] holds the loaded-asset table; resolves
//!   [`Handle<A>`] to [`LoadedAsset<A>`] with bindless ids.
//! - [`GpuUploadTask`] runs on
//!   `AsyncComputeTaskPool`, creates the wgpu buffer/texture, and
//!   registers the result in the
//!   [`hyge_render::bindless::BindlessTable`].
//!
//! R-038 adds the runtime asset types:
//! - [`MeshAsset`] — a marker for `.hyge-mesh` files; loads to
//!   `MeshData`.
//! - [`MaterialAsset`] — a marker for `.hyge-mat` files; loads to
//!   `MaterialData`.
//! - [`TextureAsset`] — a marker for `.ktx2` files; loads to
//!   `Vec<u8>` (raw KTX2 bytes).
//!
//! See `docs/architecture.md` §6.5 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-030..R-038.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod asset;
pub mod asset_types;
pub mod context;
pub mod db;
pub mod gpu_upload;
pub mod handle;
pub mod importer;
pub mod plugin;
pub mod prelude;
pub mod server;
pub mod watcher;

pub use asset::{Asset, AssetId};
pub use asset_types::{
    material_upload_task, mesh_upload_task, texture_upload_task, MaterialAsset, MeshAsset,
    TextureAsset, KTX2_MAGIC,
};
pub use context::LoadContext;
pub use db::{AssetDb, AssetRecord, DependencyEdge};
pub use gpu_upload::{
    GpuUploadPayload, GpuUploadResult, GpuUploadTask, MaterialUploadData, MeshUploadData,
    TextureUploadData,
};
pub use handle::{Handle, LoadedAsset};
pub use plugin::AssetPlugin;
pub use server::{AssetServer, GpuResourceKind, LoadedAssetGpu};
pub use watcher::{AssetResolver, FileWatcher, ReloadQueue};
