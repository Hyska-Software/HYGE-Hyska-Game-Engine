//! `.hyge-mesh` binary writer.
//!
//! The mesh format is a compact, deterministic little-endian layout:
//!
//! ```text
//! header          (24 bytes)
//!   magic         : u32  = 0x484D4548 ("HMEH")
//!   version       : u32  = 1
//!   meshlet_count : u32
//!   vertex_count  : u32
//!   index_count   : u32
//!   lod_count     : u32
//! vertex_data     (vertex_count * 32 bytes:  position[12] normal[12] uv[8])
//! index_data      (index_count * 4 bytes:     u32 indices)
//! meshlet_index_offsets (meshlet_count * 4 bytes: u32 offset into index_data)
//! meshlet_index_counts  (meshlet_count * 4 bytes: u32 count)
//! meshlet_vertex_indices(meshlet_count * 64 * 4 bytes: u32, padded with 0xFFFF_FFFF)
//! meshlet_aabbs         (meshlet_count * 24 bytes: min[12] max[12])
//! lod_index_offsets (lod_count * 4 bytes: u32 offset into index_data)
//! lod_index_counts  (lod_count * 4 bytes: u32 count)
//! ```
//!
//! The format is intentionally simple: this milestone (R-034) lays
//! down a reproducible binary the runtime can later replace with a
//! meshopt-baked meshlet stream (R-035).

use std::fs;
use std::io::Write;
use std::path::Path;

use hyge_core::result::HygeResult;

const MAGIC: u32 = 0x484D_4548; // "HMEH" little-endian
const VERSION: u32 = 1;
const MAX_MESHLET_VERTS: usize = 64;

/// A triangle mesh ready to be serialized to `.hyge-mesh`.
///
/// The intermediate representation stays close to the on-disk layout so
/// the writer is straightforward and easy to test.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    /// Interleaved per-vertex data: `position[3] + normal[3] + uv[2]`.
    pub vertices: Vec<Vertex>,
    /// Triangle list, indices into `vertices`.
    pub indices: Vec<u32>,
    /// Meshlet partitioning of `indices`. Each meshlet owns a
    /// contiguous slice of `indices` plus a vertex table of at most
    /// 64 unique vertices.
    pub meshlets: Vec<Meshlet>,
    /// LOD chain, expressed as index slices into `indices`. Empty
    /// when LODs are not generated.
    pub lods: Vec<LodRange>,
}

impl MeshData {
    /// Builds a single meshlet covering the entire index buffer.
    ///
    /// Used as a deterministic placeholder until R-035 replaces this
    /// with `meshopt_buildMeshlets`. Produces stable, reproducible
    /// output for a given `(vertices, indices)` pair.
    pub fn from_triangle_list(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        let meshlets = vec![Meshlet {
            index_offset: 0,
            index_count: indices.len() as u32,
            vertex_indices: collect_unique_vertices(&indices, MAX_MESHLET_VERTS),
            aabb_min: vertices
                .iter()
                .map(|v| v.position)
                .fold([f32::INFINITY; 3], |mut a, p| {
                    a[0] = a[0].min(p[0]);
                    a[1] = a[1].min(p[1]);
                    a[2] = a[2].min(p[2]);
                    a
                }),
            aabb_max: vertices.iter().map(|v| v.position).fold(
                [f32::NEG_INFINITY; 3],
                |mut a, p| {
                    a[0] = a[0].max(p[0]);
                    a[1] = a[1].max(p[1]);
                    a[2] = a[2].max(p[2]);
                    a
                },
            ),
        }];
        Self {
            vertices,
            indices,
            meshlets,
            lods: Vec::new(),
        }
    }
}

/// One mesh vertex. Stored as plain `f32` arrays for the milestone;
/// the R-035 path will quantize to `i16` once `meshopt` is wired in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// XYZ position.
    pub position: [f32; 3],
    /// XYZ normal.
    pub normal: [f32; 3],
    /// UV texcoord.
    pub uv: [f32; 2],
}

/// A single meshlet: a contiguous range of indices plus a bounded
/// vertex table. Bounds are stored so the runtime can do early
/// culling without re-touching the index data.
#[derive(Debug, Clone, PartialEq)]
pub struct Meshlet {
    /// Start of this meshlet's slice in [`MeshData::indices`].
    pub index_offset: u32,
    /// Number of indices in this meshlet (multiple of 3).
    pub index_count: u32,
    /// Up to [`MAX_MESHLET_VERTS`] unique vertex indices used by
    /// this meshlet. Padded with `u32::MAX` to a fixed width so the
    /// on-disk layout is trivial to seek into.
    pub vertex_indices: Vec<u32>,
    /// AABB minimum corner.
    pub aabb_min: [f32; 3],
    /// AABB maximum corner.
    pub aabb_max: [f32; 3],
}

/// An LOD level expressed as a sub-range of the full index buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodRange {
    /// Start of the LOD slice in [`MeshData::indices`].
    pub index_offset: u32,
    /// Number of indices in this LOD slice (multiple of 3).
    pub index_count: u32,
}

fn collect_unique_vertices(indices: &[u32], cap: usize) -> Vec<u32> {
    let mut seen: Vec<u32> = Vec::with_capacity(cap);
    for &i in indices {
        if !seen.contains(&i) {
            seen.push(i);
            if seen.len() == cap {
                break;
            }
        }
    }
    seen.resize(cap, u32::MAX);
    seen
}

/// Writes `mesh` to `out` as a `.hyge-mesh` file. The output is
/// deterministic for a given input.
///
/// # Errors
///
/// Returns [`hyge_core::result::HygeError::Io`] on filesystem failure.
pub fn write(path: &Path, mesh: &MeshData) -> HygeResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create parent directory"))?;
    }
    let mut f = fs::File::create(path).map_err(io_error("create mesh file"))?;

    let header = [
        MAGIC,
        VERSION,
        mesh.meshlets.len() as u32,
        mesh.vertices.len() as u32,
        mesh.indices.len() as u32,
        mesh.lods.len() as u32,
    ];
    f.write_all(bytemuck::cast_slice(&header))
        .map_err(io_error("write mesh header"))?;

    for v in &mesh.vertices {
        let data: [f32; 8] = [
            v.position[0],
            v.position[1],
            v.position[2],
            v.normal[0],
            v.normal[1],
            v.normal[2],
            v.uv[0],
            v.uv[1],
        ];
        f.write_all(bytemuck::cast_slice(&data))
            .map_err(io_error("write vertex"))?;
    }

    f.write_all(bytemuck::cast_slice(&mesh.indices))
        .map_err(io_error("write indices"))?;

    for m in &mesh.meshlets {
        f.write_all(m.index_offset.to_le_bytes().as_slice())
            .map_err(io_error("write meshlet offset"))?;
    }
    for m in &mesh.meshlets {
        f.write_all(m.index_count.to_le_bytes().as_slice())
            .map_err(io_error("write meshlet count"))?;
    }
    for m in &mesh.meshlets {
        for &vi in &m.vertex_indices {
            f.write_all(vi.to_le_bytes().as_slice())
                .map_err(io_error("write meshlet vertex index"))?;
        }
    }
    for m in &mesh.meshlets {
        let mins = m.aabb_min;
        let maxs = m.aabb_max;
        let data: [f32; 6] = [mins[0], mins[1], mins[2], maxs[0], maxs[1], maxs[2]];
        f.write_all(bytemuck::cast_slice(&data))
            .map_err(io_error("write meshlet aabb"))?;
    }

    for lod in &mesh.lods {
        f.write_all(lod.index_offset.to_le_bytes().as_slice())
            .map_err(io_error("write lod offset"))?;
    }
    for lod in &mesh.lods {
        f.write_all(lod.index_count.to_le_bytes().as_slice())
            .map_err(io_error("write lod count"))?;
    }

    f.sync_all().map_err(io_error("sync mesh file"))?;
    Ok(())
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> hyge_core::result::HygeError {
    move |e| hyge_core::result::HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tri() -> MeshData {
        MeshData::from_triangle_list(
            vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                },
            ],
            vec![0, 1, 2],
        )
    }

    #[test]
    fn write_produces_stable_deterministic_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "hyge-asset-mesh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let p1 = dir.join("a.hyge-mesh");
        let p2 = dir.join("b.hyge-mesh");

        let m = tri();
        write(&p1, &m).unwrap();
        write(&p2, &m).unwrap();

        let a = fs::read(&p1).unwrap();
        let b = fs::read(&p2).unwrap();
        assert_eq!(a, b, "writer must be deterministic for same input");
        assert!(a.len() > 24, "file must contain the header plus payload");
        assert_eq!(
            &a[0..4],
            &0x484D_4548u32.to_le_bytes(),
            "magic must be present in LE bytes"
        );
    }

    #[test]
    fn round_trip_header_values() {
        let dir = std::env::temp_dir().join(format!(
            "hyge-asset-mesh-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("rt.hyge-mesh");
        let m = tri();
        write(&p, &m).unwrap();

        let bytes = fs::read(&p).unwrap();
        let header: [u32; 6] = bytemuck::cast_slice::<u8, u32>(&bytes[0..24])
            .try_into()
            .unwrap();
        assert_eq!(header[0], MAGIC);
        assert_eq!(header[1], VERSION);
        assert_eq!(header[2], 1, "one meshlet");
        assert_eq!(header[3], 3, "three vertices");
        assert_eq!(header[4], 3, "three indices");
        assert_eq!(header[5], 0, "no LODs");
    }
}
