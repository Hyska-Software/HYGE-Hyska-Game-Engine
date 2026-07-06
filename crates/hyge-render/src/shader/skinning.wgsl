// Skinning compute shader (R-056).

struct SkinVertex {
    position : vec4<f32>,
    normal : vec4<f32>,
    joint_indices : vec4<u32>,
    joint_weights : vec4<f32>,
}

struct SkinnedVertex {
    position : vec4<f32>,
    normal : vec4<f32>,
}

@group(0) @binding(0) var<storage, read> vertices : array<SkinVertex>;
@group(0) @binding(1) var<storage, read> joints : array<mat4x4<f32>>;
@group(0) @binding(2) var<storage, read_write> out_vertices : array<SkinnedVertex>;

@compute @workgroup_size(64, 1, 1)
fn cs_main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&vertices)) {
        return;
    }
    let v = vertices[i];
    var pos = vec4<f32>(0.0);
    var nrm = vec3<f32>(0.0);
    for (var j = 0u; j < 4u; j = j + 1u) {
        let joint = joints[v.joint_indices[j]];
        let weight = v.joint_weights[j];
        pos = pos + (joint * vec4<f32>(v.position.xyz, 1.0)) * weight;
        nrm = nrm + (joint * vec4<f32>(v.normal.xyz, 0.0)).xyz * weight;
    }
    out_vertices[i].position = vec4<f32>(pos.xyz, 1.0);
    out_vertices[i].normal = vec4<f32>(normalize(nrm), 0.0);
}
