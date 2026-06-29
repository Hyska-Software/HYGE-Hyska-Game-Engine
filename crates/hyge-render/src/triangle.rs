//! The first-triangle render pass.
//!
//! R-024 wires a single [`Pass`] implementation that clears
//! the surface (or an off-screen render target) to the
//! user-supplied clear color and draws a hardcoded
//! red/green/blue triangle to its center. The pass owns the
//! `wgpu::RenderPipeline` and vertex buffer (cloned from the
//! `Renderer`'s pre-built instances), so the render-graph layer
//! only has to route the per-frame `FrameContext` to it.
//!
//! The WGSL shader is embedded as a `&str` constant; naga
//! (used internally by wgpu) compiles it to SPIR-V / DXIL /
//! MSL at `create_render_pipeline` time, so there is no
//! build.rs / pre-compile step.

use wgpu::util::DeviceExt;

use hyge_render_graph::prelude::*;

/// The vertex layout: 2-D clip-space position + 3-channel vertex
/// color. The WGSL shader expects exactly this layout at
/// `@location(0)` and `@location(1)`.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// 2-D clip-space position. `z` is supplied by the shader
    /// (always 0); `w` is always 1.
    pub position: [f32; 2],
    /// Per-vertex RGB color. Interpolated by the rasterizer and
    /// written to the color attachment unmodified.
    pub color: [f32; 3],
}

/// The three vertices of the hardcoded triangle. Centred at the
/// origin in clip space; spans the top half of the screen
/// (apex at `y = +0.5`, base at `y = -0.5`).
pub const VERTICES: &[Vertex] = &[
    Vertex { position: [ 0.0,  0.5], color: [1.0, 0.0, 0.0] }, // top,    red
    Vertex { position: [-0.5, -0.5], color: [0.0, 1.0, 0.0] }, // bottom, green
    Vertex { position: [ 0.5, -0.5], color: [0.0, 0.0, 1.0] }, // bottom, blue
];

/// The vertex+fragment WGSL shader. Loaded into a
/// `wgpu::ShaderModule` at pipeline construction time.
///
/// The shader is intentionally minimal — no uniforms, no
/// depth test, no instancing. R-040+ replace it with the
/// clustered-forward PBR shader.
pub const SHADER_SOURCE: &str = include_str!("../shader/triangle.wgsl");

/// The render pass that clears the current frame's color
/// attachment and draws the hardcoded triangle.
///
/// Constructed by [`Renderer::triangle_pass`](crate::Renderer::triangle_pass).
/// The pass owns clones of the renderer's pre-built
/// `wgpu::RenderPipeline` and `wgpu::Buffer` (both are
/// reference-counted internally by wgpu, so the clones are
/// cheap).
pub struct TrianglePass {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    clear_color: wgpu::Color,
}

impl TrianglePass {
    /// Builds the wgpu objects the pass needs: the shader
    /// module, the render pipeline, and the vertex buffer. The
    /// `surface_format` must match the swapchain (or the
    /// off-screen render target, in the test path) the pass
    /// will render to.
    pub fn create(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        clear_color: wgpu::Color,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hyge-render/triangle"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hyge-render/triangle"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, // Vertex::position
                        1 => Float32x3, // Vertex::color
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
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hyge-render/triangle-vertices"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            pipeline,
            vertex_buffer,
            clear_color,
        }
    }

    /// Returns the clear color this pass will use. (Convenience
    /// accessor for tests that want to compare against a
    /// reference clear color.)
    #[must_use]
    pub fn clear_color(&self) -> wgpu::Color {
        self.clear_color
    }
}

impl Pass for TrianglePass {
    fn name(&self) -> &str {
        "triangle"
    }

    fn reads(&self) -> Vec<ResourceHandle> {
        // The pass writes to the frame target only; the render
        // graph treats the swapchain / render target as an
        // implicit side-effect of `record()` rather than a
        // tracked resource. If we later want the barrier
        // inference to track the color attachment, we'd add a
        // "current frame target" handle here.
        Vec::new()
    }

    fn writes(&self) -> Vec<ResourceHandle> {
        Vec::new()
    }

    fn record(&mut self, ctx: &mut PassContext<'_>) {
        // The windowed `Renderer::render_frame` path always
        // hands the pass a `FrameContext`; the headless test
        // path does not, and is exercised via
        // `Renderer::render_triangle_to_texture` which routes
        // the off-screen render-target view through a custom
        // test harness rather than through this pass.
        let frame = ctx
            .frame()
            .expect("TrianglePass::record requires a FrameContext (use Renderer::render_triangle)");
        let view = frame.surface_view();

        let mut render_pass = ctx.encoder().begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hyge-render/triangle"),
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
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_is_pod_and_zeroable() {
        // Compile-time guarantee that `Vertex` can be cast to
        // bytes for the GPU upload.
        let v = Vertex { position: [0.5, -0.5], color: [0.25, 0.5, 0.75] };
        let bytes = bytemuck::bytes_of(&v);
        assert_eq!(bytes.len(), std::mem::size_of::<Vertex>());
        let v2: Vertex = bytemuck::from_bytes(bytes);
        assert_eq!(v.position, v2.position);
        assert_eq!(v.color, v2.color);
    }

    #[test]
    fn vertices_form_a_centered_triangle() {
        // Bounding box: x in [-0.5, 0.5], y in [-0.5, 0.5]
        // (centred at the origin in clip space).
        let mut x_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        let mut y_min = f32::INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        for v in VERTICES {
            x_min = x_min.min(v.position[0]);
            x_max = x_max.max(v.position[0]);
            y_min = y_min.min(v.position[1]);
            y_max = y_max.max(v.position[1]);
        }
        assert_eq!(x_min, -0.5);
        assert_eq!(x_max,  0.5);
        assert_eq!(y_min, -0.5);
        assert_eq!(y_max,  0.5);
    }

    #[test]
    fn shader_source_is_non_empty() {
        // Smoke test: make sure the file was `include_str!`d at
        // compile time. If the file is missing the build fails
        // before this test runs, so a "passes" assertion is
        // enough.
        assert!(!SHADER_SOURCE.is_empty());
        assert!(SHADER_SOURCE.contains("@vertex"));
        assert!(SHADER_SOURCE.contains("@fragment"));
    }
}
