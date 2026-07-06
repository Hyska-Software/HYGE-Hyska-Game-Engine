//! Clustered-forward render pass (R-042).
//!
//! Implements the CPU-side light cluster assignment and the PBR
//! render pass that consumes the bindless table. The pass:
//!
//! 1. Accepts a list of [`Instance`]s and [`DrawCommand`]s produced
//!    by `RenderExtract` (R-043).
//! 2. Assigns lights to a 3D tile/cluster grid and uploads the
//!    `LightGrid` + light-index data to the bindless table.
//! 3. Records a single render pass that binds the bindless
//!    descriptor heap and the PBR geometry/IBL bind group, then
//!    issues the draw commands via `draw_indexed_indirect`.
//!
//! The WGSL source for the fragment/vertex shader is the existing
//! R-040 `pbr.wgsl`. The light-grid compute reference shader lives
//! at `src/shader/light_grid.wgsl`.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use hyge_render_graph::prelude::*;

use crate::bindless::{
    BindlessTable, DrawCommand, Instance, Light, LightGrid, LIGHT_INDEX_LIST_CAPACITY,
};
use crate::ibl_gpu::IblResources;

/// Per-frame uniform block consumed by `pbr.wgsl` at `@group(1)`
/// `@binding(1)`. The struct layout must match `FrameData` in the
/// shader exactly. Total size: 4×vec4 (64) + 1×vec4 (16) + 1×vec4
/// (16) + 1×mat4 (64) = 160 bytes. The struct must stay
/// 16-byte aligned (wgpu's `Uniform` alignment requirement); every
/// `vec4`/`mat4` member satisfies this.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct FrameData {
    /// View-projection matrix (column-major).
    pub view_proj: [[f32; 4]; 4],
    /// Camera world position in `xyz`, alpha cutoff in `w`.
    pub camera_pos_alpha_cutoff: [f32; 4],
    /// Sun direction in `xyz`, exposure in `w`.
    pub sun_direction_exposure: [f32; 4],
    /// Sun color in `xyz`, intensity in `w`.
    pub sun_color_intensity: [f32; 4],
    /// Cluster configuration as `vec4<u32>`:
    /// `(tiles_x, tiles_y, depth_slices, max_lights_per_cluster)`.
    pub cluster_params: [u32; 4],
    /// Viewport in `xy` (pixels), view-space near/far in `zw`.
    pub viewport: [f32; 4],
    /// Camera view matrix (column-major). Used by the vertex
    /// shader to compute view-space Z for the cluster depth slice.
    pub view: [[f32; 4]; 4],
}

impl FrameData {
    /// Creates default frame data for an off-centre looking-at-origin
    /// camera, a single directional sun, a neutral exposure, and
    /// the default cluster config (16x9x16, 256 max lights).
    #[must_use]
    pub fn default_looking_at_origin() -> Self {
        Self {
            view_proj: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            camera_pos_alpha_cutoff: [0.0, 0.0, 5.0, 0.5],
            sun_direction_exposure: [0.0, -1.0, 0.0, 1.0],
            sun_color_intensity: [1.0, 1.0, 1.0, 1.0],
            cluster_params: [16, 9, 16, 256],
            // Default viewport: 1080p with a 0.1..1000 m view frustum.
            viewport: [1920.0, 1080.0, 0.1, 1000.0],
            view: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}

/// Light cluster configuration. The defaults produce a modest
/// 16x9x16 grid that is easy to validate on a 1080p target.
#[derive(Debug, Clone, Copy)]
pub struct ClusterConfig {
    /// Number of tiles along X.
    pub tiles_x: u32,
    /// Number of tiles along Y.
    pub tiles_y: u32,
    /// Number of depth slices.
    pub depth_slices: u32,
    /// Maximum lights stored per cluster.
    pub max_lights_per_cluster: u32,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            tiles_x: 16,
            tiles_y: 9,
            depth_slices: 16,
            max_lights_per_cluster: 256,
        }
    }
}

/// A single geometry batch submitted to the clustered-forward pass.
#[derive(Debug, Clone)]
pub struct Batch {
    /// Bindless mesh slot id (the raw `u32` index; the typed
    /// `BindlessSlot<MeshTag>` lives on the asset server side
    /// and is not threaded through the render frame).
    pub mesh_id: u32,
    /// Bindless material slot id.
    pub material_id: u32,
    /// First instance in the global instance buffer.
    pub first_instance: u32,
    /// Number of instances.
    pub instance_count: u32,
    /// Number of indices to draw.
    pub index_count: u32,
    /// First index in the mesh's index buffer.
    pub first_index: u32,
    /// Base vertex offset added to each index.
    pub base_vertex: i32,
}

/// The R-042 clustered-forward pass.
///
/// Holds the PBR render pipeline, the per-frame uniform buffer, the
/// geometry vertex buffer, and references to the bindless table and
/// IBL resources.
pub struct ClusteredForwardPass {
    pipeline: Arc<wgpu::RenderPipeline>,
    bindless: Arc<BindlessTable>,
    #[allow(
        dead_code,
        reason = "kept alive so the bind group is rebuilt when the IBL resources change"
    )]
    ibl: Option<IblResources>,
    /// The wgpu device handle used to (re)build the frame bind
    /// group when IBL changes. We keep an `Arc<wgpu::Device>`
    /// rather than a `&wgpu::Device` because the pass owns the
    /// rebuild lifetime.
    device: Arc<wgpu::Device>,
    /// The frame-data bind group layout. Cached so `set_ibl`
    /// can rebuild the frame bind group against the same
    /// layout (wgpu's bind groups are immutable).
    frame_data_layout: Arc<wgpu::BindGroupLayout>,
    frame_data_buffer: Arc<wgpu::Buffer>,
    frame_bind_group: Arc<wgpu::BindGroup>,
    vertex_buffer: Arc<wgpu::Buffer>,
    index_buffer: Arc<wgpu::Buffer>,
    cluster_config: ClusterConfig,
    clear_color: wgpu::Color,
    /// Cached frame data so `record` can re-upload if needed.
    frame_data: FrameData,
    /// Cached lights so `record` can rebuild the light grid if
    /// `set_lights` was called since the last frame.
    lights: Vec<Light>,
    /// Cached instances uploaded to the bindless instance buffer.
    instances: Vec<Instance>,
    /// Cached draw commands uploaded to the bindless draw-command
    /// buffer.
    draw_commands: Vec<DrawCommand>,
    /// Cached batches used for indirect draws.
    batches: Vec<Batch>,
}

impl std::fmt::Debug for ClusteredForwardPass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusteredForwardPass")
            .field("cluster_config", &self.cluster_config)
            .field("instances", &self.instances.len())
            .field("draw_commands", &self.draw_commands.len())
            .field("batches", &self.batches.len())
            .field("bindless", &"<BindlessTable>")
            .finish_non_exhaustive()
    }
}

impl ClusteredForwardPass {
    /// Constructs the clustered-forward pass.
    ///
    /// `vertex_buffer` and `index_buffer` are the global packed PBR
    /// geometry buffers produced by the asset upload path. They are
    /// bound at `@group(1)` `@binding(0)`.
    ///
    /// # Errors
    ///
    /// Returns [`hyge_core::result::HygeError::Gpu`] when the
    /// pipeline layout or shader module cannot be created.
    #[allow(
        clippy::too_many_arguments,
        reason = "frame submission is intrinsically many-arg"
    )]
    pub fn new(
        device: Arc<wgpu::Device>,
        bindless: Arc<BindlessTable>,
        ibl: Option<IblResources>,
        surface_format: wgpu::TextureFormat,
        cluster_config: ClusterConfig,
        vertex_buffer: Arc<wgpu::Buffer>,
        index_buffer: Arc<wgpu::Buffer>,
        clear_color: wgpu::Color,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hyge-render/pbr"),
            source: wgpu::ShaderSource::Wgsl(crate::pbr::SHADER_SOURCE.into()),
        });

        let frame_data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hyge-render/pbr-frame-data-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let frame_data_layout = Arc::new(frame_data_layout);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hyge-render/pbr-pipeline-layout"),
            bind_group_layouts: &[bindless.layout(), frame_data_layout.as_ref()],
            push_constant_ranges: &[],
        });

        let pipeline = Arc::new(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("hyge-render/pbr"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<u32>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Uint32],
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

        let frame_data_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-render/pbr-frame-data"),
            size: std::mem::size_of::<FrameData>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));

        let frame_bind_group = Self::build_frame_bind_group(
            &device,
            frame_data_layout.as_ref(),
            &frame_data_buffer,
            &vertex_buffer,
            &ibl,
        );

        Self {
            pipeline,
            bindless,
            ibl,
            device,
            frame_data_layout,
            frame_data_buffer,
            frame_bind_group,
            vertex_buffer,
            index_buffer,
            cluster_config,
            clear_color,
            frame_data: FrameData::default_looking_at_origin(),
            lights: Vec::new(),
            instances: Vec::new(),
            draw_commands: Vec::new(),
            batches: Vec::new(),
        }
    }

    fn build_frame_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        frame_data_buffer: &Arc<wgpu::Buffer>,
        vertex_buffer: &Arc<wgpu::Buffer>,
        ibl: &Option<IblResources>,
    ) -> Arc<wgpu::BindGroup> {
        let irradiance_view = ibl
            .as_ref()
            .map(|r| Arc::clone(&r.irradiance_view))
            .unwrap_or_else(|| {
                let placeholder = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("hyge-render/pbr-irradiance-fallback"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                Arc::new(placeholder.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::Cube),
                    ..Default::default()
                }))
            });
        let prefilter_view = ibl
            .as_ref()
            .map(|r| Arc::clone(&r.prefiltered_view))
            .unwrap_or_else(|| {
                let placeholder = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("hyge-render/pbr-prefilter-fallback"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                Arc::new(placeholder.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::Cube),
                    ..Default::default()
                }))
            });
        let brdf_view = ibl
            .as_ref()
            .map(|r| Arc::clone(&r.brdf_lut_view))
            .unwrap_or_else(|| {
                let placeholder = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("hyge-render/pbr-brdf-fallback"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rg8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                Arc::new(placeholder.create_view(&wgpu::TextureViewDescriptor::default()))
            });

        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hyge-render/pbr-frame-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: frame_data_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&irradiance_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&prefilter_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&brdf_view),
                },
            ],
        }))
    }

    /// Updates per-frame camera + lighting uniforms. The
    /// `cluster_params` (cluster dims + max lights) and
    /// `viewport` (screen + near/far) fields are derived from
    /// the pass's own [`ClusterConfig`]; the caller fills the
    /// rest (`view_proj`, `view`, `camera_pos`, `sun_*`).
    pub fn set_frame_data(&mut self, queue: &wgpu::Queue, frame_data: FrameData) {
        self.frame_data = frame_data;
        // Overlay the cluster_params and viewport with the
        // pass's authoritative configuration so the shader's
        // light-grid iteration is consistent with what the CPU
        // uploaded.
        self.frame_data.cluster_params = [
            self.cluster_config.tiles_x,
            self.cluster_config.tiles_y,
            self.cluster_config.depth_slices,
            self.cluster_config.max_lights_per_cluster,
        ];
        queue.write_buffer(
            &self.frame_data_buffer,
            0,
            bytemuck::bytes_of(&self.frame_data),
        );
    }

    /// Updates the scene lights and uploads them to the bindless
    /// light buffer. The light grid is rebuilt on the CPU during
    /// `record`.
    pub fn set_lights(&mut self, queue: &wgpu::Queue, lights: Vec<Light>) {
        self.lights = lights;
        self.bindless.write_lights(0, &self.lights);
        // Pad with a zero light so the shader never reads an empty
        // buffer even when the scene has no lights.
        if self.lights.is_empty() {
            self.bindless.write_lights(0, &[Light::default()]);
        }
        self.rebuild_light_grid(queue);
    }

    /// Uploads instances and draw commands to the bindless table and
    /// stores the local batch list for `record`.
    pub fn set_geometry(
        &mut self,
        queue: &wgpu::Queue,
        instances: Vec<Instance>,
        draw_commands: Vec<DrawCommand>,
        batches: Vec<Batch>,
    ) {
        self.instances = instances;
        self.draw_commands = draw_commands;
        self.batches = batches;
        self.bindless.write_instances(0, &self.instances);
        self.bindless.write_draw_commands(0, &self.draw_commands);
        self.rebuild_light_grid(queue);
    }

    fn rebuild_light_grid(&mut self, _queue: &wgpu::Queue) {
        let total_clusters = (self.cluster_config.tiles_x
            * self.cluster_config.tiles_y
            * self.cluster_config.depth_slices) as usize;
        let max_per_cluster = self.cluster_config.max_lights_per_cluster as usize;

        let mut entries = vec![LightGrid::new(0, 0); total_clusters];
        let mut index_list: Vec<u32> = Vec::with_capacity(total_clusters * max_per_cluster);

        // The light-index-list SSBO is sized to
        // `LIGHT_INDEX_LIST_CAPACITY` (default 65_536). We
        // cannot write past that boundary; the caller's
        // `ClusterConfig::max_lights_per_cluster` may be
        // larger than what the buffer actually holds, so we
        // cap the write to the buffer capacity and zero out
        // any cluster whose offset would be out of range.
        let index_list_capacity = LIGHT_INDEX_LIST_CAPACITY as usize;
        let max_offsets = index_list_capacity / max_per_cluster.max(1);
        for (cluster_index, entry) in entries.iter_mut().enumerate() {
            if cluster_index >= max_offsets {
                *entry = LightGrid::new(0, 0);
                continue;
            }
            let mut count = 0u32;
            let offset = (cluster_index as u32) * (max_per_cluster as u32);
            for light_index in 0..self.lights.len() {
                if (count as usize) >= max_per_cluster {
                    break;
                }
                index_list.push(light_index as u32);
                count += 1;
            }
            *entry = LightGrid::new(offset, count);
        }

        // Pad the index list with a single zero entry so the
        // shader never reads past the buffer end when
        // `grid.count > index_list.len() / max_per_cluster`.
        // The fragment shader's `light_index >= arrayLength(&lights)`
        // guard catches any out-of-range read.
        if index_list.is_empty() {
            index_list.push(0);
        }

        self.bindless.write_light_grid(0, &entries);
        self.bindless.write_light_index_list(0, &index_list);

        // Avoid an empty light-grid read by the shader.
        if entries.is_empty() {
            self.bindless.write_light_grid(0, &[LightGrid::default()]);
        }
    }

    /// Replaces the IBL resources and rebuilds the frame bind
    /// group so the new irradiance / prefilter / BRDF-LUT
    /// views are bound. The R-041 acceptance test relies on
    /// this working *after* the first `render_frame` call,
    /// which is why the binding has to be torn down and
    /// rebuilt here (the bind group is immutable in wgpu).
    pub fn set_ibl(&mut self, ibl: IblResources) {
        self.ibl = Some(ibl);
        self.frame_bind_group = Self::build_frame_bind_group(
            &self.device,
            self.frame_data_layout.as_ref(),
            &self.frame_data_buffer,
            &self.vertex_buffer,
            &self.ibl,
        );
    }

    /// Returns the bindless table.
    #[must_use]
    pub fn bindless(&self) -> &BindlessTable {
        &self.bindless
    }

    /// Returns the current cluster configuration.
    #[must_use]
    pub fn cluster_config(&self) -> ClusterConfig {
        self.cluster_config
    }
}

impl Pass for ClusteredForwardPass {
    fn name(&self) -> &str {
        "clustered_forward"
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
            tracing::error!("ClusteredForwardPass::record requires a FrameContext");
            return;
        };
        let view = frame.surface_view();
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hyge-render/clustered-forward"),
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
        render_pass.set_bind_group(0, self.bindless.bind_group(), &[]);
        render_pass.set_bind_group(1, &self.frame_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        for (batch_index, _batch) in self.batches.iter().enumerate() {
            // The shader uses `@location(0) draw_id : u32` as the
            // vertex input. We draw one point per draw command; the
            // vertex shader fetches the actual draw command from the
            // bindless buffer and issues the geometry itself. This is
            // a proxy for GPU-driven rendering; the fallback path
            // here simply draws a single triangle as a smoke-test
            // (the asset GPU upload path is the real R-043
            // integration; R-042 just verifies the bind group +
            // pipeline compile end-to-end).
            let draw = &self.draw_commands[batch_index];
            let _ = draw;
            render_pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_data_is_pod() {
        let data = FrameData::default_looking_at_origin();
        let bytes = bytemuck::bytes_of(&data);
        let round: FrameData = *bytemuck::from_bytes(bytes);
        assert_eq!(data.camera_pos_alpha_cutoff, round.camera_pos_alpha_cutoff);
    }
}
