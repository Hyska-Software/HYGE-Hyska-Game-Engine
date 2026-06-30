//! R-040 acceptance tests for `shader/pbr.wgsl`.

use hyge_render::prelude::*;

#[test]
fn pbr_shader_naga_validation_passes() {
    let module =
        naga::front::wgsl::parse_str(PBR_SHADER_SOURCE).expect("pbr.wgsl must parse as WGSL");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("pbr.wgsl must validate through naga");
}

#[test]
fn pbr_shader_declares_bindless_vertex_contract() {
    assert!(PBR_SHADER_SOURCE.contains("@group(0) @binding(4) var<storage, read> meshes"));
    assert!(PBR_SHADER_SOURCE.contains("@group(0) @binding(5) var<storage, read> materials"));
    assert!(PBR_SHADER_SOURCE.contains("@group(0) @binding(6) var<storage, read> instances"));
    assert!(PBR_SHADER_SOURCE.contains("@group(0) @binding(10) var<storage, read> draw_commands"));
    assert!(PBR_SHADER_SOURCE.contains("@group(1) @binding(0) var<storage, read> pbr_vertices"));
    assert!(PBR_SHADER_SOURCE.contains("@location(0) world_pos"));
    assert!(PBR_SHADER_SOURCE.contains("@location(1) world_normal"));
    assert!(PBR_SHADER_SOURCE.contains("@location(2) world_tangent"));
    assert!(PBR_SHADER_SOURCE.contains("@location(3) uv"));
}

#[test]
fn pbr_shader_declares_ggx_ibl_emissive_and_alpha_paths() {
    assert!(PBR_SHADER_SOURCE.contains("fn distribution_ggx"));
    assert!(PBR_SHADER_SOURCE.contains("fn geometry_smith"));
    assert!(PBR_SHADER_SOURCE.contains("fn fresnel_schlick"));
    assert!(PBR_SHADER_SOURCE.contains("textureSample(irradiance_map"));
    assert!(PBR_SHADER_SOURCE.contains("textureSampleLevel(prefiltered_env_map"));
    assert!(PBR_SHADER_SOURCE.contains("textureSample(brdf_lut"));
    assert!(PBR_SHADER_SOURCE.contains("MATERIAL_FLAG_EMISSIVE_MAP"));
    assert!(PBR_SHADER_SOURCE.contains("discard"));
    assert!(PBR_SHADER_SOURCE.contains("ALPHA_MODE_BLEND"));
}

#[test]
fn pbr_cpu_constants_match_shader_literals() {
    assert_eq!(ALPHA_MODE_OPAQUE, 0);
    assert_eq!(ALPHA_MODE_CUTOUT, 1);
    assert_eq!(ALPHA_MODE_BLEND, 2);
    assert_eq!(MATERIAL_FLAG_EMISSIVE_MAP, 1);
    assert_eq!(PBR_PACKED_VERTEX_STRIDE_BYTES, 48);
    assert!(PBR_SHADER_SOURCE.contains("const PBR_PACKED_VERTEX_STRIDE_BYTES : u32 = 48u"));
}
