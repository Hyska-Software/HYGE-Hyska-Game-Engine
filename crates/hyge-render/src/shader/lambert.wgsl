// Lambert WGSL shader for the M2 lit-sphere smoke test.
//
// M2 / R-038 uses Lambert because the PBR shader is R-040
// (M3). The shader pulls a per-vertex position + normal from
// the global vertex buffer (indexed by the bindless mesh's
// `vertex_offset`), transforms by the model-view-projection
// matrix, and shades by Lambert: `color = base_color * max(0,
// dot(N, sun_dir))`. A small ambient term keeps the back
// side from going fully black.
//
// Vertex layout (matches `crate::lambert::Vertex`):
//   @location(0) position : vec3<f32>
//   @location(1) normal   : vec3<f32>
//
// Uniforms (single bind group, all per-frame):
//   @group(0) @binding(0) mvp      : mat4x4<f32>
//   @group(0) @binding(1) model    : mat4x4<f32>
//   @group(0) @binding(2) material : Material (base_color + sun_dir)
//
// The material is supplied as a plain uniform buffer for
// M2; R-040 (PBR) replaces this with a `textureLoad` from
// the bindless material storage (slot 5) and the texture
// array (slot 11+). The bindless material is still used in
// the M2 path: the `LambertPass::set_material` method
// copies the bindless `GpuMaterial` into the uniform each
// frame, so the bindless slot allocation is exercised
// even though the shader doesn't read it directly.

struct VsIn {
    @location(0) position : vec3<f32>,
    @location(1) normal : vec3<f32>,
}

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0) world_normal : vec3<f32>,
}

struct Material {
    base_color : vec4<f32>,
    sun_dir : vec4<f32>,
    _pad0 : f32,
    _pad1 : f32,
    _pad2 : f32,
    _pad3 : f32,
}

@group(0) @binding(0) var<uniform> mvp : mat4x4<f32>;
@group(0) @binding(1) var<uniform> model : mat4x4<f32>;
@group(0) @binding(2) var<uniform> material : Material;

@vertex
fn vs_main(input : VsIn) -> VsOut {
    var out : VsOut;
    out.clip_pos = mvp * vec4<f32>(input.position, 1.0);
    // The normal transform is `(model * vec4(normal, 0)).xyz`
    // for a rigid body; for a non-uniform scale the correct
    // transform is `transpose(inverse(model))` but the M2
    // sphere is unit-scale so the cheaper form is exact.
    out.world_normal = (model * vec4<f32>(input.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_main(input : VsOut) -> @location(0) vec4<f32> {
    let n = normalize(input.world_normal);
    let l = normalize(material.sun_dir.xyz);
    let n_dot_l = max(dot(n, l), 0.0);
    // Lambert + a small ambient term so the back side of
    // the sphere isn't fully black.
    let ambient = 0.15;
    let lit = ambient + (1.0 - ambient) * n_dot_l;
    let color = material.base_color.rgb * lit;
    return vec4<f32>(color, 1.0);
}
