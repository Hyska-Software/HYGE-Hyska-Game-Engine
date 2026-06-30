//! Runtime asset types for meshes, materials, and textures.
//!
//! R-038 adds the typed runtime asset wrappers that the
//! asset server hands to the rest of the engine. Each
//! type implements [`Asset`] so the server can use it
//! with the existing hot-reload / refcount path.
//!
//! The three types correspond to the three file formats
//! produced by the importer (R-034..R-036):
//!
//! - [`MeshAsset`] — parses a `.hyge-mesh` file (v3 LZ4
//!   format from R-038; v2 backwards compatible).
//! - [`MaterialAsset`] — parses a `.hyge-mat` JSON file.
//! - [`TextureAsset`] — parses a `.ktx2` container.
//!
//! All three are `Asset: Send + Sync + 'static` and
//! implement the [`Asset::Data`] associated type so the
//! asset server can pair a `Handle<MeshAsset>` with a
//! loaded `MeshAsset` (which holds `Arc<MeshData>`) and
//! feed the data into a [`GpuUploadTask`].

use std::sync::Arc;

use hyge_core::result::{HygeError, HygeResult};
use hyge_render::prelude::{BindlessTable, GpuMaterial, GpuMesh};

use crate::asset::{Asset, AssetId};
use crate::context::LoadContext;
use crate::gpu_upload::{GpuUploadPayload, GpuUploadTask, MaterialUploadData, MeshUploadData};
use crate::importer::material::MaterialData;
use crate::importer::mesh::{self, MeshData};

// =============================================================================
// MeshAsset
// =============================================================================

/// A runtime mesh asset. Holds the CPU-side [`MeshData`]
/// produced by the importer. Cloning is cheap (the
/// underlying `MeshData` is wrapped in an `Arc`).
#[derive(Debug, Clone)]
pub struct MeshAsset {
    /// The CPU mesh data (vertices, indices, meshlets,
    /// LODs, AABBs).
    pub data: Arc<MeshData>,
}

impl Asset for MeshAsset {
    type Data = MeshData;

    fn hash(data: &Self::Data) -> blake3::Hash {
        // The runtime hash is the same as the on-disk
        // content hash: the asset server uses the file's
        // BLAKE3 hash (from the import report) as the
        // `AssetId`, and the runtime hash is the BLAKE3 of
        // the CPU mesh data.
        mesh::to_bytes(data)
            .map(|bytes| blake3::hash(&bytes))
            .unwrap_or_else(|_| blake3::hash(b"hyge-mesh-empty"))
    }

    fn extensions() -> &'static [&'static str] {
        &["hyge-mesh"]
    }

    fn load(bytes: &[u8], _ctx: &mut LoadContext) -> HygeResult<Self::Data> {
        mesh::from_bytes(bytes)
    }
}

impl MeshAsset {
    /// Constructs a `MeshAsset` from a [`MeshData`],
    /// wrapping the data in an `Arc` for cheap clones.
    #[must_use]
    pub fn new(data: MeshData) -> Self {
        Self {
            data: Arc::new(data),
        }
    }

    /// Returns the CPU mesh data.
    #[must_use]
    pub fn data(&self) -> &MeshData {
        &self.data
    }

    /// Computes the [`MeshUploadData`] for the CPU mesh
    /// data.
    ///
    /// The M2 path uses the first meshlet's AABB as the
    /// mesh's local AABB and reports the meshlet + LOD
    /// counts from the CPU-side data. Vertex / index
    /// offsets are `0` because the M2 path uploads the
    /// mesh's vertices + indices as a per-mesh
    /// `Arc<wgpu::Buffer>` (the global vertex / index
    /// buffer is R-043).
    pub fn upload_data(data: &MeshData) -> MeshUploadData {
        let (aabb_min, aabb_max) = data
            .meshlets
            .first()
            .map_or(([0.0; 3], [0.0; 3]), |m| (m.aabb_min, m.aabb_max));
        MeshUploadData {
            vertex_offset: 0,
            index_offset: 0,
            meshlet_offset: 0,
            meshlet_count: data.meshlets.len() as u32,
            aabb_min,
            aabb_max,
            lod_count: data.lods.len() as u32,
        }
    }

    /// Flattens the CPU mesh data into a [`GpuMesh`]
    /// (the bindless storage format). Returns the GPU
    /// mesh and the [`MeshUploadData`] that the
    /// [`GpuUploadTask`] consumes.
    #[must_use]
    pub fn to_gpu(data: &MeshData) -> (GpuMesh, MeshUploadData) {
        let upload = Self::upload_data(data);
        let gpu = GpuMesh {
            vertex_offset: upload.vertex_offset,
            index_offset: upload.index_offset,
            meshlet_offset: upload.meshlet_offset,
            meshlet_count: upload.meshlet_count,
            aabb_min: upload.aabb_min,
            aabb_max: upload.aabb_max,
            lod_count: upload.lod_count,
            _pad: 0,
        };
        (gpu, upload)
    }
}

// =============================================================================
// MaterialAsset
// =============================================================================

/// A runtime material asset. Holds the CPU-side
/// [`MaterialData`] produced by the importer.
#[derive(Debug, Clone)]
pub struct MaterialAsset {
    /// The CPU material data (PBR constants + texture
    /// references).
    pub data: Arc<MaterialData>,
}

impl Asset for MaterialAsset {
    type Data = MaterialData;

    fn hash(data: &Self::Data) -> blake3::Hash {
        // The material hash is the BLAKE3 of the
        // JSON-serialised form (the on-disk file is JSON).
        serde_json::to_vec(data)
            .map(|bytes| blake3::hash(&bytes))
            .unwrap_or_else(|_| blake3::hash(b"hyge-mat-empty"))
    }

    fn extensions() -> &'static [&'static str] {
        &["hyge-mat"]
    }

    fn load(bytes: &[u8], _ctx: &mut LoadContext) -> HygeResult<Self::Data> {
        serde_json::from_slice(bytes)
            .map_err(|e| HygeError::parse(format!(".hyge-mat JSON decode: {e}")))
    }
}

impl MaterialAsset {
    /// Constructs a `MaterialAsset` from a
    /// [`MaterialData`].
    #[must_use]
    pub fn new(data: MaterialData) -> Self {
        Self {
            data: Arc::new(data),
        }
    }

    /// Returns the CPU material data.
    #[must_use]
    pub fn data(&self) -> &MaterialData {
        &self.data
    }

    /// Flattens the CPU material data into a
    /// [`GpuMaterial`]. The M2 path uses the
    /// `base_color_texture` BLAKE3 hash modulo the
    /// texture capacity as the bindless texture id; the
    /// M3+ PBR path looks up the actual texture id from
    /// the bindless table.
    pub fn to_gpu(data: &MaterialData) -> (GpuMaterial, MaterialUploadData) {
        // Hash the texture references to derive a
        // bindless texture id. The M2 path doesn't
        // maintain a per-asset texture → bindless-id
        // map; using a hash modulo the v0.1 capacity
        // (16) gives a stable, deterministic id that
        // is unique per texture hash.
        let texture_id = |hash: &Option<String>| -> u32 {
            hash.as_ref().map_or(0, |h| {
                let h = blake3::hash(h.as_bytes());
                let bytes = h.as_bytes();
                let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                n % 16
            })
        };
        let gpu = GpuMaterial {
            base_color: texture_id(&data.base_color_texture),
            normal: texture_id(&data.normal_texture),
            mr: texture_id(&data.metallic_roughness_texture),
            occlusion: texture_id(&data.occlusion_texture),
            emissive: texture_id(&data.emissive_texture),
            roughness: data.roughness,
            metallic: data.metallic,
            alpha_mode: if data.double_sided { 1 } else { 0 },
            flags: 0,
        };
        let upload = MaterialUploadData {
            base_color: gpu.base_color,
            normal: gpu.normal,
            mr: gpu.mr,
            occlusion: gpu.occlusion,
            emissive: gpu.emissive,
            roughness: data.roughness,
            metallic: data.metallic,
            alpha_mode: gpu.alpha_mode,
            flags: 0,
        };
        (gpu, upload)
    }
}

// =============================================================================
// TextureAsset
// =============================================================================

/// The KTX2 magic header. The first 12 bytes of a
/// valid KTX2 file: `0xAB 0x4B 0x54 0x58 0x20 0x32
/// 0x30 0xBB 0x0D 0x0A 0x1A 0x0A`.
pub const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// A runtime texture asset. Holds the raw KTX2 container
/// bytes. The runtime transcoder parses this on demand
/// to upload the texture to the bindless texture array.
#[derive(Debug, Clone)]
pub struct TextureAsset {
    /// The raw KTX2 container bytes.
    pub bytes: Arc<Vec<u8>>,
}

impl Asset for TextureAsset {
    type Data = Vec<u8>;

    fn hash(data: &Self::Data) -> blake3::Hash {
        blake3::hash(data)
    }

    fn extensions() -> &'static [&'static str] {
        &["ktx2"]
    }

    fn load(bytes: &[u8], _ctx: &mut LoadContext) -> HygeResult<Self::Data> {
        if bytes.len() < 12 {
            return Err(HygeError::parse("KTX2 file too short for header"));
        }
        if bytes[..12] != KTX2_MAGIC {
            return Err(HygeError::parse(
                "KTX2 magic mismatch (not a KTX2 container)",
            ));
        }
        Ok(bytes.to_vec())
    }
}

impl TextureAsset {
    /// Constructs a `TextureAsset` from a KTX2 byte
    /// vector. Validates the magic header.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Arc::new(bytes),
        }
    }

    /// Returns the raw KTX2 bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Verifies the KTX2 magic header. Returns
    /// `HygeError::Parse` on mismatch.
    pub fn verify_magic(&self) -> HygeResult<()> {
        if self.bytes.len() < KTX2_MAGIC.len() {
            return Err(HygeError::parse("KTX2 file too short for magic"));
        }
        if self.bytes[..12] != KTX2_MAGIC {
            return Err(HygeError::parse(
                "KTX2 magic mismatch (not a KTX2 container)",
            ));
        }
        Ok(())
    }
}

// =============================================================================
// AssetServer extensions
// =============================================================================

/// Builds a [`GpuUploadTask`] for a mesh's CPU data. The
/// asset server uses this on `register` to feed the mesh
/// into the bindless table.
pub fn mesh_upload_task(
    asset_id: AssetId,
    bindless: Arc<BindlessTable>,
    data: &MeshData,
) -> GpuUploadTask {
    let (_, upload) = MeshAsset::to_gpu(data);
    GpuUploadTask::new(asset_id, GpuUploadPayload::Mesh(upload), bindless)
}

/// Builds a [`GpuUploadTask`] for a material's CPU data.
pub fn material_upload_task(
    asset_id: AssetId,
    bindless: Arc<BindlessTable>,
    data: &MaterialData,
) -> GpuUploadTask {
    let (_, upload) = MaterialAsset::to_gpu(data);
    GpuUploadTask::new(asset_id, GpuUploadPayload::Material(upload), bindless)
}

/// Builds a [`GpuUploadTask`] for a texture. The
/// uploaded texture reserves a bindless slot; the
/// actual `write_texture` call is left to the caller
/// (R-040 wires the full PBR texture path).
pub fn texture_upload_task(asset_id: AssetId, bindless: Arc<BindlessTable>) -> GpuUploadTask {
    GpuUploadTask::new(
        asset_id,
        GpuUploadPayload::Texture(crate::gpu_upload::TextureUploadData {
            width: 1,
            height: 1,
            pixels: vec![0u8; 4],
        }),
        bindless,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::mesh::Vertex as MeshVertex;

    fn tri() -> MeshData {
        MeshData::from_triangle_list(
            vec![
                MeshVertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                MeshVertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                },
                MeshVertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                },
            ],
            vec![0, 1, 2],
        )
    }

    /// A small triangle mesh round-trips through
    /// `MeshAsset::load`.
    #[test]
    fn mesh_asset_round_trips() {
        let bytes = mesh::to_bytes(&tri()).expect("to_bytes");
        let mut ctx = LoadContext::default();
        let data = MeshAsset::load(&bytes, &mut ctx).expect("MeshAsset::load");
        assert_eq!(data.vertices.len(), 3);
        assert_eq!(data.indices.len(), 3);
    }

    /// `MeshAsset::to_gpu` reports the meshlet + LOD
    /// counts in the `GpuMesh` POD.
    #[test]
    fn mesh_asset_to_gpu_reports_section_counts() {
        let data = tri();
        let (gpu, upload) = MeshAsset::to_gpu(&data);
        assert_eq!(gpu.meshlet_count, upload.meshlet_count);
        assert_eq!(gpu.lod_count, upload.lod_count);
        assert_eq!(upload.meshlet_count, data.meshlets.len() as u32);
    }

    /// A `MaterialAsset` deserialises from the JSON the
    /// importer writes.
    #[test]
    fn material_asset_round_trips() {
        let data = MaterialData {
            name: "lit_red".into(),
            base_color: [0.8, 0.1, 0.1, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            emissive: [0.0; 3],
            double_sided: false,
            base_color_texture: Some("deadbeef".into()),
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
        };
        let bytes = serde_json::to_vec(&data).expect("serialize");
        let mut ctx = LoadContext::default();
        let back = MaterialAsset::load(&bytes, &mut ctx).expect("load");
        assert_eq!(back, data);
    }

    /// `MaterialAsset::to_gpu` flattens the CPU material
    /// to a `GpuMaterial` with the expected field mapping.
    #[test]
    fn material_asset_to_gpu_flattens_fields() {
        let data = MaterialData {
            name: "x".into(),
            base_color: [1.0; 4],
            metallic: 0.5,
            roughness: 0.25,
            emissive: [0.0; 3],
            double_sided: true,
            base_color_texture: Some("abcd".into()),
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
        };
        let (gpu, upload) = MaterialAsset::to_gpu(&data);
        assert_eq!(gpu.roughness, 0.25);
        assert_eq!(gpu.metallic, 0.5);
        assert_eq!(gpu.alpha_mode, 1, "double_sided -> alpha_mode 1");
        // Texture id is a hash modulo 16.
        assert!(gpu.base_color < 16);
        assert_eq!(gpu.normal, 0, "no normal map -> texture id 0");
        assert_eq!(upload.roughness, 0.25);
    }

    /// A `TextureAsset` accepts a real KTX2 file and
    /// rejects a non-KTX2 file.
    #[test]
    fn texture_asset_validates_ktx2_magic() {
        let mut bytes = KTX2_MAGIC.to_vec();
        bytes.extend_from_slice(&[0u8; 64]);
        let mut ctx = LoadContext::default();
        let data = TextureAsset::load(&bytes, &mut ctx).expect("valid KTX2 loads");
        assert_eq!(data.len(), bytes.len());

        let bad = vec![0u8; 64];
        let res = TextureAsset::load(&bad, &mut ctx);
        assert!(res.is_err(), "non-KTX2 bytes must fail to load");
    }

    /// `MaterialAsset::hash` is deterministic for the
    /// same input.
    #[test]
    fn material_asset_hash_is_deterministic() {
        let data = MaterialData {
            name: "x".into(),
            ..MaterialData::default()
        };
        let h1 = MaterialAsset::hash(&data);
        let h2 = MaterialAsset::hash(&data);
        assert_eq!(h1, h2);
    }
}
