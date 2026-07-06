// Hyge R-040 PBR shader.
//
// Responsibilities covered here:
// - Vertex shader pulls DrawCommand / Instance / GpuMesh from the bindless
//   storage buffers and then pulls a packed vertex from the global PBR vertex
//   buffer. It outputs world position, normal, tangent, and UV.
// - Fragment shader evaluates a GGX metallic-roughness BRDF, samples
//   irradiance + prefiltered environment + BRDF LUT IBL, supports emissive
//   map contribution, and implements opaque / cutout / blend alpha modes.
//
// Bindless @group(0) mirrors docs/architecture.md §8.1 and bindless.rs.
// Frame/geometry/IBL @group(1) is the PBR pass contract consumed by the
// clustered-forward pipeline in R-042/R-043.

const PI : f32 = 3.141592653589793;
const EPSILON : f32 = 0.00001;
const NULL_SLOT : u32 = 0xffffffffu;
const ALPHA_MODE_OPAQUE : u32 = 0u;
const ALPHA_MODE_CUTOUT : u32 = 1u;
const ALPHA_MODE_BLEND : u32 = 2u;
const MATERIAL_FLAG_EMISSIVE_MAP : u32 = 1u;
const PBR_PACKED_VERTEX_STRIDE_BYTES : u32 = 48u;
const PREFILTERED_ENV_MAX_LOD : f32 = 8.0;

struct GpuMesh {
    vertex_offset : u32,
    index_offset : u32,
    meshlet_offset : u32,
    meshlet_count : u32,
    aabb_min : vec3<f32>,
    aabb_max : vec3<f32>,
    lod_count : u32,
    _pad : u32,
}

struct GpuMaterial {
    base_color : u32,
    normal : u32,
    mr : u32,
    occlusion : u32,
    emissive : u32,
    roughness : f32,
    metallic : f32,
    alpha_mode : u32,
    flags : u32,
}

struct Instance {
    transform : array<vec4<f32>, 3>,
    mesh_id : u32,
    material_id : u32,
    _pad : vec2<u32>,
}

struct Light {
    position : vec4<f32>,
    color : vec4<f32>,
    direction : vec4<f32>,
}

struct LightGrid {
    offset : u32,
    count : u32,
}

struct MeshletVisibility {
    mesh_id : u32,
    meshlet_id : u32,
    lod : u32,
    _pad : u32,
}

struct DrawCommand {
    mesh_id : u32,
    material_id : u32,
    first_instance : u32,
    instance_count : u32,
    aabb_min : vec3<f32>,
    aabb_max : vec3<f32>,
    _pad : vec2<u32>,
}

struct PbrPackedVertex {
    position_normal_x : vec4<f32>,
    normal_yz_tangent_xy : vec4<f32>,
    tangent_zw_uv : vec4<f32>,
}

struct FrameData {
    view_proj : mat4x4<f32>,
    camera_pos_alpha_cutoff : vec4<f32>,
    sun_direction_exposure : vec4<f32>,
    sun_color_intensity : vec4<f32>,
    // Cluster configuration: tiles_x, tiles_y, depth_slices,
    // max_lights_per_cluster. Used by the fragment shader to
    // compute the (cluster, tile) for the current pixel.
    cluster_params : vec4<u32>,
    // Screen width, height in `xy`; view-space near, far in `zw`.
    // Used by the fragment shader to compute the depth slice.
    viewport : vec4<f32>,
    // Camera view matrix (column-major). Used to transform the
    // world position into view space for the Z-slice lookup.
    view : mat4x4<f32>,
}

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0) world_pos : vec3<f32>,
    @location(1) world_normal : vec3<f32>,
    @location(2) world_tangent : vec4<f32>,
    @location(3) uv : vec2<f32>,
    @location(4) material_id : u32,
    @location(5) view_z : f32,
}

@group(0) @binding(0) var linear_clamp_sampler : sampler;
@group(0) @binding(1) var linear_repeat_sampler : sampler;
@group(0) @binding(2) var nearest_clamp_sampler : sampler;
@group(0) @binding(3) var shadow_sampler : sampler_comparison;
@group(0) @binding(4) var<storage, read> meshes : array<GpuMesh>;
@group(0) @binding(5) var<storage, read> materials : array<GpuMaterial>;
@group(0) @binding(6) var<storage, read> instances : array<Instance>;
@group(0) @binding(7) var<storage, read> lights : array<Light>;
@group(0) @binding(8) var<storage, read> light_grid : array<LightGrid>;
@group(0) @binding(9) var<storage, read> meshlet_visibility : array<MeshletVisibility>;
@group(0) @binding(10) var<storage, read> draw_commands : array<DrawCommand>;
@group(0) @binding(11) var material_textures : texture_2d_array<f32>;
@group(0) @binding(12) var<storage, read> light_index_list : array<u32>;

@group(1) @binding(0) var<storage, read> pbr_vertices : array<PbrPackedVertex>;
@group(1) @binding(1) var<uniform> frame : FrameData;
@group(1) @binding(2) var irradiance_map : texture_cube<f32>;
@group(1) @binding(3) var prefiltered_env_map : texture_cube<f32>;
@group(1) @binding(4) var brdf_lut : texture_2d<f32>;

fn transform_point(instance : Instance, p : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(instance.transform[0], vec4<f32>(p, 1.0)),
        dot(instance.transform[1], vec4<f32>(p, 1.0)),
        dot(instance.transform[2], vec4<f32>(p, 1.0))
    );
}

fn transform_vector(instance : Instance, v : vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(instance.transform[0].xyz, v),
        dot(instance.transform[1].xyz, v),
        dot(instance.transform[2].xyz, v)
    );
}

fn unpack_position(v : PbrPackedVertex) -> vec3<f32> {
    return v.position_normal_x.xyz;
}

fn unpack_normal(v : PbrPackedVertex) -> vec3<f32> {
    return normalize(vec3<f32>(v.position_normal_x.w, v.normal_yz_tangent_xy.xy));
}

fn unpack_tangent(v : PbrPackedVertex) -> vec4<f32> {
    return vec4<f32>(v.normal_yz_tangent_xy.zw, v.tangent_zw_uv.xy);
}

fn unpack_uv(v : PbrPackedVertex) -> vec2<f32> {
    return v.tangent_zw_uv.zw;
}

fn texture_layer(texture_id : u32) -> i32 {
    if texture_id == NULL_SLOT {
        return 0;
    }
    return i32(texture_id);
}

fn sample_material_texture(texture_id : u32, uv : vec2<f32>) -> vec4<f32> {
    return textureSample(material_textures, linear_repeat_sampler, uv, texture_layer(texture_id));
}

@vertex
fn vs_main(
    @location(0) draw_id : u32,
    @builtin(vertex_index) vertex_index : u32,
    @builtin(instance_index) instance_index : u32,
) -> VsOut {
    let draw = draw_commands[draw_id];
    let instance = instances[draw.first_instance + instance_index];
    let mesh = meshes[draw.mesh_id];
    let vertex_base = mesh.vertex_offset / PBR_PACKED_VERTEX_STRIDE_BYTES;
    let vertex = pbr_vertices[vertex_base + vertex_index];

    let local_pos = unpack_position(vertex);
    let local_normal = unpack_normal(vertex);
    let local_tangent = unpack_tangent(vertex);
    let world_pos = transform_point(instance, local_pos);
    let world_normal = normalize(transform_vector(instance, local_normal));
    let world_tangent = normalize(transform_vector(instance, local_tangent.xyz));

    var out : VsOut;
    out.clip_pos = frame.view_proj * vec4<f32>(world_pos, 1.0);
    out.world_pos = world_pos;
    out.world_normal = world_normal;
    out.world_tangent = vec4<f32>(world_tangent, local_tangent.w);
    out.uv = unpack_uv(vertex);
    out.material_id = draw.material_id;
    // View-space Z (negative for points in front of the camera).
    // The fragment uses this to compute the cluster's depth slice.
    let view_pos = frame.view * vec4<f32>(world_pos, 1.0);
    out.view_z = view_pos.z;
    return out;
}

fn normal_from_map(material : GpuMaterial, input : VsOut) -> vec3<f32> {
    if material.normal == NULL_SLOT {
        return normalize(input.world_normal);
    }
    let sampled = sample_material_texture(material.normal, input.uv).xyz * 2.0 - vec3<f32>(1.0);
    let n = normalize(input.world_normal);
    let t = normalize(input.world_tangent.xyz);
    let b = normalize(cross(n, t)) * input.world_tangent.w;
    let tbn = mat3x3<f32>(t, b, n);
    return normalize(tbn * sampled);
}

fn fresnel_schlick(cos_theta : f32, f0 : vec3<f32>) -> vec3<f32> {
    let x = clamp(1.0 - cos_theta, 0.0, 1.0);
    return f0 + (vec3<f32>(1.0) - f0) * pow(x, 5.0);
}

fn fresnel_schlick_roughness(cos_theta : f32, f0 : vec3<f32>, roughness : f32) -> vec3<f32> {
    let x = clamp(1.0 - cos_theta, 0.0, 1.0);
    return f0 + (max(vec3<f32>(1.0 - roughness), f0) - f0) * pow(x, 5.0);
}

fn distribution_ggx(n : vec3<f32>, h : vec3<f32>, roughness : f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let n_dot_h = max(dot(n, h), 0.0);
    let n_dot_h2 = n_dot_h * n_dot_h;
    let denom = (n_dot_h2 * (a2 - 1.0) + 1.0);
    return a2 / max(PI * denom * denom, EPSILON);
}

fn geometry_schlick_ggx(n_dot_v : f32, roughness : f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_v / max(n_dot_v * (1.0 - k) + k, EPSILON);
}

fn geometry_smith(n : vec3<f32>, v : vec3<f32>, l : vec3<f32>, roughness : f32) -> f32 {
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_l = max(dot(n, l), 0.0);
    let ggx_v = geometry_schlick_ggx(n_dot_v, roughness);
    let ggx_l = geometry_schlick_ggx(n_dot_l, roughness);
    return ggx_v * ggx_l;
}

fn ibl_radiance(n : vec3<f32>, v : vec3<f32>, base_color : vec3<f32>, metallic : f32, roughness : f32, f0 : vec3<f32>, ao : f32) -> vec3<f32> {
    let n_dot_v = max(dot(n, v), 0.0);
    let f = fresnel_schlick_roughness(n_dot_v, f0, roughness);
    let ks = f;
    let kd = (vec3<f32>(1.0) - ks) * (1.0 - metallic);
    let irradiance = textureSample(irradiance_map, linear_clamp_sampler, n).rgb;
    let diffuse = irradiance * base_color;
    let r = reflect(-v, n);
    let prefiltered = textureSampleLevel(prefiltered_env_map, linear_clamp_sampler, r, roughness * PREFILTERED_ENV_MAX_LOD).rgb;
    let brdf = textureSample(brdf_lut, linear_clamp_sampler, vec2<f32>(n_dot_v, roughness)).rg;
    let specular = prefiltered * (f * brdf.x + vec3<f32>(brdf.y));
    return (kd * diffuse + specular) * ao;
}

// Compute the (cluster_x, cluster_y, cluster_z) for a screen-space
// pixel and a view-space Z. The cluster grid is a uniform 3D
// partition of the view frustum:
//   - XY tiles map from screen-space UV in (0,1) to (tiles_x,
//     tiles_y).
//   - Z slices map from view-space Z (clamped to [near, far]) on
//     a uniform partition into `depth_slices` buckets.
//
// `screen_xy` is the pixel's screen coordinates in pixels (0,0) at
// the top-left. `view_z` is the pixel's view-space Z (negative for
// points in front of the camera).
fn compute_cluster_id(screen_xy : vec2<f32>, view_z : f32) -> vec3<u32> {
    let tiles_x = max(frame.cluster_params.x, 1u);
    let tiles_y = max(frame.cluster_params.y, 1u);
    let depth_slices = max(frame.cluster_params.z, 1u);
    let screen_w = max(frame.viewport.x, 1.0);
    let screen_h = max(frame.viewport.y, 1.0);
    let near = frame.viewport.z;
    let far = frame.viewport.w;
    // Screen UV -> tile indices.
    let cx = u32(clamp(screen_xy.x / screen_w * f32(tiles_x), 0.0, f32(tiles_x) - 1.0));
    let cy = u32(clamp(screen_xy.y / screen_h * f32(tiles_y), 0.0, f32(tiles_y) - 1.0));
    // Linear depth slice: 0 = nearest, depth_slices-1 = farthest.
    // The view-space Z is negative in front of the camera; map
    // |view_z| from [near, far] to [0, depth_slices - 1].
    let depth_norm = clamp((-view_z - near) / max(far - near, 1e-4), 0.0, 1.0);
    let cz = u32(clamp(depth_norm * f32(depth_slices), 0.0, f32(depth_slices) - 1.0));
    return vec3<u32>(cx, cy, cz);
}

fn cluster_linear_index(tile : vec3<u32>) -> u32 {
    let tiles_x = max(frame.cluster_params.x, 1u);
    let tiles_y = max(frame.cluster_params.y, 1u);
    return tile.x + tile.y * tiles_x + tile.z * tiles_x * tiles_y;
}

// Direct light contribution from a single Light entry.
// `world_pos` is the world-space position of the shaded surface;
// `n` is the world-space normal; `v` is the normalized view vector
// (from the surface toward the camera).
fn direct_light_radiance(world_pos : vec3<f32>, n : vec3<f32>, v : vec3<f32>, base_color : vec3<f32>, metallic : f32, roughness : f32, f0 : vec3<f32>, light : Light) -> vec3<f32> {
    // Light types (matches `LightComponent::sun/point/spot`):
    //   0 = point, 1 = spot, 2 = directional.
    let light_type = u32(light.position.w);
    // Toward-the-light vector.
    var l_dir : vec3<f32>;
    if (light_type == 2u) {
        // Directional: `light.position.xyz` is the *direction
        // toward* the sun; we negate to get the *toward-the-light*
        // vector.
        l_dir = normalize(-light.position.xyz);
    } else {
        // Point/spot: `light.position.xyz` is the world position;
        // the toward-the-light vector is the direction from the
        // shaded point to the light.
        l_dir = normalize(light.position.xyz - world_pos);
    }
    let l = l_dir;
    let h = normalize(v + l);
    let radiance = light.color.rgb * light.color.a;
    let ndf = distribution_ggx(n, h, roughness);
    let g = geometry_smith(n, v, l, roughness);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let numerator = ndf * g * f;
    let denominator = max(4.0 * max(dot(n, v), 0.0) * max(dot(n, l), 0.0), EPSILON);
    let specular = numerator / denominator;
    let ks = f;
    let kd = (vec3<f32>(1.0) - ks) * (1.0 - metallic);
    let n_dot_l = max(dot(n, l), 0.0);
    return (kd * base_color / PI + specular) * radiance * n_dot_l;
}

@fragment
fn fs_main(input : VsOut) -> @location(0) vec4<f32> {
    let material = materials[input.material_id];
    let base_sample = sample_material_texture(material.base_color, input.uv);
    let alpha_cutoff = frame.camera_pos_alpha_cutoff.w;

    if material.alpha_mode == ALPHA_MODE_CUTOUT && base_sample.a < alpha_cutoff {
        discard;
    }

    let mr_sample = sample_material_texture(material.mr, input.uv);
    let occlusion_sample = sample_material_texture(material.occlusion, input.uv);
    let roughness = clamp(material.roughness * mr_sample.g, 0.045, 1.0);
    let metallic = clamp(material.metallic * mr_sample.b, 0.0, 1.0);
    let ao = clamp(occlusion_sample.r, 0.0, 1.0);
    let n = normal_from_map(material, input);
    let v = normalize(frame.camera_pos_alpha_cutoff.xyz - input.world_pos);
    let base_color = max(base_sample.rgb, vec3<f32>(0.0));
    let f0 = mix(vec3<f32>(0.04), base_color, vec3<f32>(metallic));

    // Iterate the lights assigned to the cluster/tile this
    // pixel falls into. R-042 acceptance #4: the PBR
    // fragment samples the LightGrid for the (cluster, tile)
    // and iterates the lights in the cluster's index list.
    let screen_xy = input.clip_pos.xy;
    let cluster_tile = compute_cluster_id(screen_xy, input.view_z);
    let cluster_id = cluster_linear_index(cluster_tile);
    let grid = light_grid[cluster_id];

    var color : vec3<f32> = vec3<f32>(0.0);
    let max_lights = min(grid.count, frame.cluster_params.w);
    for (var i : u32 = 0u; i < max_lights; i = i + 1u) {
        let light_index = light_index_list[grid.offset + i];
        if (light_index >= arrayLength(&lights)) {
            continue;
        }
        color += direct_light_radiance(
            input.world_pos,
            n,
            v,
            base_color,
            metallic,
            roughness,
            f0,
            lights[light_index]
        );
    }

    color += ibl_radiance(n, v, base_color, metallic, roughness, f0, ao);

    if (material.flags & MATERIAL_FLAG_EMISSIVE_MAP) != 0u {
        color += sample_material_texture(material.emissive, input.uv).rgb;
    }

    let exposure = frame.sun_direction_exposure.w;
    let exposed = vec3<f32>(1.0) - exp(-color * exposure);
    let alpha = select(1.0, base_sample.a, material.alpha_mode == ALPHA_MODE_BLEND);
    return vec4<f32>(exposed, alpha);
}
