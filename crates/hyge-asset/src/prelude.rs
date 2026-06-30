//! Convenience re-exports for `hyge-asset`.

pub use crate::{
    asset_types::{
        material_upload_task, mesh_upload_task, texture_upload_task, MaterialAsset, MeshAsset,
        TextureAsset, KTX2_MAGIC,
    },
    gpu_upload::{
        GpuUploadPayload, GpuUploadResult, GpuUploadTask, MaterialUploadData, MeshUploadData,
        TextureUploadData,
    },
    importer::{import_gltf, ImportError, ImportOptions, ImportReport},
    server::{AssetServer, GpuResourceKind, LoadedAssetGpu},
    Asset, AssetDb, AssetId, AssetPlugin, AssetResolver, FileWatcher, Handle, LoadContext,
    LoadedAsset, ReloadQueue,
};
