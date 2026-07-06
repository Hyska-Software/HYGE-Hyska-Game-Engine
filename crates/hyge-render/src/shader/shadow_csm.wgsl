// Cascaded shadow-map depth shader (R-050).

struct ShadowFrame {
    light_view_proj : mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> shadow_frame : ShadowFrame;

struct VsOut {
    @builtin(position) pos : vec4<f32>,
}

@vertex
fn vs_main(@location(0) position : vec3<f32>) -> VsOut {
    var out : VsOut;
    out.pos = shadow_frame.light_view_proj * vec4<f32>(position, 1.0);
    return out;
}
