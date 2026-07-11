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
    importer::{
        import_environment, import_environment_with_config, import_environment_with_config_and_db,
        import_gltf, is_environment_source, EnvironmentImportReport, ImportError, ImportOptions,
        ImportReport,
    },
    server::{AssetServer, GpuResourceKind, LoadedAssetGpu},
    Asset, AssetDb, AssetId, AssetPlugin, AssetRecord, AssetResolver, DependencyEdge, FileWatcher,
    Handle, LoadContext, LoadedAsset, ReloadQueue,
};
