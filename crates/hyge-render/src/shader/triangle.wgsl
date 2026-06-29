// First-triangle shader. R-024.
//
// Loaded via `include_str!` in `src/triangle.rs` and compiled
// by wgpu / naga at pipeline construction time. The vertex
// layout (location 0 = vec2 position, location 1 = vec3 color)
// must match the `Vertex` struct and the
// `wgpu::vertex_attr_array!` mapping in `triangle.rs`.

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color:    vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0)       color:        vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(input.position, 0.0, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
