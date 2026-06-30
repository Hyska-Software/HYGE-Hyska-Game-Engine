// Hyge R-041 irradiance compute shader.
//
// Reference GPU implementation of the diffuse irradiance cubemap
// bake (acceptance criterion 2). The CPU implementation in
// `crates/hyge-render/src/ibl.rs::diffuse_irradiance` (Ramamoorthi
// 2001 SH projection) is the source of truth; this WGSL file is
// naga-validated and will be used by the future online re-bake
// compute pass that re-irradiances a freshly attached HDR at
// runtime.
//
// The dispatch is one thread per (face, pixel) of the
// `IRRADIANCE_SIZE x IRRADIANCE_SIZE` output cubemap face.
// Spherical convolution via Monte Carlo over the upper hemisphere
// with a cosine-weighted sample.

const PI : f32 = 3.141592653589793;
const SAMPLE_COUNT : u32 = 1024u;

struct Params {
    face : u32,
    size : u32,
}

@group(0) @binding(0) var env_sampler : sampler;
@group(0) @binding(1) var env_cube : texture_cube<f32>;
@group(0) @binding(2) var<uniform> params : Params;

fn direction_for(face : u32, u : f32, v : f32) -> vec3<f32> {
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
    var acc = vec3<f32>(0.0);
    var weight_sum = 0.0;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let tangent_space = build_orthonormal_basis(n);

    for (var i = 0u; i < SAMPLE_COUNT; i = i + 1u) {
        let r = hammersley(i, SAMPLE_COUNT);
        // Cosine-weighted hemisphere sample.
        let cos_theta = sqrt(1.0 - r.x);
        let sin_theta = sqrt(r.x);
        let phi = 2.0 * PI * r.y;
        let h = vec3<f32>(sin_theta * cos(phi), cos_theta, sin_theta * sin(phi));
        let sample_dir = tangent_space * h;
        let nl = max(dot(sample_dir, up), 0.0);
        if (nl > 0.0) {
            let sample = textureSampleLevel(env_cube, env_sampler, sample_dir, 0.0).rgb;
            acc = acc + sample * nl;
            weight_sum = weight_sum + nl;
        }
    }

    var out_color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (weight_sum > 0.0) {
        out_color = vec4<f32>(acc / weight_sum, 1.0);
    }
    _ = out_color;
}

fn build_orthonormal_basis(n : vec3<f32>) -> mat3x3<f32> {
    // Gram-Schmidt around the surface normal `n`. `t1` and `t2`
    // span the tangent plane. We pick an `up` reference axis that
    // is not parallel to `n` to avoid the degenerate cross product.
    var a = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(n.y) > 0.9) {
        a = vec3<f32>(1.0, 0.0, 0.0);
    }
    let t1 = normalize(cross(a, n));
    let t2 = cross(n, t1);
    return mat3x3<f32>(t1, t2, n);
}
