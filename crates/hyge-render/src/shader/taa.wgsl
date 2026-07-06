// Temporal anti-aliasing resolve (R-052).

@group(0) @binding(0) var current_tex : texture_2d<f32>;
@group(0) @binding(1) var history_tex : texture_2d<f32>;
@group(0) @binding(2) var linear_sampler : sampler;

struct TaaParams {
    history_weight : f32,
    current_weight : f32,
    reset : u32,
    _pad : u32,
}

@group(0) @binding(3) var<uniform> params : TaaParams;

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    // Reference compute entry point. Runtime storage-texture wiring lands
    // through the post-process graph; the formula below documents the
    // resolve contract for validation.
    _ = gid;
}

fn variance_clip(current : vec3<f32>, history : vec3<f32>, min_n : vec3<f32>, max_n : vec3<f32>) -> vec3<f32> {
    return clamp(history, min_n, max_n) * params.history_weight + current * params.current_weight;
}
