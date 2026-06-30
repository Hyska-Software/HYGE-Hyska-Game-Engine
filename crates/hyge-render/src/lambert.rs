//! The Lambert lit-sphere render pass (M2 / R-038).
//!
//! M2 (R-038) is the "lit sphere loaded from glTF at
//! runtime" milestone. PBR lands in M3 (R-040); Lambert
//! is the M2 lighting model — `color = base * max(0,
//! dot(N, sun_dir))` plus a small ambient term so the
//! back side of the sphere isn't fully black.
//!
//! The pass is built from a CPU-side `LambertGeometry`
//! (vertices + indices) plus a [`BindlessTable`] (R-037)
//! and a [`GpuMaterial`] that becomes the per-draw
//! uniform. The render graph is wired around a single
//! render pass that binds the lambert shader, the
//! geometry, the MVP uniform, and the material uniform,
//! then draws the geometry with `draw_indexed`.
//!
//! The WGSL shader lives at `src/shader/lambert.wgsl`.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use hyge_render_graph::prelude::*;

use crate::bindless::{BindlessTable, GpuMaterial};

/// The CPU-side vertex for the Lambert pass. Mirrors the
/// `VsIn` struct in `src/shader/lambert.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// XYZ position in local space.
    pub position: [f32; 3],
    /// XYZ normal in local space.
    pub normal: [f32; 3],
}

/// The MVP + model matrix uniform block. Matches the
/// `@group(1)` bindings in `src/shader/lambert.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MvpUniform {
    /// Column-major MVP matrix (clip space).
    pub mvp: [[f32; 4]; 4],
    /// Column-major model matrix (world-from-local).
    pub model: [[f32; 4]; 4],
}

/// The material uniform block. Matches the `Material`
/// struct in `src/shader/lambert.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialUniform {
    /// RGBA base color (the bindless material's base-color
    /// texture sampled, or the constant if no texture is
    /// bound).
    pub base_color: [f32; 4],
    /// XYZ sun direction in world space + scalar pad.
    pub sun_dir: [f32; 4],
    /// 16 bytes of padding to align the struct to 32 bytes
    /// (the WGSL side declares `_pad0.._pad3`).
    pub _pad: [f32; 4],
}

impl MaterialUniform {
    /// Builds a `MaterialUniform` from a bindless
    /// `GpuMaterial` and a sun direction. The base color
    /// is taken from the `GpuMaterial::base_color` field
    /// (which is itself a bindless texture id, but for
    /// the M2 path we just paint a flat color from a
    /// per-test constant; R-040 wires the actual
    /// `textureLoad`).
    pub fn from_bindless(material: &GpuMaterial, sun_dir: [f32; 3], base_color: [f32; 4]) -> Self {
        let _ = material; // bindless material is recorded at the M2 layer; the uniform copy is what the shader reads.
        Self {
            base_color,
            sun_dir: [sun_dir[0], sun_dir[1], sun_dir[2], 0.0],
            _pad: [0.0; 4],
        }
    }
}

/// The WGSL source for the Lambert pass. Embedded at
/// compile time so the pass is self-contained.
pub const SHADER_SOURCE: &str = include_str!("shader/lambert.wgsl");

/// The render pass that draws a lit sphere with Lambert
/// shading. Holds the geometry, the per-frame uniforms, the
/// pipeline, and the bindless material reference. Built via
/// [`LambertPass::new`]; recorded into a render graph via
/// the [`Pass`] trait.
pub struct LambertPass {
    /// Per-frame uniform buffer (MVP). Rebuilt by
    /// [`LambertPass::set_mvp`] each frame.
    mvp_buffer: Arc<wgpu::Buffer>,
    /// Per-frame uniform buffer (model matrix). Rebuilt by
    /// [`LambertPass::set_model`] each frame.
    model_buffer: Arc<wgpu::Buffer>,
    /// Per-frame uniform buffer (material). Rebuilt by
    /// [`LambertPass::set_material`] each frame.
    material_buffer: Arc<wgpu::Buffer>,
    /// The render pipeline. Created at construction time
    /// (it does not change between frames).
    pipeline: Arc<wgpu::RenderPipeline>,
    /// The vertex buffer. Created at construction time
    /// from the CPU-side [`Vertex`] slice.
    vertex_buffer: Arc<wgpu::Buffer>,
    /// The index buffer (u32). Created at construction
    /// time from the CPU-side index slice.
    index_buffer: Arc<wgpu::Buffer>,
    /// The number of indices to draw.
    index_count: u32,
    /// The bindless table. The pass uses the table to
    /// obtain the default samplers + the bindless material
    /// storage buffer; future revisions will read the
    /// material directly from the bindless storage.
    bindless: Arc<BindlessTable>,
    /// The bind group for the lambert shader's `@group(0)`
    /// (MVP, model, material). Rebuilt each frame because
    /// the uniform buffer contents change.
    frame_bind_group: Arc<wgpu::BindGroup>,
    /// The clear color used when recording the pass.
    clear_color: wgpu::Color,
}

impl std::fmt::Debug for LambertPass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LambertPass")
            .field("index_count", &self.index_count)
            .field("bindless", &"<BindlessTable>")
            .finish_non_exhaustive()
    }
}

impl LambertPass {
    /// Constructs a new Lambert pass.
    ///
    /// Allocates the vertex + index buffers from the CPU-side
    /// `vertices` and `indices` slices, builds the render
    /// pipeline, and pre-allocates the per-frame uniform
    /// buffers. The `clear_color` is the color the frame
    /// buffer is cleared to before the geometry is drawn.
    #[allow(
        clippy::too_many_lines,
        reason = "pipeline construction is intrinsically verbose"
    )]
    pub fn new(
        device: &wgpu::Device,
        bindless: Arc<BindlessTable>,
        surface_format: wgpu::TextureFormat,
        clear_color: wgpu::Color,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> Self {
        assert!(
            !vertices.is_empty(),
            "LambertPass requires at least one vertex"
        );
        assert!(
            !indices.is_empty(),
            "LambertPass requires at least one index"
        );
        assert!(
            vertices.len() <= u32::MAX as usize,
            "LambertPass vertex count must fit in u32"
        );
        assert!(
            indices.len() <= u32::MAX as usize,
            "LambertPass index count must fit in u32"
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hyge-render/lambert"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        // Single bind group: MVP, model, and material
        // (matches the `@group(0)` bindings in
        // `src/shader/lambert.wgsl`).
        let lambert_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hyge-render/lambert-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hyge-render/lambert-pipeline-layout"),
            bind_group_layouts: &[&lambert_layout],
            push_constant_ranges: &[],
        });

        let pipeline = Arc::new(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hyge-render/lambert-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3, // Vertex::position
                            1 => Float32x3, // Vertex::normal
                        ],
                    }],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            }),
        );

        let vertex_buffer = Arc::new(device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("hyge-render/lambert-vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            },
        ));
        let index_buffer = Arc::new(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hyge-render/lambert-indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
        );
        let mvp_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-render/lambert-mvp"),
            size: std::mem::size_of::<MvpUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        let model_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-render/lambert-model"),
            size: std::mem::size_of::<MvpUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        let material_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-render/lambert-material"),
            size: std::mem::size_of::<MaterialUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));

        // We don't have a queue handle in the constructor
        // (the caller owns it), so we skip the pre-write
        // and rely on the first `set_mvp` / `set_model` /
        // `set_material` call from the test to populate
        // the buffers. The bind group is still valid
        // because the buffers are pre-allocated; reading
        // from them before the first `write_buffer` call
        // returns the device's default (zero) contents.
        let _ = (&mvp_buffer, &model_buffer, &material_buffer);

        let frame_bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hyge-render/lambert-frame-bind-group"),
            layout: &lambert_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: mvp_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: model_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: material_buffer.as_entire_binding(),
                },
            ],
        }));

        Self {
            mvp_buffer,
            model_buffer,
            material_buffer,
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            bindless,
            frame_bind_group,
            clear_color,
        }
    }

    /// Updates the per-frame MVP matrix. Called each frame
    /// by the test / app before recording the pass.
    pub fn set_mvp(&self, queue: &wgpu::Queue, mvp: &MvpUniform) {
        queue.write_buffer(&self.mvp_buffer, 0, bytemuck::bytes_of(mvp));
    }

    /// Updates the per-frame model matrix. The MVP and
    /// model are kept in separate buffers because the
    /// shader uses the model matrix for the normal
    /// transform (with the cheap `(model * vec4(n, 0)).xyz`
    /// form, which is exact for a rigid body but wrong for
    /// a non-uniform scale — R-040 will switch to the
    /// inverse-transpose).
    pub fn set_model(&self, queue: &wgpu::Queue, model: &MvpUniform) {
        queue.write_buffer(&self.model_buffer, 0, bytemuck::bytes_of(model));
    }

    /// Updates the per-frame material uniform. The M2 path
    /// takes a `MaterialUniform` directly; R-040 will
    /// replace this with a per-frame read from the
    /// bindless material storage.
    pub fn set_material(&self, queue: &wgpu::Queue, material: &MaterialUniform) {
        queue.write_buffer(&self.material_buffer, 0, bytemuck::bytes_of(material));
    }

    /// Returns the bindless table. Mostly used by tests
    /// to inspect slot allocations after the pass is
    /// recorded.
    #[must_use]
    pub fn bindless(&self) -> &BindlessTable {
        &self.bindless
    }

    /// Returns the index count. Useful for tests that
    /// need to assert the geometry was actually drawn.
    #[must_use]
    pub fn index_count(&self) -> u32 {
        self.index_count
    }
}

impl Pass for LambertPass {
    fn name(&self) -> &str {
        "lambert"
    }

    fn reads(&self) -> Vec<ResourceHandle> {
        Vec::new()
    }

    fn writes(&self) -> Vec<ResourceHandle> {
        Vec::new()
    }

    fn record(&mut self, ctx: &mut PassContext<'_>) {
        let (encoder, frame) = ctx.encoder_and_frame();
        let Some(frame) = frame else {
            tracing::error!("LambertPass::record requires a FrameContext");
            return;
        };
        let view = frame.surface_view();
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hyge-render/lambert"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.clear_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Generates a unit-sphere mesh (UV sphere, `latitude` ×
/// `longitude` segments). Returns `(vertices, indices)`.
/// The sphere is centred at the origin with radius 1;
/// the Lambert test scales it via the model matrix.
///
/// # Panics
///
/// Panics if `latitude < 2` or `longitude < 3` (a UV
/// sphere needs at least one ring and three segments to
/// be a valid triangle mesh).
#[must_use]
pub fn make_uv_sphere(latitude: u32, longitude: u32) -> (Vec<Vertex>, Vec<u32>) {
    assert!(latitude >= 2, "UV sphere needs latitude >= 2");
    assert!(longitude >= 3, "UV sphere needs longitude >= 3");

    let mut vertices = Vec::with_capacity(((latitude + 1) * (longitude + 1)) as usize);
    for lat in 0..=latitude {
        let theta = std::f32::consts::PI * (lat as f32) / (latitude as f32);
        let (sin_t, cos_t) = theta.sin_cos();
        for lon in 0..=longitude {
            let phi = 2.0 * std::f32::consts::PI * (lon as f32) / (longitude as f32);
            let (sin_p, cos_p) = phi.sin_cos();
            let x = sin_t * cos_p;
            let y = cos_t;
            let z = sin_t * sin_p;
            vertices.push(Vertex {
                position: [x, y, z],
                normal: [x, y, z],
            });
        }
    }

    let mut indices = Vec::with_capacity((latitude * longitude * 6) as usize);
    for lat in 0..latitude {
        for lon in 0..longitude {
            let first = lat * (longitude + 1) + lon;
            let second = first + (longitude + 1);
            indices.push(first);
            indices.push(second);
            indices.push(first + 1);
            indices.push(second);
            indices.push(second + 1);
            indices.push(first + 1);
        }
    }

    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UV-sphere generator produces a non-empty mesh
    /// with the expected vertex + index counts.
    #[test]
    fn uv_sphere_has_expected_counts() {
        let (v, i) = make_uv_sphere(8, 16);
        assert_eq!(v.len(), 9 * 17);
        // 8 latitude rings × 16 longitude segments × 2
        // triangles × 3 indices per triangle.
        assert_eq!(i.len(), 8 * 16 * 6);
    }

    /// Every vertex of the UV sphere lies on the unit
    /// sphere (radius 1).
    #[test]
    fn uv_sphere_vertices_lie_on_unit_sphere() {
        let (v, _) = make_uv_sphere(4, 8);
        for vertex in &v {
            let r2 = vertex.position[0] * vertex.position[0]
                + vertex.position[1] * vertex.position[1]
                + vertex.position[2] * vertex.position[2];
            assert!(
                (r2 - 1.0).abs() < 1e-5,
                "vertex not on unit sphere: position={:?} r2={r2}",
                vertex.position
            );
        }
    }

    /// `MaterialUniform::from_bindless` copies the sun
    /// direction and base color from a bindless
    /// `GpuMaterial`.
    #[test]
    fn material_uniform_from_bindless_copies_fields() {
        let material = GpuMaterial {
            base_color: 7,
            normal: 0,
            mr: 0,
            occlusion: 0,
            emissive: 0,
            roughness: 0.5,
            metallic: 0.5,
            alpha_mode: 0,
            flags: 0,
        };
        let sun = [0.0, 1.0, 0.0];
        let base = [0.9, 0.1, 0.2, 1.0];
        let u = MaterialUniform::from_bindless(&material, sun, base);
        assert_eq!(u.base_color, base);
        assert_eq!(u.sun_dir, [sun[0], sun[1], sun[2], 0.0]);
        assert_eq!(u._pad, [0.0; 4]);
    }

    /// The WGSL shader source embeds the expected
    /// vertex / fragment entry points.
    #[test]
    fn shader_source_contains_required_entry_points() {
        assert!(SHADER_SOURCE.contains("@vertex"));
        assert!(SHADER_SOURCE.contains("@fragment"));
        assert!(SHADER_SOURCE.contains("vs_main"));
        assert!(SHADER_SOURCE.contains("fs_main"));
    }
}
