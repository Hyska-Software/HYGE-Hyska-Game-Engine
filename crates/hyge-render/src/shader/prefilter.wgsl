// Hyge R-041 prefilter compute shader.
//
// Reference GPU implementation of the prefilter (roughness-mipped
// environment cubemap) bake. The CPU implementation in
// `crates/hyge-render/src/ibl.rs::prefilter_env` is the source of
// truth; this WGSL file is naga-validated and will be used by the
// future "online re-bake" compute pass that re-bakes an
// environment at runtime when a user attaches a custom HDR.
//
// The dispatch is one thread per (face, pixel) for each roughness
// mip; the caller dispatches once per mip level with a fresh
// `roughness` uniform.
//
// Math (Karis 2013, "Real Shading in Unreal Engine 4"):
//   - Sample N=1024 directions per output texel via Hammersley.
//   - Half-vector importance-sampled from the GGX NDF (alpha^2).
//   - Reflect the view direction about the half vector to get
//     the light direction, sample the input cubemap.
//   - Accumulate, weight by N.L.

const PI : f32 = 3.141592653589793;
const EPSILON : f32 = 0.00001;
const SAMPLE_COUNT : u32 = 1024u;

struct Params {
    face : u32,
    size : u32,
    roughness : f32,
    sample_mip : f32,
}

@group(0) @binding(0) var env_sampler : sampler;
@group(0) @binding(1) var env_cube : texture_cube<f32>;
@group(0) @binding(2) var<uniform> params : Params;

var<workgroup> shared_samples : array<f32, 1024>;

fn direction_for(face : u32, u : f32, v : f32) -> vec3<f32> {
    // Mirrors the CPU `face_dir` in `ibl.rs`. Y-up.
    switch (face) {
        case 0u: { return normalize(vec3<f32>( 1.0, -v, -u)); }
        case 1u: { return normalize(vec3<f32>(-1.0, -v,  u)); }
        case 2u: { return normalize(vec3<f32>( u,    1.0, -v)); }
        case 3u: { return normalize(vec3<f32>( u,   -1.0,  v)); }
        case 4u: { return normalize(vec3<f32>( u,   -v,   1.0)); }
        default: { return normalize(vec3<f32>(-u,   -v,  -1.0)); }
    }
}

fn radical_inverse_vdc(bits_in : u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

fn hammersley(i : u32, n : u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}

fn importance_sample_ggx(r1 : f32, r2 : f32, a2 : f32) -> vec3<f32> {
    let cos_theta_h = sqrt((1.0 - r1) / (r1 * (a2 - 1.0) + 1.0));
    let sin_theta_h = sqrt(max(1.0 - cos_theta_h * cos_theta_h, 0.0));
    let phi_h = 2.0 * PI * r2;
    return vec3<f32>(
        sin_theta_h * cos(phi_h),
        sin_theta_h * sin(phi_h),
        cos_theta_h
    );
}

@compute @workgroup_size(8, 8, 1)
fn main(
    @builtin(global_invocation_id) gid : vec3<u32>,
) {
    let size = params.size;
    if (gid.x >= size || gid.y >= size) {
        return;
    }
    let u = (f32(gid.x) + 0.5) / f32(size) * 2.0 - 1.0;
    let v = (f32(gid.y) + 0.5) / f32(size) * 2.0 - 1.0;
    let n = direction_for(params.face, u, v);
    let v_dir = n;
    let a = max(params.roughness * params.roughness, 0.0025);
    let a2 = a * a;
    var acc = vec3<f32>(0.0);
    var total_weight = 0.0;

    for (var i = 0u; i < SAMPLE_COUNT; i = i + 1u) {
        let r = hammersley(i, SAMPLE_COUNT);
        let h = importance_sample_ggx(r.x, r.y, a2);
        let l = normalize(reflect(-v_dir, h));
        let nl = max(dot(n, l), 0.0);
        if (nl > 0.0) {
            let sample = textureSampleLevel(env_cube, env_sampler, l, params.sample_mip).rgb;
            acc = acc + sample * nl;
            total_weight = total_weight + nl;
        }
    }

    var out_color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (total_weight > 0.0) {
        out_color = vec4<f32>(acc / total_weight, 1.0);
    }
    // The output is written by the caller; this reference WGSL
    // exists for naga validation and for the future online
    // re-bake compute pipeline (which will add a `result`
    // storage texture binding and a `textureStore` here).
    _ = out_color;
}
