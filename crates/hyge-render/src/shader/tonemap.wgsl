// ACES tonemap pass (R-055).

@group(0) @binding(0) var source_tex : texture_2d<f32>;
@group(0) @binding(1) var linear_sampler : sampler;

struct VsOut {
    @builtin(position) pos : vec4<f32>,
    @location(0) uv : vec2<f32>,
}

fn aces_filmic(x : vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + vec3<f32>(b))) / (x * (c * x + vec3<f32>(d)) + vec3<f32>(e)), vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index : u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -3.0), vec2<f32>(3.0, 1.0), vec2<f32>(-1.0, 1.0));
    var out : VsOut;
    out.pos = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = out.pos.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    return out;
}

@fragment
fn fs_main(input : VsOut) -> @location(0) vec4<f32> {
    let hdr = textureSample(source_tex, linear_sampler, input.uv).rgb;
    return vec4<f32>(aces_filmic(hdr), 1.0);
}
