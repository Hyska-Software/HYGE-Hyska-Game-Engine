//! glTF 2.0 parser for the Hyge importer.
//!
//! Supports the core glTF 2.0 spec plus the extensions explicitly
//! named in the R-034 acceptance: `KHR_mesh_quantization` (built into
//! the glTF 2.0 accessor model and so first-class here),
//! `KHR_texture_basisu` (recognised, with KTX2 blobs preserved
//! verbatim when present), and `KHR_lights_punctual` (extracted
//! into the scene summary).
//!
//! The parser produces an intermediate [`GltfScene`] that the writer
//! pipeline turns into `.hyge-mesh` / `.hyge-mat` / `.ktx2` /
//! `.hyge-meta.json` files. Keeping the parse step separate from the
//! write step lets R-035 (meshlet bake) and R-036 (KTX2 transcode)
//! drop in without re-reading the source.

use std::path::Path;

use hyge_core::result::{HygeError, HygeResult};

use crate::importer::material::MaterialData;
use crate::importer::mesh::{MeshData, Vertex};
use crate::importer::texture::TextureFormat;

/// High-level glTF 2.0 scene summary used by the meta document and
/// the `inspect` command.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SceneSummary {
    /// Number of mesh nodes.
    pub mesh_count: u32,
    /// Number of mesh primitives across all meshes.
    pub primitive_count: u32,
    /// Number of unique materials referenced by primitives.
    pub material_count: u32,
    /// Number of unique textures referenced by materials.
    pub texture_count: u32,
    /// Number of `KHR_lights_punctual` lights declared on nodes.
    pub light_count: u32,
}

/// One mesh primitive decoded from a glTF accessor pair.
#[derive(Debug, Clone)]
pub struct PrimitiveData {
    /// Decoded vertices, with positions / normals / uvs filled in
    /// when the primitive declared those attributes.
    pub vertices: Vec<Vertex>,
    /// Triangle index list, decoded from the primitive's indices
    /// accessor.
    pub indices: Vec<u32>,
}

/// Decoded glTF scene ready to be written to the project cache.
pub struct GltfScene {
    /// First mesh primitive. The R-034 milestone writes a single
    /// mesh per import; multi-mesh / multi-primitive glTFs are
    /// preserved by the R-035 refactor that introduces per-mesh
    /// content hashing.
    pub mesh: MeshData,
    /// Materials extracted from the document, in declaration order.
    pub materials: Vec<MaterialData>,
    /// Image bytes keyed by glTF image index, in declaration order.
    /// The orchestrator decides whether to write each as a passthrough
    /// `.ktx2` (R-034) or skip it.
    pub images: Vec<ImageData>,
    /// `KHR_lights_punctual` count, 0 when the extension is absent.
    pub light_count: u32,
    /// Document-level scene summary.
    pub summary: SceneSummary,
}

/// One image extracted from the glTF document, including the
/// detected mime type and the decoded pixel data the importer
/// will write to `<hash>.ktx2` in the cache.
#[derive(Debug, Clone)]
pub struct ImageData {
    /// Lowercase mime type without parameters (e.g. `image/png`).
    pub mime: &'static str,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Pixel format.
    pub format: TextureFormat,
    /// Decoded pixel data, 8 bits per channel, tightly packed.
    pub pixels: Vec<u8>,
}

/// Extension names explicitly recognised by the R-034 importer.
pub const KHR_LIGHTS_PUNCTUAL: &str = "KHR_lights_punctual";
/// Hint constant. `KHR_mesh_quantization` is built into the glTF 2.0
/// accessor model, so it is always supported by the [`gltf`] crate
/// (and therefore by us) without a feature flag.
pub const KHR_MESH_QUANTIZATION: &str = "KHR_mesh_quantization";
/// Recognised but not yet fully transcoded in R-034. R-036 reads
/// the KTX2 blobs we preserve here and transcodes them to BasisU.
pub const KHR_TEXTURE_BASISU: &str = "KHR_texture_basisu";

/// Loads and parses a glTF 2.0 file from a byte slice.
///
/// `bytes` must be the complete file content (`.gltf` JSON or
/// `.glb` binary). The parser uses the [`gltf`] crate's `import`
/// feature to inline base64 / GLB buffer and image data so callers
/// do not have to follow `data:` URIs themselves.
///
/// # Errors
///
/// Returns [`HygeError::Parse`] for malformed glTF / GLB input,
/// missing referenced buffers, or unsupported required
/// extensions. [`HygeError::Io`] is reserved for filesystem reads
/// performed by the orchestrator before the parser is invoked.
pub fn parse(bytes: &[u8], _source: &Path) -> HygeResult<GltfScene> {
    let (document, buffers, images) =
        gltf::import_slice(bytes).map_err(|e| HygeError::parse(format!("gltf parse: {e}")))?;

    let light_count = document
        .lights()
        .map(|lights| lights.count() as u32)
        .unwrap_or(0);

    // -- meshes / primitives -----------------------------------------
    let mut all_primitives: Vec<PrimitiveData> = Vec::new();
    let mut material_set: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let p = decode_primitive(&primitive, &buffers)?;
            material_set.insert(primitive.material().index().unwrap_or(0));
            all_primitives.push(p);
        }
    }

    // Single mesh per import for R-034. Vertices/indices are
    // concatenated in primitive-declaration order. R-035 will
    // split them into per-primitive content-addressed entries.
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for p in &all_primitives {
        let base = vertices.len() as u32;
        vertices.extend_from_slice(&p.vertices);
        indices.extend(p.indices.iter().map(|i| i + base));
    }
    // R-035: bake the meshlet stream + LOD chain through
    // `meshopt`. The bake is a no-op on empty input (handled
    // inside [`MeshData::bake`]) so the empty placeholder is
    // preserved end-to-end.
    let mesh_data = MeshData::bake(vertices, indices)?;

    // -- materials ----------------------------------------------------
    let mut materials: Vec<MaterialData> = Vec::new();
    for (idx, mat) in document.materials().enumerate() {
        if !material_set.contains(&idx) {
            continue;
        }
        materials.push(decode_material(&mat));
    }
    if materials.is_empty() {
        materials.push(MaterialData::default());
    }

    // -- images (passthrough) ----------------------------------------
    let mut image_records: Vec<ImageData> = Vec::new();
    for (img, data) in document.images().zip(images.into_iter()) {
        let source_mime = match img.source() {
            gltf::image::Source::View { mime_type, .. } => mime_type_for(Some(mime_type)),
            gltf::image::Source::Uri { mime_type, .. } => mime_type_for(mime_type),
        };
        let format = gltf_format_to_hyge(data.format);
        image_records.push(ImageData {
            mime: source_mime,
            width: data.width,
            height: data.height,
            format,
            pixels: data.pixels,
        });
    }

    // -- scene summary ------------------------------------------------
    let summary = SceneSummary {
        mesh_count: document.meshes().count() as u32,
        primitive_count: all_primitives.len() as u32,
        material_count: materials.len() as u32,
        texture_count: image_records.len() as u32,
        light_count,
    };

    Ok(GltfScene {
        mesh: mesh_data,
        materials,
        images: image_records,
        light_count,
        summary,
    })
}

fn decode_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
) -> HygeResult<PrimitiveData> {
    let positions = primitive
        .get(&gltf::Semantic::Positions)
        .ok_or_else(|| HygeError::parse("primitive missing POSITION attribute"))?;
    let position_data = read_vec3_f32(positions, buffers, "POSITION")?;
    let count = position_data.len();

    let normals = primitive.get(&gltf::Semantic::Normals);
    let normal_data = if let Some(acc) = normals {
        Some(read_vec3_f32(acc, buffers, "NORMAL")?)
    } else {
        None
    };

    let uvs = primitive.get(&gltf::Semantic::TexCoords(0));
    let uv_data = if let Some(acc) = uvs {
        Some(read_vec2_f32(acc, buffers, "TEXCOORD_0")?)
    } else {
        None
    };

    let indices = match primitive.indices() {
        Some(acc) => read_indices_u32(acc, buffers)?,
        None => (0..count as u32).collect(),
    };

    let mut vertices: Vec<Vertex> = Vec::with_capacity(count);
    for i in 0..count {
        let position = position_data[i];
        let normal = normal_data
            .as_ref()
            .map(|n| n[i])
            .unwrap_or([0.0, 0.0, 1.0]);
        let uv = uv_data.as_ref().map(|u| u[i]).unwrap_or([0.0, 0.0]);
        vertices.push(Vertex {
            position,
            normal,
            uv,
        });
    }

    Ok(PrimitiveData { vertices, indices })
}

fn decode_material(mat: &gltf::Material) -> MaterialData {
    let pbr = mat.pbr_metallic_roughness();
    let base_color = pbr.base_color_factor();
    let base_color_texture = pbr
        .base_color_texture()
        .map(|t| t.texture().index().to_string());
    let metallic_roughness_texture = pbr
        .metallic_roughness_texture()
        .map(|t| t.texture().index().to_string());
    let normal_texture = mat
        .normal_texture()
        .map(|t| t.texture().index().to_string());
    let occlusion_texture = mat
        .occlusion_texture()
        .map(|t| t.texture().index().to_string());
    let emissive_texture = mat
        .emissive_texture()
        .map(|t| t.texture().index().to_string());

    MaterialData {
        name: mat.name().unwrap_or("").to_string(),
        base_color: [base_color[0], base_color[1], base_color[2], base_color[3]],
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        emissive: {
            let e = mat.emissive_factor();
            [e[0], e[1], e[2]]
        },
        double_sided: mat.double_sided(),
        base_color_texture,
        metallic_roughness_texture,
        normal_texture,
        occlusion_texture,
        emissive_texture,
    }
}

fn read_vec3_f32(
    accessor: gltf::Accessor,
    buffers: &[gltf::buffer::Data],
    label: &str,
) -> HygeResult<Vec<[f32; 3]>> {
    if accessor.dimensions() != gltf::accessor::Dimensions::Vec3 {
        return Err(HygeError::parse(format!("{label} accessor must be VEC3")));
    }
    let view = accessor
        .view()
        .ok_or_else(|| HygeError::parse(format!("{label} accessor has no buffer view")))?;
    let buf: &[u8] = buffers[view.buffer().index()].as_ref();
    let offset = view.offset() + accessor.offset();
    let stride = view.stride().unwrap_or(accessor.size());
    let count = accessor.count();

    // KHR_mesh_quantization: when the accessor carries min/max, use them
    // to dequantize into the original float range. When absent, fall
    // back to the type's natural normalized range.
    let min = accessor.min();
    let max = accessor.max();
    let min = min.as_ref().and_then(json_value_to_floats);
    let max = max.as_ref().and_then(json_value_to_floats);
    let quant =
        QuantizationContext::new(accessor.data_type(), min.as_deref(), max.as_deref(), label)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = offset + i * stride;
        let end = start + accessor.size();
        if end > buf.len() {
            return Err(HygeError::parse(format!("{label} accessor out of range")));
        }
        let bytes = &buf[start..end];
        let v = quant.dequantize_vec3(bytes)?;
        out.push(v);
    }
    Ok(out)
}

fn read_vec2_f32(
    accessor: gltf::Accessor,
    buffers: &[gltf::buffer::Data],
    label: &str,
) -> HygeResult<Vec<[f32; 2]>> {
    if accessor.dimensions() != gltf::accessor::Dimensions::Vec2 {
        return Err(HygeError::parse(format!("{label} accessor must be VEC2")));
    }
    let view = accessor
        .view()
        .ok_or_else(|| HygeError::parse(format!("{label} accessor has no buffer view")))?;
    let buf: &[u8] = buffers[view.buffer().index()].as_ref();
    let offset = view.offset() + accessor.offset();
    let stride = view.stride().unwrap_or(accessor.size());
    let count = accessor.count();

    let min = accessor.min();
    let max = accessor.max();
    let min = min.as_ref().and_then(json_value_to_floats);
    let max = max.as_ref().and_then(json_value_to_floats);
    let quant =
        QuantizationContext::new(accessor.data_type(), min.as_deref(), max.as_deref(), label)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = offset + i * stride;
        let end = start + accessor.size();
        if end > buf.len() {
            return Err(HygeError::parse(format!("{label} accessor out of range")));
        }
        let bytes = &buf[start..end];
        let v = quant.dequantize_vec2(bytes)?;
        out.push(v);
    }
    Ok(out)
}

fn read_indices_u32(
    accessor: gltf::Accessor,
    buffers: &[gltf::buffer::Data],
) -> HygeResult<Vec<u32>> {
    let view = accessor
        .view()
        .ok_or_else(|| HygeError::parse("indices accessor has no buffer view"))?;
    let buf: &[u8] = buffers[view.buffer().index()].as_ref();
    let offset = view.offset() + accessor.offset();
    let stride = view.stride().unwrap_or(accessor.size());
    let count = accessor.count();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let start = offset + i * stride;
        let end = start + accessor.size();
        if end > buf.len() {
            return Err(HygeError::parse("indices accessor out of range"));
        }
        let bytes = &buf[start..end];
        let value: u32 = match accessor.data_type() {
            gltf::accessor::DataType::U32 => u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            gltf::accessor::DataType::U16 => {
                u16::from_le_bytes(bytes[0..2].try_into().unwrap()) as u32
            }
            gltf::accessor::DataType::U8 => bytes[0] as u32,
            other => {
                return Err(HygeError::parse(format!(
                    "unsupported index data type: {other:?}"
                )))
            }
        };
        out.push(value);
    }
    Ok(out)
}

/// Extracts a flat `Vec<f32>` from a glTF `min` / `max` JSON value
/// (an array of numbers). Returns `None` for any other shape so
/// the dequantizer can fall back to the type's natural range.
fn json_value_to_floats(v: &serde_json::Value) -> Option<Vec<f32>> {
    let arr = v.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let n = item.as_f64()?;
        if !n.is_finite() {
            return None;
        }
        out.push(n as f32);
    }
    Some(out)
}

fn mime_type_for(mime: Option<&str>) -> &'static str {
    match mime {
        Some("image/png") => "image/png",
        Some("image/jpeg") => "image/jpeg",
        Some("image/ktx2") => "image/ktx2",
        Some("image/ktx") => "image/ktx2",
        _ => "image/png",
    }
}

/// Decoding context for a single glTF accessor.
///
/// `KHR_mesh_quantization` lets accessors store integer-typed values
/// (I8 / U8 / I16 / U16) instead of F32. The conversion back to the
/// original float range is:
///
/// ```text
/// actual = mix(target_min, target_max, (quant - quant_min) / (quant_max - quant_min))
/// ```
///
/// When the accessor carries an explicit `min` / `max`, those become
/// the target range (this is the POSITION case). Otherwise the
/// type's natural normalized range is used (NORMAL / TANGENT
/// typically use `[-1, 1]`; TEXCOORD_0 uses `[0, 1]`).
struct QuantizationContext {
    data_type: gltf::accessor::DataType,
    /// `(quant_min, quant_max, target_min, target_max)` per
    /// component, with a length matching the accessor's
    /// component count.
    components: Vec<[f32; 4]>,
    label: String,
}

impl QuantizationContext {
    fn new(
        data_type: gltf::accessor::DataType,
        min: Option<&[f32]>,
        max: Option<&[f32]>,
        label: &str,
    ) -> HygeResult<Self> {
        let (quant_min, quant_max) = match data_type {
            gltf::accessor::DataType::F32 => return Ok(Self::f32(label)),
            gltf::accessor::DataType::I8 => (-128.0, 127.0),
            gltf::accessor::DataType::U8 => (0.0, 255.0),
            gltf::accessor::DataType::I16 => (-32768.0, 32767.0),
            gltf::accessor::DataType::U16 => (0.0, 65535.0),
            other => {
                return Err(HygeError::parse(format!(
                    "unsupported accessor data type for {label}: {other:?}"
                )));
            }
        };

        // Determine the target range: explicit min/max when both are
        // present, otherwise the natural normalized range. For
        // signed integer types the natural range is [-1, 1]; for
        // unsigned integer types it is [0, 1].
        let natural = match data_type {
            gltf::accessor::DataType::I8 | gltf::accessor::DataType::I16 => (-1.0, 1.0),
            _ => (0.0, 1.0),
        };

        let components = match (min, max) {
            (Some(m), Some(max_arr)) => {
                if m.len() != max_arr.len() {
                    return Err(HygeError::parse(format!(
                        "{label} accessor min/max length mismatch: {} vs {}",
                        m.len(),
                        max_arr.len()
                    )));
                }
                m.iter()
                    .zip(max_arr.iter())
                    .map(|(lo, hi)| [quant_min, quant_max, *lo, *hi])
                    .collect()
            }
            _ => vec![[quant_min, quant_max, natural.0, natural.1]],
        };

        Ok(Self {
            data_type,
            components,
            label: label.to_string(),
        })
    }

    fn f32(label: &str) -> Self {
        Self {
            data_type: gltf::accessor::DataType::F32,
            components: Vec::new(),
            label: label.to_string(),
        }
    }

    fn dequantize_vec3(&self, bytes: &[u8]) -> HygeResult<[f32; 3]> {
        match self.data_type {
            gltf::accessor::DataType::F32 => {
                let arr: [[u8; 4]; 3] = bytemuck::cast_slice::<u8, [u8; 4]>(bytes)[0..3]
                    .try_into()
                    .map_err(|_| self.size_mismatch(12))?;
                Ok([
                    f32::from_le_bytes(arr[0]),
                    f32::from_le_bytes(arr[1]),
                    f32::from_le_bytes(arr[2]),
                ])
            }
            gltf::accessor::DataType::I8 => {
                let arr: [i8; 3] = [bytes[0] as i8, bytes[1] as i8, bytes[2] as i8];
                self.dequantize_i(arr.map(f32::from))
            }
            gltf::accessor::DataType::U8 => {
                let arr: [u8; 3] = [bytes[0], bytes[1], bytes[2]];
                self.dequantize_u(arr.map(f32::from))
            }
            gltf::accessor::DataType::I16 => {
                let arr: [i16; 3] = [
                    i16::from_le_bytes([bytes[0], bytes[1]]),
                    i16::from_le_bytes([bytes[2], bytes[3]]),
                    i16::from_le_bytes([bytes[4], bytes[5]]),
                ];
                self.dequantize_i(arr.map(f32::from))
            }
            gltf::accessor::DataType::U16 => {
                let arr: [u16; 3] = [
                    u16::from_le_bytes([bytes[0], bytes[1]]),
                    u16::from_le_bytes([bytes[2], bytes[3]]),
                    u16::from_le_bytes([bytes[4], bytes[5]]),
                ];
                self.dequantize_u(arr.map(f32::from))
            }
            other => Err(HygeError::parse(format!(
                "{} accessor data type {other:?} is not supported for VEC3 attributes",
                self.label
            ))),
        }
    }

    fn dequantize_vec2(&self, bytes: &[u8]) -> HygeResult<[f32; 2]> {
        match self.data_type {
            gltf::accessor::DataType::F32 => {
                let arr: [[u8; 4]; 2] = bytemuck::cast_slice::<u8, [u8; 4]>(bytes)[0..2]
                    .try_into()
                    .map_err(|_| self.size_mismatch(8))?;
                Ok([f32::from_le_bytes(arr[0]), f32::from_le_bytes(arr[1])])
            }
            gltf::accessor::DataType::I8 => {
                let arr: [i8; 2] = [bytes[0] as i8, bytes[1] as i8];
                self.dequantize_i2(arr.map(f32::from))
            }
            gltf::accessor::DataType::U8 => {
                let arr: [u8; 2] = [bytes[0], bytes[1]];
                self.dequantize_u2(arr.map(f32::from))
            }
            gltf::accessor::DataType::I16 => {
                let arr: [i16; 2] = [
                    i16::from_le_bytes([bytes[0], bytes[1]]),
                    i16::from_le_bytes([bytes[2], bytes[3]]),
                ];
                self.dequantize_i2(arr.map(f32::from))
            }
            gltf::accessor::DataType::U16 => {
                let arr: [u16; 2] = [
                    u16::from_le_bytes([bytes[0], bytes[1]]),
                    u16::from_le_bytes([bytes[2], bytes[3]]),
                ];
                self.dequantize_u2(arr.map(f32::from))
            }
            other => Err(HygeError::parse(format!(
                "{} accessor data type {other:?} is not supported for VEC2 attributes",
                self.label
            ))),
        }
    }

    fn dequantize_i(&self, q: [f32; 3]) -> HygeResult<[f32; 3]> {
        self.dequantize_n(q, &q_i_default_range())
    }

    fn dequantize_u(&self, q: [f32; 3]) -> HygeResult<[f32; 3]> {
        self.dequantize_n(q, &q_u_default_range())
    }

    fn dequantize_i2(&self, q: [f32; 2]) -> HygeResult<[f32; 2]> {
        self.dequantize_n2(q, &q_i_default_range())
    }

    fn dequantize_u2(&self, q: [f32; 2]) -> HygeResult<[f32; 2]> {
        self.dequantize_n2(q, &q_u_default_range())
    }

    fn dequantize_n(&self, q: [f32; 3], default: &[f32; 4]) -> HygeResult<[f32; 3]> {
        let c = self.components_for(3, default)?;
        Ok([lerp(c[0], q[0]), lerp(c[1], q[1]), lerp(c[2], q[2])])
    }

    fn dequantize_n2(&self, q: [f32; 2], default: &[f32; 4]) -> HygeResult<[f32; 2]> {
        let c = self.components_for(2, default)?;
        Ok([lerp(c[0], q[0]), lerp(c[1], q[1])])
    }

    fn components_for(&self, expected: usize, default: &[f32; 4]) -> HygeResult<Vec<[f32; 4]>> {
        if self.components.is_empty() {
            // F32 path: never reaches here. For non-F32 types we
            // always populate `components` in `new`.
            return Err(HygeError::parse(format!(
                "{} quantization context is empty",
                self.label
            )));
        }
        if self.components.len() == 1 {
            // The accessor had no min/max: one shared range
            // applies to every component.
            Ok(vec![*default; expected])
        } else if self.components.len() == expected {
            Ok(self.components.clone())
        } else {
            Err(HygeError::parse(format!(
                "{} accessor min/max length {} does not match expected {}",
                self.label,
                self.components.len(),
                expected
            )))
        }
    }

    fn size_mismatch(&self, expected: usize) -> HygeError {
        HygeError::parse(format!(
            "{} accessor byte slice size mismatch (expected {expected})",
            self.label
        ))
    }
}

#[inline]
fn lerp(range: [f32; 4], q: f32) -> f32 {
    let [q_min, q_max, t_min, t_max] = range;
    let span = q_max - q_min;
    if span == 0.0 {
        return t_min;
    }
    let t = (q - q_min) / span;
    t_min + t * (t_max - t_min)
}

fn q_i_default_range() -> [f32; 4] {
    [-1.0, 1.0, -1.0, 1.0]
}

fn q_u_default_range() -> [f32; 4] {
    [0.0, 1.0, 0.0, 1.0]
}

fn gltf_format_to_hyge(f: gltf::image::Format) -> TextureFormat {
    match f {
        gltf::image::Format::R8 => TextureFormat::R8,
        gltf::image::Format::R8G8 => TextureFormat::R8G8,
        gltf::image::Format::R8G8B8 => TextureFormat::R8G8B8,
        gltf::image::Format::R8G8B8A8 => TextureFormat::R8G8B8A8,
        gltf::image::Format::R16 => TextureFormat::R16,
        gltf::image::Format::R16G16 => TextureFormat::R16G16,
        gltf::image::Format::R16G16B16 | gltf::image::Format::R16G16B16A16 => {
            TextureFormat::R16G16B16A16
        }
        gltf::image::Format::R32G32B32FLOAT | gltf::image::Format::R32G32B32A32FLOAT => {
            TextureFormat::R32G32B32A32FLOAT
        }
    }
}
