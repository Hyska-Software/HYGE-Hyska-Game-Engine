// Point/spot shadow atlas depth shader (R-051).

struct AtlasFrame {
    light_view_proj : mat4x4<f32>,
    atlas_rect : vec4<f32>,
}

@group(0) @binding(0) var<uniform> atlas_frame : AtlasFrame;

struct VsOut {
    @builtin(position) pos : vec4<f32>,
}

@vertex
fn vs_main(@location(0) position : vec3<f32>) -> VsOut {
    var out : VsOut;
    out.pos = atlas_frame.light_view_proj * vec4<f32>(position, 1.0);
    return out;
}
