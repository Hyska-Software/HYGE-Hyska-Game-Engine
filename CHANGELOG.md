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

- **Render graph crate (`hyge-render-graph`):** DAG of render passes with
  automatic barrier inference and a transient resource allocator skeleton.
    - `RenderGraph` (wraps `petgraph::DiGraph<PassId, ()>`) with
      `add_resource` / `add_pass` / `compile` / `pass` / `resource` /
      `pass_count` / `resource_count` accessors. `compile()` infers
      writer→reader edges from pass `reads`/`writes` declarations, topsorts
      the DAG, walks the topological order emitting per-resource barriers,
      and transfers the pass objects into the returned `CompiledGraph`.
    - `Pass` trait with `name` / `reads` / `writes` / `record` methods and
      optional `texture_usages` / `buffer_usages` usage hints (used by the
      barrier inference to fill in the `to` usage field; default is empty).
    - `PassContext` exposes the `wgpu::CommandEncoder`, the compiled
      resource table (for resolving `ResourceHandle` → `wgpu::Texture` /
      `wgpu::Buffer`), and a `set_texture` / `set_buffer` mutator on the
      `ResourceTable` for installing persistent resources before
      `execute`.
    - `ResourceHandle`, `ResourceKind::{Texture,Buffer}`,
      `ResourceLifetime::{Transient,Persistent}`, `TextureDesc`,
      `BufferDesc` (all use the `wgpu::TextureFormat` /
      `wgpu::TextureUsages` / `wgpu::BufferUsages` types directly).
    - `Barrier::{Texture,Buffer}{resource,from,to}` matching the
      `wgpu::TextureUses` / `wgpu::BufferUses` spec from
      `docs/architecture.md` §6.3.
    - `TransientAllocator` (skeleton) with `register` / `descriptor` /
      `mark_live` / `next_frame` hooks; the actual first-fit pool
      reuses matching-size slots across frames, scheduled for R-022.
    - `CompiledGraph` + `CompiledPass` with `passes` / `barriers` /
      `table` / `table_mut` / `allocator` / `allocator_mut` /
      `resource_entries` / `execute(encoder)` accessors. `execute()`
      walks the pass sequence pushing a `wgpu` debug group per pass,
      logging the inferred barriers, and calling each pass's
      `Pass::record`. Per-resource `wgpu` transition emission is the
      next item (R-021) — wgpu 22's high-level `CommandEncoder` does
      not expose per-resource `transition_resources` directly.
    - 3 integration tests in `compile.rs` (skip gracefully when no wgpu
      adapter is available): 3-pass graph topological order + barrier
      emission; cycle detection; double-writer rejection. Plus small
      unit tests in every module (handle display/predicates, barrier
      display, default trait methods, allocator liveness, graph
      builder sanity).
- **Core error variant:** `HygeError::RenderGraphCycle(String)` +
  `HygeError::render_graph_cycle()` constructor, returned by
  `RenderGraph::compile` on cycle detection.
- **Workspace dependency:** `pollster = "0.4"` for blocking on wgpu's
  async `request_adapter` / `request_device` in tests.
