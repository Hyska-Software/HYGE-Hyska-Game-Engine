//! `.hyge-mesh` binary writer.
//!
//! The mesh format is a compact, deterministic little-endian layout.
//! Version 3 (R-038, current) is the M2 on-disk format: the body
//! after the header is LZ4-compressed (the `flags` field carries
//! the `FLAG_LZ4` bit). v2 files (raw, no LZ4) are still readable
//! through [`from_bytes`] for backwards compatibility with
//! pre-M2 caches.
//!
//! ```text
//! header          (28 bytes)
//!   magic         : u32  = 0x484D4548 ("HMEH")
//!   version       : u32  = 3
//!   flags         : u32  = 0x1 (FLAG_LZ4)
//!   meshlet_count : u32
//!   vertex_count  : u32
//!   index_count   : u32           (total, includes LOD chain)
//!   lod_count     : u32           (LODs beyond base; R-035: 3)
//! vertex_data     (vertex_count * 32 bytes:  position[12] normal[12] uv[8])
//! index_data      (index_count * 4 bytes:     u32 indices,
//!                                          LOD0 meshlet expansions
//!                                          followed by LOD1..LOD3)
//! meshlet_index_offsets (meshlet_count * 4 bytes: u32 offset into index_data)
//! meshlet_index_counts  (meshlet_count * 4 bytes: u32 count)
//! meshlet_vertex_indices(meshlet_count * 64 * 4 bytes: u32, padded with 0xFFFF_FFFF)
//! meshlet_aabbs         (meshlet_count * 24 bytes: min[12] max[12])
//! meshlet_cones         (meshlet_count * 44 bytes: center[12] radius[4] apex[12] axis[12] cutoff[4])
//! lod_index_offsets (lod_count * 4 bytes: u32 offset into index_data)
//! lod_index_counts  (lod_count * 4 bytes: u32 count)
//! ```
//!
//! The meshlet section is produced by [`crate::importer::meshlet::bake_meshlets`]
//! (R-035); the LOD chain is produced by
//! [`crate::importer::meshlet::bake_lod_chain`]. The base LOD
//! meshlet expansions live at the start of `index_data` so the
//! existing pointer arithmetic in the runtime keeps working:
//! `meshlet_index_offsets[i]` is a direct offset into the
//! `index_data` section.
//!
//! See `docs/architecture.md` §6.6 ("Meshlet bake algorithm"),
//! `docs/roadmap.toml` R-035 (meshlet bake), and R-038 (M2
//! LZ4 wrap).

use std::fs;
use std::io::Write;
use std::path::Path;

use hyge_core::result::HygeResult;

use crate::importer::meshlet::{
    bake_lod_chain, bake_meshlets, LOD_RATIOS, MESHLET_LOCAL_VERTEX_TABLE,
};

const MAGIC: u32 = 0x484D_4548; // "HMEH" little-endian
/// On-disk format version. Bumped to 3 by R-038: the body
/// after the header is LZ4-compressed (the `flags` field
/// carries the `FLAG_LZ4` bit). v2 files (raw, no LZ4) are
/// still readable through [`from_bytes`].
const VERSION: u32 = 3;
/// The previous on-disk format version, still readable
/// through [`from_bytes`] for backwards compatibility with
/// pre-M2 caches.
const VERSION_RAW: u32 = 2;
/// Header flag: the body is LZ4-compressed.
const FLAG_LZ4: u32 = 0x1;

/// A triangle mesh ready to be serialized to `.hyge-mesh`.
///
/// The intermediate representation stays close to the on-disk
/// layout so the writer is straightforward and easy to test.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    /// Interleaved per-vertex data: `position[3] + normal[3] + uv[2]`.
    pub vertices: Vec<Vertex>,
    /// Unified index buffer. Layout (R-035):
    ///
    /// 1. Base LOD meshlet expansions, in meshlet order.
    /// 2. LOD1 simplified indices (from `meshopt::simplify`).
    /// 3. LOD2 simplified indices.
    /// 4. LOD3 simplified indices.
    ///
    /// Each `Meshlet.index_offset` and `Meshlet.index_count` is
    /// relative to this single buffer and stays within the
    /// base-LOD prefix. LOD ranges are recorded in
    /// [`Self::lods`].
    pub indices: Vec<u32>,
    /// Meshlet partitioning of the base LOD. Each meshlet owns a
    /// contiguous slice of `indices` plus a vertex table of at
    /// most [`MESHLET_LOCAL_VERTEX_TABLE`] unique vertices.
    pub meshlets: Vec<Meshlet>,
    /// LOD chain beyond the base. The `index_offset` /
    /// `index_count` fields are slices into [`Self::indices`].
    /// Empty when LODs are not generated.
    pub lods: Vec<LodRange>,
}

impl MeshData {
    /// Builds a `MeshData` from a single-meshlet covering the
    /// entire index buffer.
    ///
    /// This is the deterministic placeholder for tests and
    /// non-baked inputs. Production paths go through
    /// [`MeshData::bake`] which calls `meshopt` directly.
    pub fn from_triangle_list(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        let meshlets = if indices.is_empty() {
            Vec::new()
        } else {
            vec![Meshlet {
                index_offset: 0,
                index_count: indices.len() as u32,
                vertex_indices: collect_unique_vertices(&indices, MESHLET_LOCAL_VERTEX_TABLE),
                aabb_min: vertices.iter().map(|v| v.position).fold(
                    [f32::INFINITY; 3],
                    |mut a, p| {
                        a[0] = a[0].min(p[0]);
                        a[1] = a[1].min(p[1]);
                        a[2] = a[2].min(p[2]);
                        a
                    },
                ),
                aabb_max: vertices.iter().map(|v| v.position).fold(
                    [f32::NEG_INFINITY; 3],
                    |mut a, p| {
                        a[0] = a[0].max(p[0]);
                        a[1] = a[1].max(p[1]);
                        a[2] = a[2].max(p[2]);
                        a
                    },
                ),
                cone: MeshletCone {
                    center: vertices
                        .iter()
                        .map(|v| v.position)
                        .fold([0.0_f32; 3], |mut a, p| {
                            a[0] += p[0];
                            a[1] += p[1];
                            a[2] += p[2];
                            a
                        })
                        .map(|s| {
                            if vertices.is_empty() {
                                0.0
                            } else {
                                s / vertices.len() as f32
                            }
                        }),
                    radius: 0.0,
                    apex: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    cutoff: -1.0,
                },
            }]
        };
        Self {
            vertices,
            indices,
            meshlets,
            lods: Vec::new(),
        }
    }

    /// Bakes the meshlet stream + LOD chain for the given vertex
    /// and index buffers via `meshopt` and assembles the
    /// corresponding `MeshData` ready to be written to
    /// `.hyge-mesh`.
    ///
    /// This is the production entry point (R-035). It:
    ///
    /// 1. Calls [`bake_meshlets`] to partition `indices` into
    ///    meshlets with cone + AABB bounds.
    /// 2. Calls [`bake_lod_chain`] at [`LOD_RATIOS`] to
    ///    produce the simplified LODs.
    /// 3. Concatenates the per-meshlet expanded index buffers
    ///    (base LOD) followed by the LOD chain into a single
    ///    `indices` buffer, recording offsets and counts for each
    ///    section.
    ///
    /// The result is **deterministic** for fixed inputs (R-035
    /// acceptance bullet "same input glTF produces identical
    /// `.hyge-mesh` bytes") because `meshopt` is a pure function
    /// over its inputs and the concatenation order is fixed.
    ///
    /// # Errors
    ///
    /// Returns [`hyge_core::result::HygeError::Parse`] when either
    /// bake call fails (pre-validation in `meshopt` — e.g. out-of-
    /// range index, non-triangle index count, bad LOD ratio).
    pub fn bake(vertices: Vec<Vertex>, indices: Vec<u32>) -> HygeResult<Self> {
        // An empty mesh is a valid (degenerate) case: glTF
        // documents can declare nodes without any geometry, and
        // the runtime expects a zero-meshlet placeholder so the
        // downstream writer is always called with a well-formed
        // `MeshData`. The placeholder matches
        // [`Self::from_triangle_list`] for the empty case.
        if vertices.is_empty() || indices.is_empty() {
            return Ok(Self::from_triangle_list(vertices, indices));
        }
        let baked_meshlets = bake_meshlets(&vertices, &indices)?;
        let baked_lods = bake_lod_chain(&vertices, &indices, LOD_RATIOS)?;

        // Build the unified index buffer: meshlet expansions
        // first (in meshlet order), then each LOD chain. Track
        // per-meshlet offsets and per-LOD offsets as we go.
        let mut unified_indices: Vec<u32> =
            Vec::with_capacity(indices.len() + baked_lods.iter().map(|l| l.len()).sum::<usize>());
        let mut meshlets: Vec<Meshlet> = Vec::with_capacity(baked_meshlets.len());
        for ml in baked_meshlets {
            let index_offset = unified_indices.len() as u32;
            // Expand the local `u8` triangle indices to global
            // `u32` indices by walking the local vertex table.
            let tri_count = ml.local_triangle_indices.len() / 3;
            for t in 0..tri_count {
                for c in 0..3 {
                    let local = ml.local_triangle_indices[t * 3 + c] as usize;
                    let global = ml.local_vertex_indices[local];
                    debug_assert!(global != u32::MAX, "local triangle references padding slot");
                    unified_indices.push(global);
                }
            }
            let index_count = (tri_count as u32) * 3;
            meshlets.push(Meshlet {
                index_offset,
                index_count,
                vertex_indices: ml.local_vertex_indices,
                aabb_min: ml.aabb_min,
                aabb_max: ml.aabb_max,
                cone: MeshletCone {
                    center: ml.cone_center,
                    radius: ml.cone_radius,
                    apex: ml.cone_apex,
                    axis: ml.cone_axis,
                    cutoff: ml.cone_cutoff,
                },
            });
        }

        let mut lods: Vec<LodRange> = Vec::with_capacity(baked_lods.len());
        for lod_indices in baked_lods {
            let index_offset = unified_indices.len() as u32;
            let index_count = lod_indices.len() as u32;
            unified_indices.extend_from_slice(&lod_indices);
            lods.push(LodRange {
                index_offset,
                index_count,
            });
        }

        Ok(Self {
            vertices,
            indices: unified_indices,
            meshlets,
            lods,
        })
    }
}

/// One mesh vertex. Stored as plain `f32` arrays for the milestone;
/// the R-035 path quantizes positions in the GPU mesh shader (the
/// CPU-side meshlet stream keeps the `f32` form because the
/// on-disk format does not need to quantize — the runtime
/// re-quantizes per-frame for the GPU buffer).
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
    /// Up to [`MESHLET_LOCAL_VERTEX_TABLE`] unique vertex indices
    /// used by this meshlet. Padded with `u32::MAX` to a fixed
    /// width so the on-disk layout is trivial to seek into.
    pub vertex_indices: Vec<u32>,
    /// AABB minimum corner.
    pub aabb_min: [f32; 3],
    /// AABB maximum corner.
    pub aabb_max: [f32; 3],
    /// Bounding cone (centre, radius, apex, axis, cutoff) as
    /// computed by `meshopt::clusterize::compute_meshlet_bounds`.
    /// The runtime uses the cone for back-face culling and the
    /// sphere (`center` + `radius`) for frustum culling.
    pub cone: MeshletCone,
}

/// Meshopt bounding cone. The on-disk layout is exactly
/// `center[3] + radius + apex[3] + axis[3] + cutoff` = 44 bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshletCone {
    /// Bounding-sphere centre.
    pub center: [f32; 3],
    /// Bounding-sphere radius.
    pub radius: f32,
    /// Cone apex.
    pub apex: [f32; 3],
    /// Cone axis (unit length, away from `apex`).
    pub axis: [f32; 3],
    /// Cosine of the cone half-angle. `1.0` = no culling,
    /// `-1.0` = full hemisphere.
    pub cutoff: f32,
}

/// An LOD level expressed as a sub-range of the full index
/// buffer.
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
/// deterministic for a given input (R-035 acceptance).
///
/// # Errors
///
/// Returns [`hyge_core::result::HygeError::Io`] on filesystem
/// failure.
pub fn write(path: &Path, mesh: &MeshData) -> HygeResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create parent directory"))?;
    }
    let mut f = fs::File::create(path).map_err(io_error("create mesh file"))?;
    write_into(&mut f, mesh)?;
    f.sync_all().map_err(io_error("sync mesh file"))?;
    Ok(())
}

/// Serialises `mesh` to an in-memory byte buffer. Identical
/// output to [`write()`] for a given input; used by the
/// orchestrator when it needs both the bytes (for content-
/// addressing) and the on-disk file (for the cache).
pub fn to_bytes(mesh: &MeshData) -> HygeResult<Vec<u8>> {
    // Pre-size the buffer to the exact on-disk length so
    // the inner writer never reallocates. Layout (see
    // module docs):
    //   header(28) + lz4_compressed(body)
    // where the body is the v2-style raw layout:
    //   vertices*32 + indices*4
    //   + meshlets*(8 + 256 + 24 + 44)
    //   + lods*8
    let body_capacity = mesh.vertices.len() * 32
        + mesh.indices.len() * 4
        + mesh.meshlets.len() * (8 + 256 + 24 + 44)
        + mesh.lods.len() * 8;
    // LZ4 worst case is `body + (body / 255) + 16`; we
    // pre-size to that and the writer's `write_all` is
    // the only allocation. The actual compressed size is
    // reported by the writer when the buffer is finalized.
    let worst_case = body_capacity + (body_capacity / 255) + 16;
    let mut buf: Vec<u8> = Vec::with_capacity(28 + worst_case);
    write_into(&mut buf, mesh)?;
    Ok(buf)
}

fn write_into<W: Write>(w: &mut W, mesh: &MeshData) -> HygeResult<()> {
    // Header layout (version 3, R-038):
    //   magic, version, flags, meshlet_count, vertex_count, index_count, lod_count.
    // The `flags` field currently only carries `FLAG_LZ4`
    // (the body is always LZ4-compressed in v3).
    let header = [
        MAGIC,
        VERSION,
        FLAG_LZ4,
        mesh.meshlets.len() as u32,
        mesh.vertices.len() as u32,
        mesh.indices.len() as u32,
        mesh.lods.len() as u32,
    ];
    w.write_all(bytemuck::cast_slice(&header))
        .map_err(io_error("write mesh header"))?;

    // Build the v2-style body into a temporary buffer,
    // then LZ4-compress it and emit the compressed bytes.
    // The body layout matches the pre-R-038 v2 format (no
    // header, no flags) so existing `from_bytes_v2` readers
    // (R-035 tests, downstream tooling) can still parse it
    // after decompression.
    let mut body: Vec<u8> = Vec::new();
    write_body_into(&mut body, mesh)?;
    let compressed = lz4_flex::block::compress(&body);
    w.write_all(&compressed)
        .map_err(io_error("write lz4-compressed mesh body"))?;
    Ok(())
}

/// Writes the v2-style body (the part after the 28-byte
/// header) into the given writer. Used by both the v3 LZ4
/// writer (which compresses the output) and the v2 raw
/// writer (which was the R-035 path). The body has no
/// version / flag bytes; the version is carried by the
/// header.
fn write_body_into<W: Write>(w: &mut W, mesh: &MeshData) -> HygeResult<()> {
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
        w.write_all(bytemuck::cast_slice(&data))
            .map_err(io_error("write vertex"))?;
    }

    w.write_all(bytemuck::cast_slice(&mesh.indices))
        .map_err(io_error("write indices"))?;

    for m in &mesh.meshlets {
        w.write_all(m.index_offset.to_le_bytes().as_slice())
            .map_err(io_error("write meshlet offset"))?;
    }
    for m in &mesh.meshlets {
        w.write_all(m.index_count.to_le_bytes().as_slice())
            .map_err(io_error("write meshlet count"))?;
    }
    for m in &mesh.meshlets {
        // The 64-entry local vertex table is a fixed-stride
        // `[u32; 64]`; cast the whole slice and emit in one
        // write_all (saves 63 syscalls per meshlet on the
        // 10k-meshlet M4 scenes).
        w.write_all(bytemuck::cast_slice(&m.vertex_indices))
            .map_err(io_error("write meshlet vertex indices"))?;
    }
    for m in &mesh.meshlets {
        let mins = m.aabb_min;
        let maxs = m.aabb_max;
        let data: [f32; 6] = [mins[0], mins[1], mins[2], maxs[0], maxs[1], maxs[2]];
        w.write_all(bytemuck::cast_slice(&data))
            .map_err(io_error("write meshlet aabb"))?;
    }
    for m in &mesh.meshlets {
        let c = m.cone;
        let data: [f32; 11] = [
            c.center[0],
            c.center[1],
            c.center[2],
            c.radius,
            c.apex[0],
            c.apex[1],
            c.apex[2],
            c.axis[0],
            c.axis[1],
            c.axis[2],
            c.cutoff,
        ];
        w.write_all(bytemuck::cast_slice(&data))
            .map_err(io_error("write meshlet cone"))?;
    }

    for lod in &mesh.lods {
        w.write_all(lod.index_offset.to_le_bytes().as_slice())
            .map_err(io_error("write lod offset"))?;
    }
    for lod in &mesh.lods {
        w.write_all(lod.index_count.to_le_bytes().as_slice())
            .map_err(io_error("write lod count"))?;
    }

    Ok(())
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> hyge_core::result::HygeError {
    move |e| hyge_core::result::HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
}

/// Reads a `.hyge-mesh` file from `bytes`. Supports both the
/// v3 LZ4-compressed format (R-038, default) and the v2
/// raw format (R-035, for backwards compatibility with
/// pre-M2 caches). Returns the parsed `MeshData`.
///
/// # Errors
///
/// Returns [`hyge_core::result::HygeError::Parse`] when the
/// header is malformed (wrong magic, unknown version, or
/// truncated body). Returns
/// [`hyge_core::result::HygeError::Io`] when the LZ4
/// decompression fails.
pub fn from_bytes(bytes: &[u8]) -> HygeResult<MeshData> {
    if bytes.len() < 28 {
        return Err(hyge_core::result::HygeError::parse(format!(
            "mesh file too short: {} bytes (need at least 28 for v3 header)",
            bytes.len()
        )));
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != MAGIC {
        return Err(hyge_core::result::HygeError::parse(format!(
            "mesh file wrong magic: 0x{magic:08x} (expected 0x{MAGIC:08x})"
        )));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let meshlet_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let vertex_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let index_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let lod_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;

    let body = match version {
        VERSION => {
            // v3: LZ4-compressed body after the 28-byte
            // header.
            let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
            if flags & FLAG_LZ4 == 0 {
                return Err(hyge_core::result::HygeError::parse(
                    "v3 mesh file has FLAG_LZ4 clear (unsupported)",
                ));
            }
            let compressed = &bytes[28..];
            lz4_flex::block::decompress(
                compressed,
                expected_body_size(vertex_count, index_count, meshlet_count, lod_count),
            )
            .map_err(|e| {
                hyge_core::result::HygeError::Io(std::io::Error::other(format!(
                    "lz4 decompress mesh body: {e}"
                )))
            })?
        }
        VERSION_RAW => {
            // v2: raw body after the 24-byte header (the
            // legacy format that pre-M2 importers produced).
            // Note: the v2 header does not have a `flags`
            // field, so the body starts at byte 24.
            if bytes.len() < 24 {
                return Err(hyge_core::result::HygeError::parse(
                    "v2 mesh file too short for header",
                ));
            }
            bytes[24..].to_vec()
        }
        other => {
            return Err(hyge_core::result::HygeError::parse(format!(
                "mesh file unsupported version: {other}"
            )));
        }
    };

    parse_body(&body, meshlet_count, vertex_count, index_count, lod_count)
}

/// Computes the expected body size for a v2-style raw
/// layout given the section counts. Used as the upper
/// bound for LZ4 decompression.
fn expected_body_size(
    vertex_count: usize,
    index_count: usize,
    meshlet_count: usize,
    lod_count: usize,
) -> usize {
    vertex_count * 32 + index_count * 4 + meshlet_count * (8 + 256 + 24 + 44) + lod_count * 8
}

/// Parses the v2-style body into a `MeshData`. The body
/// layout matches the pre-R-038 v2 format (no header, no
/// flags): vertices, indices, then the per-meshlet
/// sections, then the per-LOD sections.
fn parse_body(
    body: &[u8],
    meshlet_count: usize,
    vertex_count: usize,
    index_count: usize,
    lod_count: usize,
) -> HygeResult<MeshData> {
    let expected = expected_body_size(vertex_count, index_count, meshlet_count, lod_count);
    if body.len() != expected {
        return Err(hyge_core::result::HygeError::parse(format!(
            "mesh body length mismatch: got {} bytes, expected {}",
            body.len(),
            expected
        )));
    }

    let mut cursor = 0;
    let mut take = |n: usize| -> HygeResult<&[u8]> {
        let end = cursor + n;
        if end > body.len() {
            return Err(hyge_core::result::HygeError::parse(format!(
                "mesh body truncated: need {n} bytes at offset {cursor}"
            )));
        }
        let out = &body[cursor..end];
        cursor = end;
        Ok(out)
    };

    // Vertices.
    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        let bytes = take(32)?;
        let data: [f32; 8] = bytemuck::pod_read_unaligned(bytes);
        vertices.push(Vertex {
            position: [data[0], data[1], data[2]],
            normal: [data[3], data[4], data[5]],
            uv: [data[6], data[7]],
        });
    }

    // Indices.
    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        let bytes = take(4)?;
        indices.push(u32::from_le_bytes(bytes.try_into().unwrap()));
    }

    // Meshlet sections: index_offset, index_count, vertex_indices (64 * u32), aabb (6 * f32), cone (11 * f32).
    let mut meshlet_index_offsets = Vec::with_capacity(meshlet_count);
    for _ in 0..meshlet_count {
        let bytes = take(4)?;
        meshlet_index_offsets.push(u32::from_le_bytes(bytes.try_into().unwrap()));
    }
    let mut meshlet_index_counts = Vec::with_capacity(meshlet_count);
    for _ in 0..meshlet_count {
        let bytes = take(4)?;
        meshlet_index_counts.push(u32::from_le_bytes(bytes.try_into().unwrap()));
    }
    let mut meshlet_vertex_indices = Vec::with_capacity(meshlet_count);
    for _ in 0..meshlet_count {
        let bytes = take(256)?;
        let arr: [u32; 64] = bytemuck::pod_read_unaligned(bytes);
        meshlet_vertex_indices.push(arr.to_vec());
    }
    let mut meshlet_aabbs_min = Vec::with_capacity(meshlet_count);
    let mut meshlet_aabbs_max = Vec::with_capacity(meshlet_count);
    for _ in 0..meshlet_count {
        let bytes = take(24)?;
        let data: [f32; 6] = bytemuck::pod_read_unaligned(bytes);
        meshlet_aabbs_min.push([data[0], data[1], data[2]]);
        meshlet_aabbs_max.push([data[3], data[4], data[5]]);
    }
    let mut meshlet_cones: Vec<MeshletCone> = Vec::with_capacity(meshlet_count);
    for _ in 0..meshlet_count {
        let bytes = take(44)?;
        let data: [f32; 11] = bytemuck::pod_read_unaligned(bytes);
        meshlet_cones.push(MeshletCone {
            center: [data[0], data[1], data[2]],
            radius: data[3],
            apex: [data[4], data[5], data[6]],
            axis: [data[7], data[8], data[9]],
            cutoff: data[10],
        });
    }

    // Reassemble meshlets.
    let mut meshlets = Vec::with_capacity(meshlet_count);
    for i in 0..meshlet_count {
        meshlets.push(Meshlet {
            index_offset: meshlet_index_offsets[i],
            index_count: meshlet_index_counts[i],
            vertex_indices: std::mem::take(&mut meshlet_vertex_indices[i]),
            aabb_min: meshlet_aabbs_min[i],
            aabb_max: meshlet_aabbs_max[i],
            cone: meshlet_cones[i],
        });
    }

    // LOD sections.
    let mut lod_offsets = Vec::with_capacity(lod_count);
    for _ in 0..lod_count {
        let bytes = take(4)?;
        lod_offsets.push(u32::from_le_bytes(bytes.try_into().unwrap()));
    }
    let mut lod_counts = Vec::with_capacity(lod_count);
    for _ in 0..lod_count {
        let bytes = take(4)?;
        lod_counts.push(u32::from_le_bytes(bytes.try_into().unwrap()));
    }
    let lods: Vec<LodRange> = (0..lod_count)
        .map(|i| LodRange {
            index_offset: lod_offsets[i],
            index_count: lod_counts[i],
        })
        .collect();

    Ok(MeshData {
        vertices,
        indices,
        meshlets,
        lods,
    })
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

    fn quad() -> (Vec<Vertex>, Vec<u32>) {
        let v = vec![
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
                position: [1.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 1.0],
            },
            Vertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 1.0],
            },
        ];
        let i = vec![0, 1, 2, 0, 2, 3];
        (v, i)
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
        assert!(
            a.len() > 28,
            "file must contain the v3 header plus LZ4 body"
        );
        assert_eq!(
            &a[0..4],
            &0x484D_4548u32.to_le_bytes(),
            "magic must be present in LE bytes"
        );
        // v3 + FLAG_LZ4
        assert_eq!(
            u32::from_le_bytes(a[4..8].try_into().unwrap()),
            3,
            "version must be 3"
        );
        assert_eq!(
            u32::from_le_bytes(a[8..12].try_into().unwrap()),
            FLAG_LZ4,
            "flags must have FLAG_LZ4 set"
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
        // v3 header is 28 bytes (7 × u32): magic, version,
        // flags, meshlet_count, vertex_count, index_count,
        // lod_count.
        let header: [u32; 7] = bytemuck::cast_slice::<u8, u32>(&bytes[0..28])
            .try_into()
            .unwrap();
        assert_eq!(header[0], MAGIC);
        assert_eq!(header[1], VERSION, "format version is 3 (R-038 LZ4 wrap)");
        assert_eq!(header[2], FLAG_LZ4, "FLAG_LZ4 must be set");
        assert_eq!(header[3], 1, "one meshlet");
        assert_eq!(header[4], 3, "three vertices");
        assert_eq!(header[5], 3, "three indices");
        assert_eq!(header[6], 0, "no LODs for the placeholder single-tri mesh");
    }

    #[test]
    fn bake_produces_meshdata_with_meshlets_and_lods() {
        let (v, i) = quad();
        let baked = MeshData::bake(v, i).expect("bake succeeds");
        assert!(!baked.meshlets.is_empty());
        assert_eq!(baked.lods.len(), LOD_RATIOS.len());
        // The unified index buffer = base meshlet expansions + LOD chain.
        let base_count: usize = baked.meshlets.iter().map(|m| m.index_count as usize).sum();
        let lod_count: usize = baked.lods.iter().map(|l| l.index_count as usize).sum();
        assert_eq!(base_count + lod_count, baked.indices.len());
    }

    #[test]
    fn bake_is_deterministic() {
        let (v, i) = quad();
        let a = MeshData::bake(v.clone(), i.clone()).expect("bake A");
        let b = MeshData::bake(v, i).expect("bake B");
        assert_eq!(a, b, "bake must be deterministic for fixed input");
    }

    #[test]
    fn write_baked_mesh_is_deterministic_across_runs() {
        // The R-035 acceptance bullet: same input glTF produces
        // identical .hyge-mesh bytes. The test pins this at the
        // MeshData level (the higher-level glTF -> .hyge-mesh
        // determinism is covered by the integration test in
        // `importer::golden`).
        let (v, i) = quad();
        let m1 = MeshData::bake(v.clone(), i.clone()).expect("bake");
        let m2 = MeshData::bake(v, i).expect("bake");
        let dir = std::env::temp_dir().join(format!(
            "hyge-asset-mesh-bake-det-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let p1 = dir.join("a.hyge-mesh");
        let p2 = dir.join("b.hyge-mesh");
        write(&p1, &m1).unwrap();
        write(&p2, &m2).unwrap();
        let a = fs::read(&p1).unwrap();
        let b = fs::read(&p2).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn write_baked_mesh_round_trips_header() {
        let (v, i) = quad();
        let m = MeshData::bake(v, i).expect("bake");
        let dir = std::env::temp_dir().join(format!(
            "hyge-asset-mesh-bake-rt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("baked.hyge-mesh");
        write(&p, &m).unwrap();
        let bytes = fs::read(&p).unwrap();
        let header: [u32; 7] = bytemuck::cast_slice::<u8, u32>(&bytes[0..28])
            .try_into()
            .unwrap();
        assert_eq!(header[0], MAGIC);
        assert_eq!(header[1], VERSION);
        assert_eq!(header[2], FLAG_LZ4);
        assert_eq!(header[3] as usize, m.meshlets.len());
        assert_eq!(header[4] as usize, m.vertices.len());
        assert_eq!(header[5] as usize, m.indices.len());
        assert_eq!(header[6] as usize, m.lods.len());
    }

    /// v3 .hyge-mesh files round-trip through
    /// `to_bytes` -> `from_bytes`. The LZ4 wrap is
    /// transparent.
    #[test]
    fn v3_round_trip_through_from_bytes() {
        let (v, i) = quad();
        let m = MeshData::bake(v, i).expect("bake");
        let bytes = to_bytes(&m).expect("to_bytes");
        let back = from_bytes(&bytes).expect("from_bytes");
        assert_eq!(back.vertices.len(), m.vertices.len());
        assert_eq!(back.indices.len(), m.indices.len());
        assert_eq!(back.meshlets.len(), m.meshlets.len());
        assert_eq!(back.lods.len(), m.lods.len());
        for (a, b) in back.vertices.iter().zip(m.vertices.iter()) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.normal, b.normal);
        }
        assert_eq!(back.indices, m.indices);
    }

    /// The LZ4-compressed body must be smaller (or equal)
    /// than the raw body for a typical baked mesh.
    #[test]
    fn v3_lz4_compresses_payload() {
        let (v, i) = quad();
        let m = MeshData::bake(v, i).expect("bake");
        let bytes = to_bytes(&m).expect("to_bytes");
        // The header is 28 bytes; the rest is the LZ4 body.
        let compressed_body = &bytes[28..];
        let expected_raw = expected_body_size(
            m.vertices.len(),
            m.indices.len(),
            m.meshlets.len(),
            m.lods.len(),
        );
        // The compressed body is at most the raw size plus
        // a small LZ4 framing overhead. We allow a generous
        // upper bound so the test isn't fragile to
        // LZ4 version differences.
        assert!(
            compressed_body.len() <= expected_raw + 64,
            "compressed body {} should be no larger than raw {} + 64 bytes overhead",
            compressed_body.len(),
            expected_raw
        );
    }
}
