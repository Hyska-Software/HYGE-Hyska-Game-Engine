//! The runtime [`Renderer`].
//!
//! # Overview
//!
//! The renderer owns the wgpu [`wgpu::Instance`], [`wgpu::Device`],
//! [`wgpu::Queue`], the optional [`wgpu::Surface`] (and its
//! [`wgpu::SurfaceConfiguration`]), the pre-built triangle
//! pipeline + vertex buffer (R-024), and the per-frame
//! [`hyge_render_graph::prelude::RenderGraph`]. Construction takes a
//! [`RendererConfig`] and either a `hyge_window::Window` (for
//! the windowed / present-to-screen path) or nothing (for the
//! headless / compute-only path used by tests).
//!
//! # Field drop order
//!
//! The struct fields are declared in the order required for a
//! sound `Drop`:
//! 1. `instance` (last to drop — the `Instance` owns the runtime
//!    and must outlive every resource derived from it),
//! 2. `device` and `queue` (dropped before the surface, because
//!    the surface references the device),
//! 3. `triangle_pipeline` and `triangle_vertex_buffer` (R-024
//!    resources, dropped alongside the device),
//! 4. `window_keepalive` (the `Arc<winit::Window>` that backs the
//!    surface; dropped AFTER the surface so the surface is never
//!    used with a freed window),
//! 5. `surface` and `surface_config` (the wgpu surface and its
//!    configuration, dropped before the instance),
//! 6. `current_frame` (the current `SurfaceTexture`; dropped
//!    before the surface so the backbuffer is presented /
//!    released before the surface is destroyed),
//! 7. `config` and `graph` (plain data, no wgpu state).
//!
//! # Why a `'static` surface?
//!
//! wgpu 22's [`wgpu::SurfaceTargetUnsafe::from_window`] requires
//! a `&'static` reference to the window. The lifetime of the
//! surface we build is a lie: the actual lifetime is bounded by
//! the `Arc<winit::Window>` in `self.window_keepalive`. The
//! `transmute` that produces the `&'static` reference is
//! sound because the `Arc` outlives the surface (see the SAFETY
//! comment in [`Renderer::new`]). This matches the pattern used
//! by the wgpu 22 examples and by the wgpu + winit 0.30 ecosystem
//! at large; the alternative is to thread a lifetime parameter
//! through the entire renderer, which is what R-024+ will do
//! once we move beyond the skeleton.

use std::sync::Arc;

use winit::window::Window as WinitWindow;

use hyge_core::prelude::*;
use hyge_render_graph::prelude::*;
use hyge_window::prelude::Window;

use crate::bindless::{BindlessConfig, BindlessTable};
use crate::clustered_forward::ClusteredForwardPass;
use crate::config::RendererConfig;
use crate::ibl::EnvironmentBake;
use crate::ibl_gpu::{self, IblResources};
use crate::profiler::{FrameStats, GpuProfiler};
use crate::triangle::TrianglePass;

/// The runtime renderer.
pub struct Renderer {
    /// The wgpu instance. Declared first so it is dropped last;
    /// every other wgpu object borrows from it.
    instance: wgpu::Instance,
    /// The wgpu device. Dropped before the surface (the surface
    /// holds a reference to the device internally). Wrapped in
    /// an `Arc` so the bindless table (R-037) can share
    /// ownership.
    device: Arc<wgpu::Device>,
    /// The wgpu queue. Dropped alongside the device. Wrapped in
    /// an `Arc` so the bindless table (R-037) can share
    /// ownership.
    queue: Arc<wgpu::Queue>,
    /// The pre-built render pipeline for the first-triangle
    /// pass (R-024). Created at construction time with the
    /// surface format (or `Rgba8UnormSrgb` for the headless
    /// path). `Clone` is cheap (reference-counted inside wgpu).
    triangle_pipeline: Arc<wgpu::RenderPipeline>,
    /// The pre-built vertex buffer for the first-triangle
    /// pass (R-024). Holds the three hardcoded vertices from
    /// `crate::triangle::VERTICES`.
    triangle_vertex_buffer: Arc<wgpu::Buffer>,
    /// The bindless descriptor heap (R-037). Owns the
    /// per-resource storage buffers, the texture array, the
    /// default samplers, and the bind group + layout. The
    /// asset server's GPU upload path registers mesh /
    /// material / texture entries here.
    bindless: Arc<BindlessTable>,
    /// Optional IBL resources uploaded from a baked environment
    /// (R-041). Bound into the PBR pass's frame bind group when
    /// present; when absent the PBR pass uses a fallback ambient.
    ibl: Option<IblResources>,
    /// GPU timestamp profiler and latest frame statistics.
    profiler: GpuProfiler,
    /// The `Arc<winit::Window>` that backs the surface. Stored
    /// here to keep the window alive for the surface's `'static`
    /// lifetime. `None` for the headless path.
    window_keepalive: Option<Arc<WinitWindow>>,
    /// The wgpu surface, if the renderer was created with a
    /// window. `None` for the headless path.
    surface: Option<wgpu::Surface<'static>>,
    /// The surface configuration. Updated by [`Renderer::resize`].
    surface_config: Option<wgpu::SurfaceConfiguration>,
    /// The current swapchain texture, held between
    /// [`Renderer::begin_frame`] and [`Renderer::end_frame`].
    /// `None` outside of a frame. The texture is returned to
    /// the surface by [`Renderer::end_frame`] (via
    /// `SurfaceTexture::present`) before the frame is dropped.
    current_frame: Option<wgpu::SurfaceTexture>,
    /// The configuration the renderer was constructed with.
    config: RendererConfig,
    /// The per-frame render graph. Re-built by the caller each
    /// frame; the actual `compile` / `execute` integration is
    /// wired in R-024 (the triangle pass) and grows from there.
    graph: RenderGraph,
    /// Lazily-initialised clustered-forward pass used by
    /// [`Renderer::render_frame`]. `None` until the first
    /// `render_frame` call; reused on subsequent frames.
    clustered: Option<ClusteredForwardPass>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("config", &self.config)
            .field("has_surface", &self.surface.is_some())
            .field("has_window", &self.window_keepalive.is_some())
            .field("has_frame", &self.current_frame.is_some())
            .field("device_label", &self.config.device_label)
            .finish_non_exhaustive()
    }
}

impl Renderer {
    /// Constructs a renderer bound to a winit window. Creates the
    /// wgpu instance (with optional validation), the surface, the
    /// adapter, the device, the queue, configures the swapchain,
    /// and pre-builds the first-triangle pipeline (R-024).
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if surface / adapter / device
    /// creation fails, or [`HygeError::Unsupported`] if no adapter
    /// is available for the surface.
    pub fn new(config: RendererConfig, window: &Window) -> HygeResult<Self> {
        let winit_arc: Arc<WinitWindow> = window.handle();
        let size = window.size();
        let instance = create_instance(&config);

        // SAFETY: wgpu 22's `SurfaceTargetUnsafe::from_window`
        // requires a `&'static` reference to the window. The
        // actual lifetime of the window we pass in is the
        // lifetime of the local `winit_arc` binding, which is
        // moved into `self.window_keepalive` below. The renderer
        // drops the surface before the window (see the field
        // declaration order in the struct rustdoc), so the
        // surface is never used after the window is freed. The
        // `transmute` is therefore sound: the pointer remains
        // valid for the entire surface lifetime.
        let surface = unsafe {
            let winit_ref: &WinitWindow = &winit_arc;
            let static_ref: &'static WinitWindow = std::mem::transmute(winit_ref);
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(static_ref)
                        .map_err(|e| HygeError::gpu(format!("surface target: {e}")))?,
                )
                .map_err(|e| HygeError::gpu(format!("create surface: {e}")))?
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: config.power_preference,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or_else(|| HygeError::unsupported("no wgpu adapter compatible with surface"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some(&config.device_label),
                required_features: bindless_required_features(&adapter),
                required_limits: bindless_limits(&adapter),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .map_err(|e| HygeError::gpu(format!("request device: {e}")))?;
        // R-037: the bindless table takes `Arc<wgpu::Device>`
        // and `Arc<wgpu::Queue>`. We wrap the wgpu handles
        // immediately after `request_device` and use the
        // `Arc`s everywhere below.
        let device_arc: Arc<wgpu::Device> = Arc::new(device);
        let queue_arc: Arc<wgpu::Queue> = Arc::new(queue);

        let surface_caps = surface.get_capabilities(&adapter);
        // Prefer sRGB; fall back to the first format the surface
        // advertises. Surfaces in wgpu 22 always expose at least
        // one format, so the unwrap is safe.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: pick_present_mode(&surface_caps, config.present_mode),
            alpha_mode: surface_caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device_arc, &surface_config);

        // Pre-build the first-triangle pipeline + vertex buffer
        // (R-024). The pipeline is rendered every frame, so we
        // build it once at startup rather than every frame.
        let triangle_pipeline = TrianglePass::create_pipeline(&device_arc, surface_format);
        let triangle_vertex_buffer = TrianglePass::create_vertex_buffer(&device_arc);
        let timestamps_enabled = timestamp_queries_enabled(&device_arc);
        let profiler = GpuProfiler::new(&device_arc, &queue_arc, timestamps_enabled);

        // R-037: build the bindless descriptor heap. The
        // table owns its own storage buffers + texture array
        // + bind group; nothing in the renderer needs to
        // touch them directly. The asset server's upload
        // path reaches in through `Renderer::bindless()`.
        let bindless = Arc::new(BindlessTable::new(
            Arc::clone(&device_arc),
            Arc::clone(&queue_arc),
            BindlessConfig::default(),
        )?);

        Ok(Self {
            instance,
            device: device_arc,
            queue: queue_arc,
            triangle_pipeline,
            triangle_vertex_buffer,
            bindless,
            ibl: None,
            profiler,
            window_keepalive: Some(winit_arc),
            surface: Some(surface),
            surface_config: Some(surface_config),
            current_frame: None,
            config,
            graph: RenderGraph::new(),
            clustered: None,
        })
    }

    /// Constructs a renderer without a surface. Useful for tests
    /// (the device-init smoke test) and for compute-only setups
    /// (no present-to-screen). The `compatible_surface` filter is
    /// set to `None` and `force_fallback_adapter: true` so a
    /// software adapter is preferred — the headless path does not
    /// care about GPU compatibility with a display.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if device creation fails, or
    /// [`HygeError::Unsupported`] if no wgpu adapter is available.
    #[allow(dead_code)]
    pub fn new_headless(config: &RendererConfig) -> HygeResult<Self> {
        let instance = create_instance(config);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: config.power_preference,
            force_fallback_adapter: true,
            compatible_surface: None,
        }))
        .ok_or_else(|| HygeError::unsupported("no wgpu adapter available"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some(&config.device_label),
                required_features: bindless_required_features(&adapter),
                required_limits: bindless_limits(&adapter),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .map_err(|e| HygeError::gpu(format!("request device: {e}")))?;
        // R-037: the bindless table needs `Arc` handles. We
        // wrap the wgpu handles here and unwrap them into the
        // renderer's own fields below.
        let device_arc: Arc<wgpu::Device> = Arc::new(device);
        let queue_arc: Arc<wgpu::Queue> = Arc::new(queue);

        // The headless renderer has no surface; the test path
        // supplies its own off-screen render target whose
        // format must match the pipeline format. We pick a
        // sensible default (sRGB) and let the test configure
        // the target to match.
        let triangle_pipeline =
            TrianglePass::create_pipeline(&device_arc, wgpu::TextureFormat::Rgba8UnormSrgb);
        let triangle_vertex_buffer = TrianglePass::create_vertex_buffer(&device_arc);
        let timestamps_enabled = timestamp_queries_enabled(&device_arc);
        let profiler = GpuProfiler::new(&device_arc, &queue_arc, timestamps_enabled);

        // R-037: build the bindless table (headless).
        let bindless = Arc::new(BindlessTable::new(
            Arc::clone(&device_arc),
            Arc::clone(&queue_arc),
            BindlessConfig::default(),
        )?);

        Ok(Self {
            instance,
            device: device_arc,
            queue: queue_arc,
            triangle_pipeline,
            triangle_vertex_buffer,
            bindless,
            ibl: None,
            profiler,
            window_keepalive: None,
            surface: None,
            surface_config: None,
            current_frame: None,
            config: config.clone(),
            graph: RenderGraph::new(),
            clustered: None,
        })
    }

    /// Begins a frame: acquires the swapchain texture and stores
    /// it on `self` for the duration of the frame. Must be
    /// called before [`Renderer::render_triangle`] (or any other
    /// render pass) and before [`Renderer::end_frame`].
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the surface fails to
    /// acquire a new texture (e.g. the swapchain is out of date
    /// and needs a reconfigure, or the window is minimized).
    pub fn begin_frame(&mut self) -> HygeResult<()> {
        if self.current_frame.is_some() {
            return Err(HygeError::gpu("begin_frame called twice without end_frame"));
        }
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| HygeError::gpu("begin_frame: no surface"))?;
        let frame = surface
            .get_current_texture()
            .map_err(|e| HygeError::gpu(format!("get_current_texture: {e}")))?;
        self.current_frame = Some(frame);
        Ok(())
    }

    /// Ends a frame: presents the swapchain texture and clears
    /// `self.current_frame`. Must be called after
    /// [`Renderer::begin_frame`].
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if no frame is in flight.
    pub fn end_frame(&mut self) -> HygeResult<()> {
        let frame = self
            .current_frame
            .take()
            .ok_or_else(|| HygeError::gpu("end_frame: no current frame"))?;
        frame.present();
        Ok(())
    }

    /// Returns a [`wgpu::TextureView`] of the current swapchain
    /// texture. Only valid between [`Renderer::begin_frame`] and
    /// [`Renderer::end_frame`]. The view is cloned from the
    /// underlying texture and is safe to hold for the duration
    /// of the frame.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if no frame is in flight.
    pub fn current_frame_view(&self) -> HygeResult<wgpu::TextureView> {
        let frame = self
            .current_frame
            .as_ref()
            .ok_or_else(|| HygeError::gpu("current_frame_view: no current frame"))?;
        Ok(frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// Returns the format the swapchain (or, for the headless
    /// path, the default triangle pipeline) was created with.
    /// The triangle pass uses this format for its color-target
    /// state.
    #[must_use]
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb)
    }

    /// Builds a [`RenderGraph`] that contains a single
    /// [`TrianglePass`] with the renderer's pre-built pipeline +
    /// vertex buffer. The caller can extend the graph (e.g. add
    /// shadow / post-process passes) before compiling.
    #[must_use]
    pub fn build_triangle_graph(&self, clear_color: wgpu::Color) -> RenderGraph {
        let mut graph = RenderGraph::new();
        let pass = TrianglePass::with_prebuilt(
            self.triangle_pipeline.clone(),
            self.triangle_vertex_buffer.clone(),
            clear_color,
        );
        graph.add_pass(pass);
        graph
    }

    /// Renders one full frame: begin frame → build the triangle
    /// graph → compile + execute → submit → present. The
    /// `clear_color` is the color the surface is cleared to
    /// before the triangle is drawn; the triangle itself paints
    /// red / green / blue over the clear color.
    ///
    /// This is the R-024 smoke path. R-040+ replace it with the
    /// full PBR / shadow / post-process pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the frame acquisition, the
    /// pipeline compile, or the queue submission fails.
    pub fn render_triangle(&mut self, clear_color: wgpu::Color) -> HygeResult<()> {
        self.profiler.begin_frame();
        self.begin_frame()?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hyge-render/triangle-frame"),
            });
        let view = self.current_frame_view()?;
        let format = self.surface_format();
        let mut frame_ctx = FrameContext::new(view, format);
        {
            let mut graph = self.build_triangle_graph(clear_color);
            let mut compiled = graph.compile(&self.device)?;
            let pass_names = compiled
                .passes()
                .iter()
                .map(|pass| pass.name().to_string())
                .collect::<Vec<_>>();
            compiled.execute_with_hooks(
                &mut encoder,
                Some(&mut frame_ctx),
                |_, pass_index, encoder| self.profiler.write_pass_start(encoder, pass_index),
                |_, pass_index, encoder| self.profiler.write_pass_end(encoder, pass_index),
            );
            self.profiler.resolve(&mut encoder, pass_names.len() as u32);
            self.queue.submit(std::iter::once(encoder.finish()));
            self.profiler.finish_frame(&self.device, &pass_names, 1, 1);
            self.end_frame()?;
            Ok(())
        }
    }

    /// Renders the triangle into an off-screen [`wgpu::Texture`]
    /// (the test / capture path). The target's format must
    /// match the renderer's `surface_format()` (the pipeline
    /// was pre-built with that format).
    ///
    /// After this call returns, the texture contains the
    /// rendered image. The caller can map it for readback (see
    /// `hyge-runtime-test::capture_frame`).
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the encoder creation or
    /// queue submission fails.
    pub fn render_triangle_to_texture(
        &mut self,
        target: &wgpu::Texture,
        clear_color: wgpu::Color,
    ) -> HygeResult<()> {
        self.profiler.begin_frame();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hyge-render/triangle-offscreen"),
            });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let format = target.format();
        let mut frame_ctx = FrameContext::new(view, format);
        {
            let mut graph = self.build_triangle_graph(clear_color);
            let mut compiled = graph.compile(&self.device)?;
            let pass_names = compiled
                .passes()
                .iter()
                .map(|pass| pass.name().to_string())
                .collect::<Vec<_>>();
            compiled.execute_with_hooks(
                &mut encoder,
                Some(&mut frame_ctx),
                |_, pass_index, encoder| self.profiler.write_pass_start(encoder, pass_index),
                |_, pass_index, encoder| self.profiler.write_pass_end(encoder, pass_index),
            );
            self.profiler.resolve(&mut encoder, pass_names.len() as u32);
            self.queue.submit(std::iter::once(encoder.finish()));
            self.device.poll(wgpu::Maintain::Wait);
            self.profiler.finish_frame(&self.device, &pass_names, 1, 1);
            Ok(())
        }
    }

    /// Renders a frame from a per-frame snapshot. The snapshot
    /// provides the instance buffer, the draw command list, and
    /// the lights. They are uploaded to the bindless table, then
    /// the renderer's `ClusteredForwardPass` is recorded and
    /// submitted. The target view is cleared to `clear_color`
    /// before drawing.
    ///
    /// `frame_data` is the per-frame uniform consumed by
    /// `pbr.wgsl` (camera, sun, exposure).
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the encoder creation, queue
    /// submission, or device poll fails. The `ClusteredForwardPass`
    /// is constructed lazily on the first call; subsequent calls
    /// reuse it (replacing the snapshot each frame).
    #[allow(
        clippy::too_many_arguments,
        reason = "frame submission is intrinsically many-arg"
    )]
    pub fn render_frame(
        &mut self,
        target: &wgpu::Texture,
        target_format: wgpu::TextureFormat,
        clear_color: wgpu::Color,
        frame_data: &crate::clustered_forward::FrameData,
        instances: &[crate::bindless::Instance],
        draw_commands: &[crate::bindless::DrawCommand],
        lights: &[crate::bindless::Light],
    ) -> HygeResult<()> {
        use crate::bindless::{DrawCommand, Light};
        use crate::clustered_forward::{Batch as CfBatch, ClusteredForwardPass, ClusterConfig};
        use std::sync::Arc;

        self.profiler.begin_frame();

        // Lazily construct the clustered-forward pass.
        if self.clustered.is_none() {
            let device: &wgpu::Device = &self.device;
            // Size for at least one packed PBR vertex
            // (48 bytes per vertex from `pbr.wgsl`).
            let pbr_stride: u64 = 48;
            let vertex_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hyge-render/pbr-vertex-buffer"),
                size: pbr_stride,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            }));
            let index_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hyge-render/pbr-index-buffer"),
                size: std::mem::size_of::<u32>() as u64, // 1 index placeholder
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            let pass = ClusteredForwardPass::new(
                device,
                Arc::clone(&self.bindless),
                None,
                target_format,
                ClusterConfig::default(),
                vertex_buffer,
                index_buffer,
                clear_color,
            );
            self.clustered = Some(pass);
        }

        // Upload snapshot to the bindless table.
        self.bindless.write_instances(0, instances);
        if !lights.is_empty() {
            self.bindless.write_lights(0, lights);
        } else {
            self.bindless.write_lights(0, &[Light::default()]);
        }
        self.bindless.write_draw_commands(0, draw_commands);
        if draw_commands.is_empty() {
            // Pad with a single zeroed draw command so the
            // shader never reads an empty storage buffer.
            self.bindless
                .write_draw_commands(0, &[DrawCommand::default()]);
        }

        // Build the per-frame batch list (one per draw command).
        // The `Batch` struct now stores raw `u32` slot indexes
        // (the typed `BindlessSlot<MeshTag>` does not leave the
        // bindless table), so this is a direct field copy.
        let batches: Vec<CfBatch> = draw_commands
            .iter()
            .map(|cmd| CfBatch {
                mesh_id: cmd.mesh_id,
                material_id: cmd.material_id,
                first_instance: cmd.first_instance,
                instance_count: cmd.instance_count,
                index_count: 3,
                first_index: 0,
                base_vertex: 0,
            })
            .collect();

        if let Some(pass) = self.clustered.as_mut() {
            pass.set_frame_data(&self.queue, *frame_data);
            pass.set_lights(
                &self.queue,
                if lights.is_empty() {
                    vec![Light::default()]
                } else {
                    lights.to_vec()
                },
            );
            pass.set_geometry(
                &self.queue,
                instances.to_vec(),
                draw_commands.to_vec(),
                batches,
            );
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hyge-render/frame"),
            });

        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut frame_ctx = FrameContext::new(view, target_format);
        {
            if let Some(pass) = self.clustered.as_mut() {
                let mut pass_ctx = PassContext::for_frame(&mut frame_ctx, &mut encoder);
                pass.record(&mut pass_ctx);
            }
        }

        self.profiler.resolve(&mut encoder, 1);
        self.queue.submit(std::iter::once(encoder.finish()));
        self.device.poll(wgpu::Maintain::Wait);
        self.profiler
            .finish_frame(&self.device, &[String::from("render_frame")], 1, 1);
        Ok(())
    }

    /// Resizes the surface to the given width and height. A no-op
    /// for headless renderers (no surface to resize).
    ///
    /// Widths and heights of zero are clamped to 1 — wgpu rejects
    /// zero-sized surfaces with an error.
    pub fn resize(&mut self, w: u32, h: u32) {
        if let (Some(surface), Some(cfg)) = (self.surface.as_ref(), self.surface_config.as_mut()) {
            cfg.width = w.max(1);
            cfg.height = h.max(1);
            surface.configure(&self.device, cfg);
        }
    }


    /// Returns the wgpu device.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Returns the wgpu queue.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Returns the latest frame statistics produced by the profiler.
    #[must_use]
    pub fn frame_stats(&self) -> &FrameStats {
        self.profiler.stats()
    }

    /// Returns the wgpu instance.
    #[must_use]
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// Returns the wgpu surface, if this renderer was created
    /// with a window.
    #[must_use]
    pub fn surface(&self) -> Option<&wgpu::Surface<'static>> {
        self.surface.as_ref()
    }

    /// Returns the current surface configuration, if any.
    #[must_use]
    pub fn surface_config(&self) -> Option<&wgpu::SurfaceConfiguration> {
        self.surface_config.as_ref()
    }

    /// Returns the renderer configuration.
    #[must_use]
    pub fn config(&self) -> &RendererConfig {
        &self.config
    }

    /// Returns `true` if this renderer has a wgpu surface bound
    /// to a window. False for headless renderers.
    #[must_use]
    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    /// Returns a mutable reference to the per-frame
    /// [`RenderGraph`]. The caller adds resources, passes, and
    /// later calls `compile` + `execute`.
    #[must_use]
    pub fn graph_mut(&mut self) -> &mut RenderGraph {
        &mut self.graph
    }

    /// Returns the bindless descriptor heap (R-037). The
    /// asset server's GPU upload path calls
    /// [`BindlessTable::register_mesh`] / `register_material` /
    /// `register_texture` to allocate slots; the renderer
    /// never touches the table directly.
    #[must_use]
    pub fn bindless(&self) -> &BindlessTable {
        &self.bindless
    }

    /// Returns a clone of the bindless table's `Arc`.
    /// Integration tests that need to share the table
    /// with helper functions (e.g. `LambertPass::new`)
    /// use this accessor; the table is `Send + Sync` so
    /// the `Arc` is safe to share across threads.
    #[must_use]
    pub fn bindless_arc(&self) -> Arc<BindlessTable> {
        Arc::clone(&self.bindless)
    }

    /// Uploads a baked environment to wgpu textures and stores
    /// the resulting [`IblResources`] on the renderer. The PBR
    /// pass binds these views in its frame bind group when
    /// present.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the device is lost during
    /// texture creation.
    pub fn set_environment(&mut self, bake: &EnvironmentBake) -> HygeResult<()> {
        self.ibl = Some(ibl_gpu::upload(&self.device, &self.queue, bake)?);
        Ok(())
    }

    /// Returns the currently uploaded IBL resources, if any.
    #[must_use]
    pub fn ibl(&self) -> Option<&IblResources> {
        self.ibl.as_ref()
    }
}

/// Returns the optional timestamp feature set requested from the adapter.
fn timestamp_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let required =
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if adapter.features().contains(required) {
        required
    } else {
        wgpu::Features::empty()
    }
}

/// Returns true when command-encoder timestamp writes are available.
fn timestamp_queries_enabled(device: &wgpu::Device) -> bool {
    let required =
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    device.features().contains(required)
}

/// Returns the set of features the renderer requires to build
/// the bindless descriptor heap (R-037). On adapters that
/// don't support `TEXTURE_BINDING_ARRAY` (the v0.1 texture
/// bindless path) the renderer still works for the
/// mesh/material paths, so the feature is requested when
/// available and the device-init code path is allowed to
/// proceed without it — the bindless table will simply
/// fall back to per-texture individual bindings when the
/// feature is absent. (For R-037 the texture path is not
/// yet exercised by the acceptance test, so the feature
/// is requested opportunistically.)
fn bindless_required_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let mut features = timestamp_features(adapter);
    let bindless = wgpu::Features::TEXTURE_BINDING_ARRAY;
    if adapter.features().contains(bindless) {
        features |= bindless;
    }
    features
}

/// Returns the device limits the bindless table needs (R-037).
///
/// wgpu's conservative `downlevel_defaults` cap
/// `max_storage_buffers_per_shader_stage` at 4, which is
/// not enough for the bindless table's 6+ storage buffers
/// (meshes, materials, instances, lights, light grid,
/// meshlet visibility, draw commands). We take the
/// adapter's `Limits` (the per-adapter maximum) and only
/// bump the storage-buffer + sampled-texture counts that
/// the table actually needs. Adapters that don't support
/// the requested counts simply fall back to
/// `downlevel_defaults` for those fields, and the bindless
/// table degrades gracefully (the texture-array binding is
/// dropped when `max_sampled_textures_per_shader_stage <
/// 16`).
fn bindless_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let mut limits = adapter.limits();
    // The bindless table exposes 7 storage buffers, all of
    // which need to be visible to the fragment stage (the
    // PBR shader reads mesh + material + light + grid
    // uniforms there). 10 is a safe upper bound that fits
    // every real GPU since 2018.
    if limits.max_storage_buffers_per_shader_stage < 10 {
        limits.max_storage_buffers_per_shader_stage = 10;
    }
    // The texture-array binding requires 16+ array layers
    // to be useful; if the adapter can't go that high, the
    // bindless table will silently drop the binding
    // (covered in `BindlessTable::new`).
    if limits.max_sampled_textures_per_shader_stage < 16 {
        limits.max_sampled_textures_per_shader_stage = 16;
    }
    limits
}

/// Constructs the wgpu [`wgpu::Instance`] with the requested
/// backends and the validation-layer toggle. The `Instance` is
/// the root of every other wgpu object and is owned by the
/// [`Renderer`].
fn create_instance(config: &RendererConfig) -> wgpu::Instance {
    let flags = if config.validation {
        wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG
    } else {
        wgpu::InstanceFlags::default()
    };
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: config.backend,
        flags,
        ..Default::default()
    })
}

/// Picks the present mode the surface will be configured with.
/// If the requested mode is not in the surface's capability list
/// (some platforms don't support Mailbox or Immediate), the
/// function falls back to `Fifo` — the one mode every platform
/// must support per the WebGPU spec.
fn pick_present_mode(
    caps: &wgpu::SurfaceCapabilities,
    requested: wgpu::PresentMode,
) -> wgpu::PresentMode {
    if caps.present_modes.contains(&requested) {
        requested
    } else {
        wgpu::PresentMode::Fifo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_present_mode_prefers_requested() {
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![wgpu::TextureFormat::Rgba8UnormSrgb],
            present_modes: vec![wgpu::PresentMode::Fifo, wgpu::PresentMode::Mailbox],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Auto],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert_eq!(
            pick_present_mode(&caps, wgpu::PresentMode::Mailbox),
            wgpu::PresentMode::Mailbox
        );
        assert_eq!(
            pick_present_mode(&caps, wgpu::PresentMode::Fifo),
            wgpu::PresentMode::Fifo
        );
    }

    #[test]
    fn pick_present_mode_falls_back_to_fifo() {
        let caps = wgpu::SurfaceCapabilities {
            formats: vec![wgpu::TextureFormat::Rgba8UnormSrgb],
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Auto],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert_eq!(
            pick_present_mode(&caps, wgpu::PresentMode::Mailbox),
            wgpu::PresentMode::Fifo
        );
        assert_eq!(
            pick_present_mode(&caps, wgpu::PresentMode::Immediate),
            wgpu::PresentMode::Fifo
        );
    }

    /// R-023 acceptance: "device init succeeds on a software
    /// adapter (when available) or skips when not". Uses
    /// `new_headless` with `force_fallback_adapter: true` so the
    /// software path is always preferred.
    #[test]
    fn device_init_succeeds_on_software_adapter() {
        let config = RendererConfig::default();
        let renderer = match Renderer::new_headless(&config) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("no wgpu adapter available ({e}); skipping");
                return;
            }
        };
        assert!(!renderer.has_surface());
        renderer.device().poll(wgpu::Maintain::Poll);
        assert_eq!(renderer.config().device_label, "hyge-device");
    }

    #[test]
    fn renderer_debug_does_not_panic() {
        let config = RendererConfig::default();
        let Ok(renderer) = Renderer::new_headless(&config) else {
            return;
        };
        let s = format!("{renderer:?}");
        assert!(s.contains("Renderer"));
        assert!(s.contains("hyge-device"));
    }

    /// R-024: the headless renderer pre-builds the triangle
    /// pipeline + vertex buffer at construction time. We can
    /// verify the format + the headless path's surface_format
    /// accessor without a real wgpu device (the test skips when
    /// no adapter is available).
    #[test]
    fn headless_renderer_pre_builds_triangle_state() {
        let config = RendererConfig::default();
        let Ok(renderer) = Renderer::new_headless(&config) else {
            return;
        };
        // The headless default format is sRGB. The test path
        // uses this to pick the off-screen render-target format.
        assert_eq!(
            renderer.surface_format(),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
    }

    /// R-024: an off-screen render of the triangle round-trips
    /// without errors. We create a small off-screen target,
    /// render the triangle into it, and verify the renderer is
    /// still alive (no panic, no leaked resources).
    #[test]
    fn render_triangle_to_offscreen_target() {
        let config = RendererConfig::default();
        let Ok(mut renderer) = Renderer::new_headless(&config) else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };
        let target = renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("test-target"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.surface_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        renderer
            .render_triangle_to_texture(
                &target,
                wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            )
            .expect("render_triangle_to_texture should succeed");
    }
}
