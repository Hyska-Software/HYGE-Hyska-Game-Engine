// Dual-Kawase bloom shader (R-054).

@group(0) @binding(0) var source_tex : texture_2d<f32>;
@group(0) @binding(1) var linear_sampler : sampler;

struct BloomParams {
    threshold : f32,
    intensity : f32,
    radius : f32,
    _pad : f32,
}

@group(0) @binding(2) var<uniform> params : BloomParams;

fn bright_pass(c : vec3<f32>) -> vec3<f32> {
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let scale = max(luma - params.threshold, 0.0) / max(luma, 0.0001);
    return c * scale;
}

@fragment
fn downsample_fs(@builtin(position) pos : vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source_tex));
    let uv = pos.xy / dims;
    let step = params.radius / dims;
    let c = bright_pass(textureSample(source_tex, linear_sampler, uv).rgb);
    let a = textureSample(source_tex, linear_sampler, uv + vec2<f32>(step.x, step.y)).rgb;
    let b = textureSample(source_tex, linear_sampler, uv + vec2<f32>(-step.x, step.y)).rgb;
    let d = textureSample(source_tex, linear_sampler, uv + vec2<f32>(step.x, -step.y)).rgb;
    let e = textureSample(source_tex, linear_sampler, uv + vec2<f32>(-step.x, -step.y)).rgb;
    return vec4<f32>((c * 4.0 + a + b + d + e) / 8.0, 1.0);
}

@fragment
fn upsample_fs(@builtin(position) pos : vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(source_tex));
    let uv = pos.xy / dims;
    let step = params.radius / dims;
    let c = textureSample(source_tex, linear_sampler, uv).rgb;
    let a = textureSample(source_tex, linear_sampler, uv + vec2<f32>(step.x, 0.0)).rgb;
    let b = textureSample(source_tex, linear_sampler, uv - vec2<f32>(step.x, 0.0)).rgb;
    let d = textureSample(source_tex, linear_sampler, uv + vec2<f32>(0.0, step.y)).rgb;
    let e = textureSample(source_tex, linear_sampler, uv - vec2<f32>(0.0, step.y)).rgb;
    return vec4<f32>((c * 4.0 + a + b + d + e) / 8.0 * params.intensity, 1.0);
}
