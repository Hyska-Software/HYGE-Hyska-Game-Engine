// Meshlet culling and LOD selection shader (R-058).

struct MeshletBounds {
    center_radius : vec4<f32>,
    meshlet_id : u32,
    mesh_id : u32,
    lod_count : u32,
    _pad : u32,
}

struct VisibleMeshlet {
    mesh_id : u32,
    meshlet_id : u32,
    lod : u32,
    _pad : u32,
}

@group(0) @binding(0) var<storage, read> meshlets : array<MeshletBounds>;
@group(0) @binding(1) var<storage, read_write> visible : array<VisibleMeshlet>;
@group(0) @binding(2) var<storage, read_write> visible_count : atomic<u32>;

@compute @workgroup_size(64, 1, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let idx = gid.x;
    if (idx >= arrayLength(&meshlets)) {
        return;
    }
    let m = meshlets[idx];
    // Placeholder for frustum plane test; the CPU mirror and renderer feed
    // already culled batches until GPU frustum constants are bound.
    let out_idx = atomicAdd(&visible_count, 1u);
    visible[out_idx].mesh_id = m.mesh_id;
    visible[out_idx].meshlet_id = m.meshlet_id;
    visible[out_idx].lod = select(1u, 0u, m.center_radius.w > 1.0);
    visible[out_idx]._pad = 0u;
}
