# Changelog

All notable changes to Hyge are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [v0.1.0-m0] - 2026-06-29

### Added

- **Workspace foundation:** 14-crate workspace, root `Cargo.toml` with
  `resolver = "2"` and `[workspace.dependencies]` single-versioning policy.
- **Toolchain pin:** `rust-toolchain.toml` (stable 1.80), `rustfmt.toml`
  (imports_granularity = Crate, group_imports = StdExternalCrate),
  `clippy.toml` (msrv = 1.80, allows for the documented pedantic lints).
- **Governance:** 14 Architecture Decision Records in `docs/adr/`,
  ownership map in `docs/ownership.md`, roadmap reporting script
  (`tools/roadmap_report/`).
- **CI:** GitHub Actions matrix (`ci.yml` — Windows + Linux x {stable,
  beta}, headless-render job on Linux with Vulkan ICD, changelog job
  enforcing the breaking-change-without-ADR invariant), nightly job
  (`nightly.yml` — miri on `hyge-render` and `hyge-render-graph`, udeps,
  fuzz build), roadmap reporting workflow.
- **Core crates:**
    - `hyge-core`: math (Aabb, Frustum, Plane, Ray, glam re-exports),
      color (LinearRGB / Srgb with IEC 61966-2-1 transfer function), log
      (init_tracing with env filter), id (AssetId BLAKE3 newtype), result
      (HygeError, HygeResult), prelude.
    - `hyge-ecs`: bevy_ecs wrapper, `HygePlugin` trait, `AppBuilder`-friendly
      `AppHygeExt::add_hyge_plugin`, `Label` schedule enum (7 variants in
      execution order), 7 `SystemSet` enums (TransformSet, InputSet,
      AssetSet, PhysicsSet, AudioSet, ScriptSet, EditorSet), prelude.
    - `hyge-window`: `WindowConfig`, `Window` (wraps `Arc<winit::Window>`),
      `WindowPlugin` (registers events and resources), `WindowState`
      resource, event types (`WindowResized`, `WindowCloseRequested`,
      `WindowFocused`, `DeviceEvent` + `DeviceEventKind`), winit → Hyge
      event translation, Windows `RegisterRawInputDevices` raw input
      registration.
    - `hyge-app`: `App` struct (wraps bevy App + window), `AppBuilder`
      trait, `AppConfig` (with 9 subsystem fields: window, clear_color,
      renderer, assets, physics, audio, input, script, editor), `default_plugins()`
      returning `Vec<Box<dyn HygePlugin>>` with `WindowPlugin`,
      `ApplicationHandler` impl for the winit event loop integration.
- **M0 example:** `examples/hello_window.rs` — minimal App that opens a
  window with a configurable title and clear color.
- **M0 smoke test:** `crates/hyge-app/tests/m0_smoke.rs` — structural
  verification that the App boots, the 7 schedules are registered, the
  default plugins install without conflict, and the example file exists.

### Changed

- N/A (initial M0 release).

### Fixed

- N/A (initial M0 release).

## [Unreleased]

### Added

- **First-triangle render pass (R-024):** The runtime now
  renders a hardcoded red/green/blue triangle on top of the
  AppConfig's clear color each frame. The triangle is the
  M1 "first triangle" deliverable and replaces the M0
  clear-color smoke.
    - `crates/hyge-render/src/triangle.rs` —
      `TrianglePass` (a `hyge_render_graph::Pass` impl that
      clears the surface view and draws the three hardcoded
      vertices), `Vertex` (Pod/Zeroable, 2-D position + 3-D
      color), `VERTICES` (the three vertices, centered at the
      origin in clip space), `SHADER_SOURCE` (loaded via
      `include_str!` from `src/shader/triangle.wgsl`). The
      shader is minimal: `@vertex` writes a vec4 clip position
      and forwards the color, `@fragment` writes
      `vec4(color, 1.0)`. The pass owns a `wgpu::RenderPipeline`
      + `wgpu::Buffer` (cloned from the renderer's pre-built
      instances).
    - `crates/hyge-render/src/shader/triangle.wgsl` — the WGSL
      shader, compiled at pipeline construction time by
      `wgpu` / `naga`.
    - `crates/hyge-render-graph/src/frame.rs` — `FrameContext<'surface>`
      holding the current frame's `wgpu::TextureView` + format.
      Constructed by `Renderer::begin_frame`, consumed by
      `Renderer::end_frame`. Passes get it via
      `PassContext::frame()`.
    - `PassContext` (in `hyge-render-graph/src/pass.rs`) now
      holds an `Option<&mut FrameContext<'a>>`. The
      `Some(...)` arm is used by the windowed render path; the
      `None` arm is the headless test path (no surface).
    - `CompiledGraph::execute(encoder, frame)` (in
      `hyge-render-graph/src/compile.rs`) — the new signature
      threads the optional `FrameContext` through to each
      pass's `record` call.
    - `Renderer` (in `hyge-render/src/renderer.rs`) — pre-builds
      the triangle pipeline + vertex buffer at construction
      time (one with the surface format on the windowed path,
      one with `Rgba8UnormSrgb` on the headless path).
      New methods: `begin_frame`, `end_frame`,
      `current_frame_view`, `surface_format`,
      `build_triangle_graph(clear_color)`,
      `render_triangle(clear_color)` (the R-024 frame path:
      begin → build graph → compile → execute → submit →
      present) and `render_triangle_to_texture(target,
      clear_color)` (the off-screen test path that polls
      `Maintain::Wait` before returning so callers can map the
      texture immediately).
    - `crates/hyge-runtime-test` — implemented the test
      harness: `TestRenderer` (headless wgpu, software adapter
      preferred), `capture_frame` (copies a `wgpu::Texture`
      to a row-major un-padded RGBA8 `Vec<u8>`), `assert_image_matches`
      (per-pixel diff with a tolerance, returns `bool`-ish
      assertion), `hash_image` (BLAKE3 hash of a byte buffer
      as a `blake3:<hex>` string).
    - `crates/hyge-runtime-test/tests/first_triangle.rs` — the
      R-024 acceptance snapshot test:
      `first_triangle_captures_reference_frame_within_tolerance`
      (renders, captures, asserts the center pixel is
      non-clear, the four corners are clear, and the full
      frame matches an all-clear reference within 30%
      tolerance — the triangle covers ~25% of the frame),
      `first_triangle_is_deterministic_across_renders` (two
      consecutive renders produce identical hashes + bytes),
      and `test_renderer_new_handles_no_adapter_gracefully`
      (smoke test for the constructor).
    - `hyge-app` — `App` now owns `Option<Renderer>`. `App::resumed`
      creates the renderer after the Window is created
      (using `RendererConfig::default()`). `App::about_to_wait`
      calls `renderer.render_triangle(clear_color)` where
      `clear_color` is `AppConfig::clear_color` cast to
      `wgpu::Color`. `App::window_event` calls
      `renderer.resize(w, h)` on `WindowEvent::Resized`.
      `examples/hello_window.rs` updated to demonstrate the
      triangle (title `Hyge Hello Window — first triangle`,
      clear color dark blue).

### Changed

- The R-020 `TransientAllocator::mark_live` helper was removed in
  favor of the new `allocate` (first-touch) / `deallocate`
  (in-frame release) / `next_frame` (end-of-frame) trio. The
  `register` API is unchanged; existing callers
  (`CompiledGraph::from_parts`) continue to work — only the
  semantics shifted from "register creates a slot" to "register
  stores the descriptor; slot materializes on first `allocate`",
  which is the correct behavior for first-touch arenas.

- `CompiledGraph::execute` signature changed from
  `execute(&mut self, encoder: &mut wgpu::CommandEncoder)` to
  `execute(&mut self, encoder: &mut wgpu::CommandEncoder, frame:
  Option<&mut FrameContext<'_>>)`. R-020 tests don't call
  `execute`, so no regressions.

### Changed

- `RenderGraph::compile` no longer uses a `HashMap<ResourceHandle,
  Access>` for the per-resource state during barrier inference; it
  uses the new `BarrierStateTable` (same algorithmic behavior, but
  raw-vec layout for the hot path).
- `BarrierStateTable::set` overwrites an existing slot without
  dropping the previous value (sound because `AccessState: Copy`).
- The `tests` directory layout was changed: the new
  `tests/barrier_inference.rs` integration test lives at
  `crates/hyge-render-graph/tests/barrier_inference.rs` (not
  `crates/hyge-render-graph/tests/tests/...`) so cargo picks it up
  as an integration test.

### Notes

The R-020 deliverable (`hyge-render-graph` crate skeleton — DAG,
`Pass` trait, `ResourceHandle`, `CompiledGraph`, transient allocator
skeleton) was rolled into the same `[Unreleased]` section above
in the previous batch of commits; see the R-020 entry in
`docs/roadmap.toml` for the full surface. R-021 builds on R-020
by fleshing out the barrier inference algorithm.
