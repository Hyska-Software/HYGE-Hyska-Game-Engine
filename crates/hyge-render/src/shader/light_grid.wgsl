// Hyge R-042 light-grid compute shader.
//
// CPU-driven clustered-forward light culling. The CPU partitions the
// screen-depth frustum into a 3D grid of tiles (clusters). This shader
// is the GPU-side validation / reference implementation: each
// workgroup writes a compact light-index list for one cluster by
// testing every light against the cluster AABB.
//
// In the runtime path the CPU builds the same lists on the host
// (single-threaded, fast enough for <4k lights) and uploads the
// `LightGrid` and index buffers to the bindless table. This shader
// exists for the future GPU-driven culling path and for naga
// validation.

const PI : f32 = 3.141592653589793;
const MAX_LIGHTS_PER_CLUSTER : u32 = 256u;

struct Light {
    position : vec4<f32>,
    color : vec4<f32>,
    direction : vec4<f32>,
}

struct ClusterBounds {
    min : vec3<f32>,
    max : vec3<f32>,
}

struct Params {
    cluster_count_x : u32,
    cluster_count_y : u32,
    cluster_count_z : u32,
    light_count : u32,
}

@group(0) @binding(0) var<storage, read> lights : array<Light>;
@group(0) @binding(1) var<uniform> params : Params;
@group(0) @binding(2) var<storage, read_write> light_grid : array<u32>;
@group(0) @binding(3) var<storage, read_write> light_index_list : array<u32>;

fn cluster_aabb(id : vec3<u32>) -> ClusterBounds {
    let n = vec3<f32>(id);
    let inv = vec3<f32>(
        f32(params.cluster_count_x),
        f32(params.cluster_count_y),
        f32(params.cluster_count_z)
    );
    let min_p = n / inv * 2.0 - vec3<f32>(1.0);
    let max_p = (n + vec3<f32>(1.0)) / inv * 2.0 - vec3<f32>(1.0);
    // Z is packed logarithmically in the runtime; here we use a
    // linear proxy for the reference shader.
    return ClusterBounds(min_p, max_p);
}

fn sphere_aabb_intersect(center : vec3<f32>, radius : f32, bounds : ClusterBounds) -> bool {
    let closest = clamp(center, bounds.min, bounds.max);
    let dist2 = dot(center - closest, center - closest);
    return dist2 <= radius * radius;
}

fn light_radius(light : Light) -> f32 {
    // Very conservative infinite radius for directional lights;
    // point/spot use a fixed 10 unit cutoff in this reference.
    let ty = u32(light.position.w);
    if (ty == 2u) {
        return 1.0e6;
    }
    return 10.0;
}

@compute @workgroup_size(8, 8, 1)
fn main(
    @builtin(global_invocation_id) gid : vec3<u32>,
) {
    if (gid.x >= params.cluster_count_x ||
        gid.y >= params.cluster_count_y ||
        gid.z >= params.cluster_count_z) {
        return;
    }

    let cluster_id = gid.x +
        gid.y * params.cluster_count_x +
        gid.z * params.cluster_count_x * params.cluster_count_y;
    let bounds = cluster_aabb(gid);

    var local_count : u32 = 0u;
    for (var i : u32 = 0u; i < params.light_count && local_count < MAX_LIGHTS_PER_CLUSTER; i = i + 1u) {
        let light = lights[i];
        let radius = light_radius(light);
        if (sphere_aabb_intersect(light.position.xyz, radius, bounds)) {
            // In a full GPU implementation we would atomically append
            // to a global index list. For the reference shader we
            // simply count and store the first light id as evidence.
            let list_offset = cluster_id * MAX_LIGHTS_PER_CLUSTER + local_count;
            light_index_list[list_offset] = i;
            local_count = local_count + 1u;
        }
    }

    // Each grid entry stores [offset, count]. We pack them as two
    // consecutive u32 values.
    light_grid[cluster_id * 2u] = cluster_id * MAX_LIGHTS_PER_CLUSTER;
    light_grid[cluster_id * 2u + 1u] = local_count;
}
