//! Hyge render: the runtime renderer.
//!
//! Owns the wgpu device / queue (typically on a dedicated render
//! thread), the wgpu surface and swapchain configuration bound to
//! the application window, the per-frame render graph, the
//! pre-built first-triangle pipeline (R-024), and the bindless
//! descriptor heap (R-037). R-040+ add the clustered forward
//! pipeline, cascaded shadows, meshlet culling, post-process,
//! and IBL.
//!
//! # R-023 (skeleton), R-024 (first triangle), R-025 (profiler),
//!   R-037 (bindless table)
//!
//! The public surface:
//!
//! - [`RendererConfig`](config::RendererConfig) — the
//!   user-facing configuration (backends, vsync, present_mode,
//!   power preference, validation).
//! - [`Renderer`](renderer::Renderer) — the runtime type, with
//!   `new(config, window)` (windowed), `new_headless(config)`
//!   (compute / test), `begin_frame` / `end_frame` for surface
//!   frame control, `render_triangle(clear_color)` for the
//!   R-024 first-triangle pass, `render_triangle_to_texture`
//!   for the off-screen test path, `resize(w, h)`, and the usual
//!   accessors for `device`, `queue`, `instance`, `surface`,
//!   `surface_config`, `config`, `surface_format`, `has_surface`,
//!   `bindless`, and `graph_mut`.
//! - [`BindlessTable`](bindless::BindlessTable) — the
//!   bindless descriptor heap with the slot layout from
//!   `docs/architecture.md` §8.1. Allocates mesh, material,
//!   texture, instance, light, light-grid, meshlet-visibility,
//!   and draw-command slots. Returns typed ids
//!   ([`MeshId`](bindless::MeshId),
//!   [`MaterialId`](bindless::MaterialId),
//!   [`TextureId`](bindless::TextureId)) that the asset
//!   server feeds to the GPU upload path.
//! - [`TrianglePass`](triangle::TrianglePass) — the first
//!   render-graph pass. The WGSL shader lives at
//!   `src/shader/triangle.wgsl`.
//! - [`FrameStats`](profiler::FrameStats) — the per-frame profiling
//!   resource populated by timestamp queries and draw counters.
//!
//! See `docs/architecture.md` §6.4 for the full planned surface,
//! and `docs/roadmap.toml` for the R-024+ roadmap (bindless,
//! clustered forward, post-process).

#![warn(missing_docs)]

pub mod bindless;
pub mod config;
pub mod profiler;
pub mod renderer;
pub mod triangle;

/// The renderer prelude.
pub mod prelude;
