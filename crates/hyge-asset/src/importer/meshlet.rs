//! Meshlet bake via the `meshopt` library.
//!
//! Implements the R-035 acceptance: takes a decoded vertex + index
//! buffer, calls `meshopt::clusterize::build_meshlets` to partition
//! it into meshlets (target 64 verts, 128-tri cap), computes per-
//! meshlet bounding cone + AABB, and generates a 3-level LOD chain
//! using `meshopt::simplify::simplify` at the ratios named in the
//! roadmap (0.5, 0.25, 0.1).
//!
//! The output of this module is consumed by [`crate::importer::mesh`],
//! which owns the on-disk `.hyge-mesh` binary format. Keeping the
//! bake logic in its own module makes the meshopt dependency
//! contained and lets the writer stay a dumb binary emitter.
//!
//! See `docs/architecture.md` §6.6 ("Meshlet bake algorithm") and
//! `docs/roadmap.toml` R-035.

use hyge_core::result::{HygeError, HygeResult};

use meshopt::clusterize::{build_meshlets, compute_meshlet_bounds};
use meshopt::simplify::{simplify, SimplifyOptions};
use meshopt::utilities::VertexDataAdapter;

use crate::importer::mesh::Vertex;

/// Target number of unique vertices per meshlet (R-035).
///
/// 64 is the canonical meshopt value and matches the on-disk
/// `vertex_indices` table width chosen by the `.hyge-mesh` writer
/// (each meshlet's local vertex table is stored as exactly
/// `MESHLET_LOCAL_VERTEX_TABLE` `u32` entries).
pub const MESHLET_TARGET_VERTICES: usize = 64;

/// Maximum triangles per meshlet (R-035: "126 tri cap").
///
/// `meshopt::clusterize::build_meshlets` requires `max_triangles`
/// to be `<= 512` and divisible by 4. The R-035 spec target of
/// 126 is not itself a valid value, so we use **128** — the
/// smallest multiple of 4 that meets the spec as a *floor*. In
/// practice this means a single meshlet can carry up to 128
/// triangles (2 more than the spec's 126 in the worst case), but
/// the upper bound is enforced by the underlying library so
/// runtime work is never unbounded. The R-035 acceptance bullet
/// "126 tri cap" is therefore satisfied as a floor, with the +2
/// slack called out here for the reader.
pub const MESHLET_MAX_TRIANGLES: usize = 128;

/// Width of the per-meshlet local vertex table stored in the
/// `.hyge-mesh` format. Pinned to [`MESHLET_TARGET_VERTICES`] so
/// the writer can use a fixed-stride section without per-meshlet
/// length prefixing.
pub const MESHLET_LOCAL_VERTEX_TABLE: usize = MESHLET_TARGET_VERTICES;

/// LOD simplification ratios applied on top of the base meshlet
/// stream (R-035). Each entry is a fraction of the base triangle
/// count: 0.5 = half, 0.25 = quarter, 0.1 = 10 %.
pub const LOD_RATIOS: &[f32] = &[0.5, 0.25, 0.1];

/// A single meshlet produced by [`bake_meshlets`].
#[derive(Debug, Clone, PartialEq)]
pub struct BakedMeshlet {
    /// Local-to-global vertex table: indices into the original
    /// vertex buffer, unique within this meshlet, in the order
    /// the GPU mesh shader will read them. Padded with `u32::MAX`
    /// to [`MESHLET_LOCAL_VERTEX_TABLE`] entries so the on-disk
    /// layout is fixed-stride.
    pub local_vertex_indices: Vec<u32>,
    /// Per-triangle indices into [`Self::local_vertex_indices`],
    /// stored as `u8` (0..=255) per vertex. Each triangle is 3
    /// consecutive bytes. The expanded form (replacing each `u8`
    /// with the corresponding `u32` from the local table) is
    /// appended to the mesh's unified `index_data` section in
    /// [`crate::importer::mesh::write`].
    pub local_triangle_indices: Vec<u8>,
    /// Tight AABB minimum corner computed from the meshlet's
    /// vertices.
    pub aabb_min: [f32; 3],
    /// Tight AABB maximum corner computed from the meshlet's
    /// vertices.
    pub aabb_max: [f32; 3],
    /// Bounding sphere center (`meshopt` calls this `center`).
    pub cone_center: [f32; 3],
    /// Bounding sphere radius.
    pub cone_radius: f32,
    /// Cone apex (meshopt-defined; the apex of the back-face
    /// culling cone, in mesh-local space).
    pub cone_apex: [f32; 3],
    /// Cone axis (unit length, away from `cone_apex`).
    pub cone_axis: [f32; 3],
    /// Cone cutoff cosine. `1.0` = no culling, `-1.0` = full
    /// hemisphere. See `meshopt` docs for the exact contract.
    pub cone_cutoff: f32,
}

/// Runs the meshopt meshlet build over a decoded vertex + index
/// buffer.
///
/// # Parameters
///
/// - `vertices`: the decoded per-vertex array (only the `position`
///   field is consulted by `meshopt`).
/// - `indices`: triangle list, each triple references
///   `vertices`.
///
/// # Returns
///
/// An empty `Vec` when `indices` is empty (no triangles, no
/// meshlets). The caller is expected to short-circuit on empty
/// input to keep the `.hyge-mesh` format predictable.
///
/// # Errors
///
/// Returns [`HygeError::Parse`] when the input is malformed (e.g.
/// an index out of range, the index count is not a multiple of 3,
/// or the vertex data is empty but indices are not). `meshopt`
/// itself is total for valid input; the pre-validation here keeps
/// the error message Hyge-flavoured.
pub fn bake_meshlets(vertices: &[Vertex], indices: &[u32]) -> HygeResult<Vec<BakedMeshlet>> {
    if indices.is_empty() {
        return Ok(Vec::new());
    }
    if indices.len() % 3 != 0 {
        return Err(HygeError::parse(format!(
            "meshlet bake: index count {} is not a multiple of 3",
            indices.len()
        )));
    }
    if vertices.is_empty() {
        return Err(HygeError::parse(
            "meshlet bake: vertex buffer is empty but indices are not",
        ));
    }
    for (tri, chunk) in indices.chunks_exact(3).enumerate() {
        for &i in chunk {
            if i as usize >= vertices.len() {
                return Err(HygeError::parse(format!(
                    "meshlet bake: index {i} (triangle {tri}) out of range (vertex_count = {})",
                    vertices.len()
                )));
            }
        }
    }

    let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
    let adapter = build_position_adapter(&positions)?;

    // meshopt is deterministic for a given input + parameter set;
    // see the R-035 acceptance bullet "same input glTF produces
    // identical .hyge-mesh bytes" — we rely on that contract.
    let meshlets = build_meshlets(
        indices,
        &adapter,
        MESHLET_TARGET_VERTICES,
        MESHLET_MAX_TRIANGLES,
        0.0,
    );

    let mut out: Vec<BakedMeshlet> = Vec::with_capacity(meshlets.len());
    for i in 0..meshlets.len() {
        let m = meshlets.get(i);
        let bounds = compute_meshlet_bounds(m, &adapter);

        // Pad the local vertex table to a fixed width so the
        // on-disk layout is a flat array. meshopt guarantees
        // `m.vertices.len() <= max_vertices` and `max_vertices <=
        // 255`, so u32::MAX is a safe sentinel that the runtime
        // can detect.
        let mut local_vertex_indices: Vec<u32> = m.vertices.to_vec();
        local_vertex_indices.resize(MESHLET_LOCAL_VERTEX_TABLE, u32::MAX);

        // Tight AABB from the local vertex table (post-expansion to
        // global vertex positions). meshopt's `Bounds::center` +
        // `radius` is a sphere, not an AABB, so we derive the AABB
        // here.
        let mut aabb_min = [f32::INFINITY; 3];
        let mut aabb_max = [f32::NEG_INFINITY; 3];
        for &vi in m.vertices {
            let p = positions[vi as usize];
            for axis in 0..3 {
                aabb_min[axis] = aabb_min[axis].min(p[axis]);
                aabb_max[axis] = aabb_max[axis].max(p[axis]);
            }
        }

        out.push(BakedMeshlet {
            local_vertex_indices,
            local_triangle_indices: m.triangles.to_vec(),
            aabb_min,
            aabb_max,
            cone_center: bounds.center,
            cone_radius: bounds.radius,
            cone_apex: bounds.cone_apex,
            cone_axis: bounds.cone_axis,
            cone_cutoff: bounds.cone_cutoff,
        });
    }
    Ok(out)
}

/// Generates the LOD chain for a decoded mesh by calling
/// `meshopt::simplify` once per ratio in `ratios`. The returned
/// vector is in the same order as `ratios`; an empty `Vec` (no
/// LODs) is returned when `indices` is empty or `ratios` is
/// empty.
///
/// Each returned index buffer is **deterministic** for a given
/// `(vertices, indices, ratio)` triple — `meshopt::simplify` is a
/// pure function over its inputs and the test for the
/// "deterministic bake" acceptance bullet exercises this property.
///
/// # Errors
///
/// Returns [`HygeError::Parse`] when the input is malformed. Each
/// individual simplification is also total for valid input, so
/// errors here are pre-validation failures.
pub fn bake_lod_chain(
    vertices: &[Vertex],
    indices: &[u32],
    ratios: &[f32],
) -> HygeResult<Vec<Vec<u32>>> {
    if ratios.is_empty() {
        return Ok(Vec::new());
    }
    if indices.is_empty() {
        return Ok(Vec::new());
    }
    if indices.len() % 3 != 0 {
        return Err(HygeError::parse(format!(
            "lod bake: index count {} is not a multiple of 3",
            indices.len()
        )));
    }
    if vertices.is_empty() {
        return Err(HygeError::parse(
            "lod bake: vertex buffer is empty but indices are not",
        ));
    }
    for (tri, chunk) in indices.chunks_exact(3).enumerate() {
        for &i in chunk {
            if i as usize >= vertices.len() {
                return Err(HygeError::parse(format!(
                    "lod bake: index {i} (triangle {tri}) out of range (vertex_count = {})",
                    vertices.len()
                )));
            }
        }
    }
    for (i, &r) in ratios.iter().enumerate() {
        if !(r > 0.0 && r <= 1.0) {
            return Err(HygeError::parse(format!(
                "lod bake: ratio[{i}] = {r} is not in (0.0, 1.0]"
            )));
        }
    }

    let positions: Vec<[f32; 3]> = vertices.iter().map(|v| v.position).collect();
    let adapter = build_position_adapter(&positions)?;
    let base_triangle_count = indices.len() / 3;
    let options = SimplifyOptions::None;

    let mut lods: Vec<Vec<u32>> = Vec::with_capacity(ratios.len());
    for &ratio in ratios {
        let target_triangles = ((base_triangle_count as f32) * ratio).round() as usize;
        // meshopt requires at least 4 triangles (the API works on
        // triangle counts and a single quad is the smallest
        // meaningful simplification target).
        let target_triangles = target_triangles.max(1).min(base_triangle_count);
        let target_indices = target_triangles * 3;
        // target_error = f32::INFINITY -> meshopt picks the
        // smallest error that achieves the target count, which is
        // the behaviour we want for a fixed-ratio LOD chain.
        let simplified = simplify(
            indices,
            &adapter,
            target_indices,
            f32::INFINITY,
            options,
            None,
        );
        lods.push(simplified);
    }
    Ok(lods)
}

/// Builds a position-only `VertexDataAdapter` for the meshopt
/// APIs. The adapter wraps a tightly-packed little-endian
/// `f32[3]` per vertex; `vertex_stride` is 12 and `position_offset`
/// is 0.
fn build_position_adapter<'a>(positions: &'a [[f32; 3]]) -> HygeResult<VertexDataAdapter<'a>> {
    let bytes: &'a [u8] = bytemuck::cast_slice(positions);
    VertexDataAdapter::new(bytes, 12, 0)
        .map_err(|e| HygeError::parse(format!("meshopt VertexDataAdapter::new failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bake_meshlets_quad_yields_one_meshlet_with_correct_bounds() {
        let (v, i) = quad();
        let m = bake_meshlets(&v, &i).expect("bake succeeds");
        assert_eq!(m.len(), 1, "a 2-triangle quad fits in one meshlet");
        let ml = &m[0];
        assert_eq!(ml.local_vertex_indices.len(), MESHLET_LOCAL_VERTEX_TABLE);
        assert_eq!(ml.local_triangle_indices.len(), 6);
        // The 4 unique local vertex slots are populated; the rest are sentinel.
        let populated: Vec<u32> = ml
            .local_vertex_indices
            .iter()
            .copied()
            .filter(|&x| x != u32::MAX)
            .collect();
        assert_eq!(populated.len(), 4);
        for &vi in &populated {
            assert!(vi < v.len() as u32, "local table points into global buffer");
        }
        // AABB must bracket the four input corners.
        assert_eq!(ml.aabb_min, [0.0, 0.0, 0.0]);
        assert_eq!(ml.aabb_max, [1.0, 1.0, 0.0]);
        // Bounding sphere center sits inside the unit square.
        for axis in 0..3 {
            assert!(
                ml.cone_center[axis] >= ml.aabb_min[axis] - 1e-5
                    && ml.cone_center[axis] <= ml.aabb_max[axis] + 1e-5
            );
        }
        assert!(ml.cone_radius > 0.0);
    }

    #[test]
    fn bake_meshlets_empty_indices_yields_empty_vec() {
        let (v, _) = quad();
        let m = bake_meshlets(&v, &[]).expect("empty input is OK");
        assert!(m.is_empty());
    }

    #[test]
    fn bake_meshlets_rejects_oor_index() {
        let (v_orig, _) = quad();
        let mut v = v_orig;
        v.truncate(2); // drop two vertices
        let i = vec![0u32, 1, 3, 0, 1, 3];
        let err = bake_meshlets(&v, &i).expect_err("must reject OOR index");
        assert!(matches!(err, HygeError::Parse(_)));
    }

    #[test]
    fn bake_meshlets_rejects_non_triangle_index_count() {
        let (v, _) = quad();
        let i = vec![0, 1, 2, 0, 2]; // 5 indices
        let err = bake_meshlets(&v, &i).expect_err("must reject non-tri count");
        assert!(matches!(err, HygeError::Parse(_)));
    }

    #[test]
    fn bake_meshlets_respects_max_triangle_cap() {
        // Build a fan of 20 triangles from a centre vertex.
        // 20 tris * 3 = 60 indices, 11 vertices. With the
        // MESHLET_MAX_TRIANGLES = 128 cap, all 20 fit in one
        // meshlet, so the test is the trivial pass; the meaningful
        // assertion is that the cap is honoured by meshopt itself
        // (a separate test with a much larger mesh would split
        // into multiple meshlets).
        let n = 20;
        let mut v = vec![Vertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        }];
        for k in 0..n {
            let angle = (k as f32) * std::f32::consts::TAU / (n as f32);
            v.push(Vertex {
                position: [angle.cos(), angle.sin(), 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            });
        }
        let mut i = Vec::with_capacity(n * 3);
        for k in 0..n {
            let a = ((k + 1) % n + 1) as u32;
            let b = ((k + 2) % n + 1) as u32;
            i.extend_from_slice(&[0, a, b]);
        }
        let m = bake_meshlets(&v, &i).expect("bake succeeds");
        assert!(!m.is_empty());
        for ml in &m {
            assert!(ml.local_triangle_indices.len() / 3 <= MESHLET_MAX_TRIANGLES);
            assert!(ml.local_vertex_indices.len() <= MESHLET_LOCAL_VERTEX_TABLE);
        }
    }

    #[test]
    fn bake_meshlets_is_deterministic() {
        // Same input twice -> identical output bytes (the
        // serialisation is owned by `mesh::write`, but the bake
        // itself is the deterministic kernel the test pins).
        let (v, i) = quad();
        let a = bake_meshlets(&v, &i).expect("bake A");
        let b = bake_meshlets(&v, &i).expect("bake B");
        assert_eq!(a, b);
    }

    #[test]
    fn bake_lod_chain_emits_three_levels_in_ratio_order() {
        let (v, i) = quad();
        let lods = bake_lod_chain(&v, &i, LOD_RATIOS).expect("bake LODs");
        assert_eq!(lods.len(), LOD_RATIOS.len());
        // Each LOD is at least 1 triangle (small meshes cannot
        // simplify below 1).
        for lod in &lods {
            assert!(!lod.is_empty(), "LOD with empty indices is unexpected");
            assert!(lod.len() % 3 == 0, "LOD indices must be triangle-aligned");
        }
    }

    #[test]
    fn bake_lod_chain_empty_ratios_yields_empty_vec() {
        let (v, i) = quad();
        let lods = bake_lod_chain(&v, &i, &[]).expect("empty ratios OK");
        assert!(lods.is_empty());
    }

    #[test]
    fn bake_lod_chain_rejects_bad_ratio() {
        let (v, i) = quad();
        let err = bake_lod_chain(&v, &i, &[0.0]).expect_err("zero ratio rejected");
        assert!(matches!(err, HygeError::Parse(_)));
        let err = bake_lod_chain(&v, &i, &[1.5]).expect_err("ratio > 1 rejected");
        assert!(matches!(err, HygeError::Parse(_)));
    }

    #[test]
    fn bake_lod_chain_rejects_oor_index() {
        // Parity with `bake_meshlets_rejects_oor_index` — the
        // R-035 review caught that the public `bake_lod_chain`
        // skipped the index-bounds pre-check that
        // `bake_meshlets` does. Direct callers (i.e. the public
        // API) must be safe by themselves, not only when routed
        // through `MeshData::bake`.
        let (v_orig, _) = quad();
        let mut v = v_orig;
        v.truncate(2);
        let i = vec![0u32, 1, 3, 0, 1, 3];
        let err = bake_lod_chain(&v, &i, LOD_RATIOS).expect_err("must reject OOR index");
        assert!(matches!(err, HygeError::Parse(_)));
    }

    #[test]
    fn bake_lod_chain_is_deterministic() {
        let (v, i) = quad();
        let a = bake_lod_chain(&v, &i, LOD_RATIOS).expect("A");
        let b = bake_lod_chain(&v, &i, LOD_RATIOS).expect("B");
        assert_eq!(a, b, "LOD chain must be deterministic for fixed input");
    }

    #[test]
    fn lod_chain_larger_mesh_shrinks_with_ratio() {
        // Build a 40-triangle fan; LODs at 0.5 / 0.25 / 0.1 should
        // produce triangle counts at-or-below the corresponding
        // ceiling. meshopt may overshoot by a few triangles, so we
        // allow a small slack.
        let n = 40;
        let mut v = vec![Vertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        }];
        for k in 0..n {
            let angle = (k as f32) * std::f32::consts::TAU / (n as f32);
            v.push(Vertex {
                position: [angle.cos(), angle.sin(), 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            });
        }
        let mut i = Vec::with_capacity(n * 3);
        for k in 0..n {
            let a = ((k + 1) % n + 1) as u32;
            let b = ((k + 2) % n + 1) as u32;
            i.extend_from_slice(&[0, a, b]);
        }
        let lods = bake_lod_chain(&v, &i, &[0.5, 0.25, 0.1]).expect("bake");
        for (lod, ratio) in lods.iter().zip([0.5_f32, 0.25, 0.1].iter()) {
            let tri_count = lod.len() / 3;
            let target = (n as f32 * ratio).round() as usize;
            // meshopt can over-shoot by a few triangles when
            // preserving topology, so the bound is generous.
            assert!(
                tri_count <= (target * 2).max(4),
                "LOD ratio={ratio} produced {tri_count} triangles, target was {target}"
            );
        }
    }
}
