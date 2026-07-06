// SMAA 3-pass shader entry points (R-053).

@group(0) @binding(0) var source_tex : texture_2d<f32>;
@group(0) @binding(1) var linear_sampler : sampler;

@fragment
fn edge_fs(@builtin(position) pos : vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source_tex));
    let uv = pos.xy / dims;
    let c = textureSample(source_tex, linear_sampler, uv).rgb;
    let r = textureSample(source_tex, linear_sampler, uv + vec2<f32>(1.0 / dims.x, 0.0)).rgb;
    let d = textureSample(source_tex, linear_sampler, uv + vec2<f32>(0.0, 1.0 / dims.y)).rgb;
    let edge = max(length(c - r), length(c - d));
    return vec4<f32>(edge, edge, edge, 1.0);
}

@fragment
fn blend_weight_fs(@location(0) edge : vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(edge, 0.0, 1.0);
}

@fragment
fn neighborhood_fs(@builtin(position) pos : vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source_tex));
    let uv = pos.xy / dims;
    let c = textureSample(source_tex, linear_sampler, uv).rgb;
    let l = textureSample(source_tex, linear_sampler, uv - vec2<f32>(1.0 / dims.x, 0.0)).rgb;
    let r = textureSample(source_tex, linear_sampler, uv + vec2<f32>(1.0 / dims.x, 0.0)).rgb;
    return vec4<f32>((l + c + r) / 3.0, 1.0);
}
