# Hyge — Architecture

> **Document type:** architecture specification. The implementation roadmap (M0–M7) lives in `docs/roadmap.toml`. Development instructions for agents live in `AGENTS.md` (project root).
> **Audience:** implementation-capable agents who will execute the roadmap and the human reviewers who will sign off on each milestone.
> **Status:** locked decisions; new decisions require an ADR in `docs/adr/`.

---

## Table of Contents

1. [Vision & Goals](#1-vision--goals)
2. [Glossary](#2-glossary)
3. [Architectural Principles](#3-architectural-principles)
4. [Closed Architectural Decisions](#4-closed-architectural-decisions)
5. [Workspace & Crate Map](#5-workspace--crate-map)
6. [Crate-by-Crate Specification](#6-crate-by-crate-specification)
7. [Runtime: Schedules, Threading, FrameSnapshot](#7-runtime-schedules-threading-framesnapshot)
8. [Renderer Architecture](#8-renderer-architecture)
9. [Asset Pipeline](#9-asset-pipeline)
10. [Scene, Prefab, Instancing](#10-scene-prefab-instancing)
11. [Cross-Cutting Subsystems](#11-cross-cutting-subsystems)
12. [Editor (`hyge-editor`)](#12-editor-hyge-editor)
13. [Tools (`hyge-tools`)](#13-tools-hyge-tools)
14. [Coding Standards & Unsafe Policy](#14-coding-standards--unsafe-policy)
15. [Versioning, Compatibility, Release Channels](#15-versioning-compatibility-release-channels)
16. [Risks & Mitigations](#16-risks--mitigations)
17. [Out of Scope for v0.1 (Explicit)](#17-out-of-scope-for-v01-explicit)
18. [Open Questions for Future ADRs](#18-open-questions-for-future-adrs)

---

## 1. Vision & Goals

- **Base AAA-ready**: bindless rendering, render graph, clustered forward with PBR+IBL, meshlet LOD, hybrid CPU/GPU culling.
- **Modern ECS**: archetypal, parallel schedules, relations, observers, change detection, reflection-driven scripting.
- **Content pipeline**: glTF source-of-truth → BLAKE3 content-addressed DB → hot-reload, with CLI importer that bakes meshlets/clusters and compresses.
- **Full editor** in v0.1: viewport, hierarchy, inspector, content browser, hot-reload of scene.
- **Single-player local in v0.1**; networking deferred to v0.2.
- **Production-grade quality bar**: rustdoc on every public item, snapshot tests for renderer, fuzz tests for parsers, CI on Windows + Linux, zero clippy warnings.

---

## 2. Glossary

| Term | Meaning |
|------|---------|
| ECS | Entity Component System (`bevy_ecs`). |
| SoA | Structure of Arrays. Renderer reads SoA views to maximize `wgpu::Buffer` throughput. |
| Archetype | A `bevy_ecs` archetype: a set of components. |
| Bindless | Rendering pattern where one large descriptor heap is indexed by `uint` IDs at draw time; no per-draw bind updates. |
| Render Graph | DAG of passes with declared reads/writes on resources. Used to infer barriers and lifetime. |
| Transient Resource | A render-graph-managed resource (texture/buffer) that is allocated and freed within a frame. |
| Persistent Resource | A render-graph resource whose lifetime spans many frames (e.g. shadow atlas, bindless heap). |
| Clustered Forward | Forward shading with light culling in (Z-cluster, XY-tile) buckets. |
| Light Grid | Per-tile list of light indices + offsets, consumed by the forward shader. |
| Meshlet | A small cluster of typically 64–128 vertices and ≤256 indices, used for fine-grained culling. |
| LOD | Level of Detail. |
| CSM | Cascaded Shadow Map. |
| IBL | Image-Based Lighting (prefilter cubemap + irradiance). |
| PBR | Physically-Based Rendering (metallic-roughness). |
| TAA | Temporal Anti-Aliasing. |
| SMAA | Subpixel Morphological Anti-Aliasing. |
| ACES | Academy Color Encoding System (tonemap operator). |
| FrameSnapshot | Immutable per-frame struct produced by `RenderExtract`, consumed by the render thread. |
| Prefab | A reusable entity template, content-addressed by BLAKE3. |
| `HygePlugin` | Trait every crate implements to register systems, resources, and events into the `App`. |
| `prelude` | Re-export module of a crate that callers `use` to bring in the most-used types. |
| `A: Asset` | Trait bound for an asset type: BLAKE3 hashable, deserializable, GPU-uploadable. |
| DCC | Digital Content Creation tool (Blender, Maya, …). |
| Hot-reload | Recompiling/reimporting assets at runtime without restarting the engine. |
| Content Addressing | Referencing files by cryptographic hash (BLAKE3) of their contents, not by path. |
| Persistent Component | A component flagged to survive hot-reload of scene. |
| ADR | Architecture Decision Record (in `docs/adr/NNNN-title.md`). |

---

## 3. Architectural Principles

1. **Crate boundaries are API contracts.** Every public surface goes through a crate's `prelude` and is registered as a `HygePlugin`.
2. **Data-oriented from the start.** Renderer reads SoA views; ECS components are `bytemuck::Pod + Zeroable` where they cross the GPU boundary.
3. **No global state.** All engine subsystems are `Resource`s, components, or `Plugin`s.
4. **Render thread is read-only on the World.** It consumes a per-frame `FrameSnapshot` extracted in `RenderExtract`.
5. **Content addressing everywhere.** Assets are referenced by BLAKE3 hash; hot-reload is invalidated by hash change.
6. **Reflect everything exposed to scripting.** `bevy_reflect` is the bridge between Rust types and Lua.
7. **`unsafe` is gated, documented, and tested.** No `unsafe` outside `hyge-render` and `hyge-render-graph`; every `unsafe` block has a `// SAFETY:` comment; `miri` runs nightly on those crates.
8. **Public APIs are minimal and orthogonal.** Every public type earns its keep. "Add a field" is a breaking change; design accordingly.
9. **ECS archetypes > reflection > scripting.** Components are first-class Rust types; scripting reads them through `bevy_reflect`; never re-encode components in a parallel type system.
10. **Render graph > hardcoded pipeline.** New passes are declared, not inlined. Barrier inference is automatic.

---

## 4. Closed Architectural Decisions

These are binding. Any deviation requires a new ADR in `docs/adr/`.

| # | Decision | Choice | ADR |
|---|----------|--------|-----|
| 1 | Graphics API | `wgpu` 0.20+ (DX12/Vulkan/Metal/WebGPU). PC Windows primary. | ADR-0001 |
| 2 | ECS | `bevy_ecs` 0.14+ (archetypal, parallel schedule, observers, relations, change detection). | ADR-0002 |
| 3 | Renderer | Bindless + Render Graph + Clustered Forward (PBR+IBL) + hybrid CPU/GPU culling + meshlet LOD. | ADR-0003 |
| 4 | Workspace | 14 fat crates (Bevy-style) with `prelude` + `HygePlugin`. | ADR-0004 |
| 5 | Concurrency | Multi-thread; dedicated render thread; per-frame `FrameSnapshot`. | ADR-0005 |
| 6 | Asset pipeline | BLAKE3 content-addressed DB; glTF source; `.hyge-world` binary scene; hot-reload via `notify`; CLI importer with LZ4 + Zstd + meshlet pre-process. | ADR-0006 |
| 7 | Scene/Prefab | Hierarchical BLAKE3-keyed prefabs, instancing, override, sub-archetypes `StaticMesh`/`DynamicMesh`. | ADR-0007 |
| 8 | Scripting | Lua via `mlua` + `bevy_reflect`-derived bindings + hot-reload. WASM deferred to v0.2. | ADR-0008 |
| 9 | Physics | `rapier3d` 0.18+ wrapped in `hyge-physics` behind feature flag `physics-rapier`. | ADR-0009 |
| 10 | Audio | `kira` 0.9+ + spatial 3D + streaming + HRTF optional + bus-by-category. | ADR-0010 |
| 11 | Windowing/Input | `winit` 0.30 + `gilrs` 0.10+ + `Action<T>` + TOML bindings + raw input (Windows) + hot-reload. | ADR-0011 |
| 12 | Networking | Out of scope for v0.1. v0.2: `hyge-net` with `quinn`/QUIC. | ADR-0012 |
| 13 | Editor | CLI tools + optional `egui` debug overlay + external PySide6/QML editor in v0.1. | ADR-0015 |
| 14 | Build | `cargo` workspace, `stable` only. No nightly. MSRV 1.80. | ADR-0014 |

---

## 5. Workspace & Crate Map

### 5.1 Directory layout

```
hyge/                                     # workspace root
├── Cargo.toml                            # [workspace] + resolver = "2"
├── rust-toolchain.toml                   # stable channel pinned (1.80+)
├── AGENTS.md                             # agent-facing instructions
├── README.md
├── CHANGELOG.md
├── .github/workflows/                    # CI workflows
│   ├── ci.yml                            # build, test, lint
│   └── nightly.yml                       # miri, udeps, fuzz
├── docs/
│   ├── architecture.md                   # this file
│   ├── roadmap.toml                      # R-XXX implementation backlog
│   ├── adr/                              # Architecture Decision Records
│   ├── mdbook/                           # user-facing tutorials (built in M7)
│   └── perf/                             # performance baselines
├── crates/
│   ├── hyge-core/
│   ├── hyge-ecs/
│   ├── hyge-render/
│   ├── hyge-render-graph/
│   ├── hyge-asset/
│   ├── hyge-scene/
│   ├── hyge-physics/
│   ├── hyge-audio/
│   ├── hyge-window/
│   ├── hyge-input/
│   ├── hyge-script/
│   ├── hyge-app/
│   ├── hyge-editor/
│   ├── hyge-tools/
│   └── hyge-runtime-test/
└── examples/
    └── hyge-sandbox/                     # built in M7; demonstrates every system
```

### 5.2 Crate map (purpose)

| Crate | Purpose | Depends on (Cargo deps, not internal) |
|---|---|---|
| `hyge-core` | Math (`glam`), color spaces, log (`tracing`), asserts, UUID, prelude. | `glam`, `tracing`, `thiserror`, `bytemuck` |
| `hyge-ecs` | Re-exports `bevy_ecs` + defines `Schedule`s, `SystemSet`s, `HygePlugin` trait, `Resource` derive macros if needed. | `bevy_ecs`, `bevy_tasks`, `bevy_utils` |
| `hyge-render` | wgpu device, bindless table, clustered forward, meshlet culling, post-process, frame snapshot producer. | `wgpu`, `bytemuck`, `bitflags`, `naga` |
| `hyge-render-graph` | DAG, barrier inference, transient allocator, pass compilation. | `wgpu`, `petgraph`, `indexmap` |
| `hyge-asset` | BLAKE3 DB, loader registry, hot-reload watcher, refcounted handles. | `rusqlite`, `blake3`, `notify`, `tokio` (or `async-channel`), `lz4_flex`, `zstd`, `serde`, `rmp-serde` |
| `hyge-scene` | glTF loader, `.hyge-world` (de)serialization, prefab system, instancing extraction. | `gltf`, `meshopt`, `serde`, `rmp-serde`, `ron` |
| `hyge-physics` | `rapier3d` wrapper, fixed-timestep, character controller, queries, events. | `rapier3d` |
| `hyge-audio` | `kira` backend, spatial 3D, bus mixer, HRTF. | `kira`, `kira-spatial-audio`, `oddio` (optional HRTF) |
| `hyge-window` | winit event loop, raw input (Windows), surface, swapchain. | `winit`, `raw-window-handle`, `windows-sys` |
| `hyge-input` | `Action<T>`, TOML bindings, event translation, hot-reload. | `toml`, `serde`, `notify` |
| `hyge-script` | mlua runtime, reflect bindings, script hot-reload, sandbox. | `mlua`, `bevy_reflect` |
| `hyge-app` | `App` builder, plugin assembly, runtime orchestration. | `hyge-ecs`, `hyge-window`, `hyge-asset`, `hyge-render`, `hyge-render-graph`, `hyge-scene`, `hyge-physics`, `hyge-audio`, `hyge-input`, `hyge-script` |
| `hyge-editor` | Rust editor service: protocol, snapshots, commands, undo/redo and engine integration. | `hyge-editor-protocol`, `hyge-app`, `hyge-scene`, `hyge-render` |
| `hyge-editor-protocol` | Versioned JSON framing and message types shared with the Qt frontend. | `serde`, `serde_json` |
| `hyge-tools` | CLI: `import`, `cook`, `headless`, `inspect`, `serve`, `doctor`. | `clap`, `hyge-asset`, `hyge-scene`, `hyge-render`, `hyge-app` |
| `hyge-runtime-test` | Headless wgpu test harness (no surface); reusable assertion utilities. | `wgpu`, `image`, `hyge-render` |

### 5.3 Internal dependency graph (acyclic)

```
                    ┌────────────┐
                    │ hyge-core  │
                    └─────┬──────┘
                          │
                    ┌─────▼──────┐
                    │ hyge-ecs   │
                    └─────┬──────┘
       ┌────────────┬─────┼─────┬────────────┬─────────────┬───────────┐
       ▼            ▼     ▼     ▼             ▼             ▼           ▼
 hyge-asset   hyge-render  hyge-scene  hyge-physics  hyge-audio  hyge-input  hyge-script
       │            │         │            │            │           │            │
       │      ┌─────▼─────┐   │            │            │           │            │
       │      │hyge-render│   │            │            │           │            │
       │      │   -graph  │   │            │            │           │            │
       │      └───────────┘   │            │            │           │            │
       │                      │            │            │           │            │
       └──────────┬───────────┴─────┬──────┴──────┬─────┴─────┬─────┴─────┬──────┘
                  ▼                 ▼             ▼           ▼           ▼
                hyge-app          hyge-window (depends on hyge-ecs, hyge-input)
                  │
       ┌──────────┼──────────┐
       ▼          ▼          ▼
 hyge-editor   hyge-tools  examples/hyge-sandbox
```

`hyge-app` is the only crate that knows the full graph. Everything below it composes via `HygePlugin` registration.

### 5.4 Workspace `[workspace.dependencies]`

To enforce single-version policy, every external dep must be declared once in root `Cargo.toml` under `[workspace.dependencies]` and referenced as `dep = { workspace = true }` in member crates.

---

## 6. Crate-by-Crate Specification

For each crate, the implementation must:
- expose a `prelude` module re-exporting the public surface.
- expose a `HygePlugin` impl (except leaf utility crates).
- contain `#[cfg(test)]` unit tests for non-trivial logic.
- compile with `cargo clippy --workspace --all-targets -- -D warnings`.
- have rustdoc on every public item (CI-enforced).

### 6.1 `hyge-core`

**Purpose:** foundational types used everywhere.

**Public surface (sketch):**
```rust
// crates/hyge-core/src/lib.rs
pub mod math;       // glam re-exports + Vec3/Quat/Mat4 helpers
pub mod color;      // LinearSRGB, sRGB, ACEScg, OkLab, conversion
pub mod log;        // init_tracing(), spans, hyge_log! macro
pub mod id;         // AssetId(blake3::Hash), EntityId(u32 generation)
pub mod prelude;    // re-exports
pub mod result;     // HygeError, HygeResult<T>

pub fn version() -> &'static str;
```

**File layout:**
```
hyge-core/src/
  lib.rs
  math.rs
  color.rs
  log.rs
  id.rs
  result.rs
  prelude.rs
```

**Notable types:**
- `AssetId` = newtype around `[u8; 32]` (BLAKE3 hash).
- `color::LinearRGB` (f32×3) and `color::Srgb` (u8×3) with explicit conversion.
- `math::Aabb`, `math::Frustum`, `math::Ray`.

**Tests required:** `color.rs` (round-trip conversions, gamma), `math.rs` (AABB ops, frustum extraction), `id.rs` (BLAKE3 hash determinism).

### 6.2 `hyge-ecs`

**Purpose:** the only crate that imports `bevy_ecs` directly. Defines `HygePlugin`, `Schedule`s, `SystemSet`s, `Resource` derives if needed.

**Public surface (sketch):**
```rust
pub use bevy_ecs::*;
pub use bevy_tasks::{AsyncComputeTaskPool, IoTaskPool, TaskPool};
pub use bevy_reflect::Reflect;

pub trait HygePlugin {
    fn name(&self) -> &'static str;
    fn build(&self, app: &mut App);
}

pub mod schedule {
    pub enum Label { First, PreUpdate, FixedUpdate, Update, RenderExtract, Render, Last }
    impl ScheduleLabel for Label { /* ... */ }
}

pub mod set {
    #[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
    pub enum TransformSet { Propagate, Sync, Flush }
    // ... InputSet, AssetSet, PhysicsSet, AudioSet, ScriptSet, EditorSet
}
```

**File layout:**
```
hyge-ecs/src/
  lib.rs
  plugin.rs
  schedule.rs
  set.rs
  prelude.rs
```

**Tests required:** `plugin.rs` (plugin registration order), `schedule.rs` (label uniqueness).

### 6.3 `hyge-render-graph`

**Purpose:** DAG of passes; barrier inference; transient resource allocation.

**Public surface (sketch):**
```rust
pub struct RenderGraph { /* internal: petgraph DiGraph */ }
pub struct GraphBuilder<'a> { graph: &'a mut RenderGraph }

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct PassId(usize);

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct ResourceHandle(u32);

pub enum ResourceKind { Texture(TextureDesc), Buffer(BufferDesc) }
pub enum ResourceLifetime { Transient, Persistent }

pub trait Pass {
    type Input;
    type Output;
    fn name(&self) -> &str;
    fn reads(&self) -> Box<dyn Iterator<Item = ResourceHandle>>;
    fn writes(&self) -> Box<dyn Iterator<Item = ResourceHandle>>;
    fn record(&mut self, ctx: &mut PassContext<'_>);
}

pub struct CompiledGraph { /* pre-compiled pass sequence + barriers */ }

impl RenderGraph {
    pub fn new() -> Self;
    pub fn add_pass<P: Pass>(&mut self, pass: P) -> PassId;
    pub fn add_resource(&mut self, kind: ResourceKind, lt: ResourceLifetime) -> ResourceHandle;
    pub fn add_edge(&mut self, from: PassId, to: PassId, resource: ResourceHandle);
    pub fn compile(&mut self, device: &wgpu::Device) -> CompiledGraph;
    pub fn execute(&self, encoder: &mut wgpu::CommandEncoder);
}
```

**Algorithm — barrier inference:**
1. Topological order of passes via `petgraph::algo::toposort`.
2. For each pass, walk all reads/writes; for each resource touched, if previous pass wrote it (or it was `Persistent`), emit a barrier matching (texture usage, buffer usage) transition.
3. For `Transient` resources, an internal arena allocator is used; first-touch allocates, last-release frees.

**File layout:**
```
hyge-render-graph/src/
  lib.rs
  graph.rs
  pass.rs
  resource.rs
  barrier.rs
  allocator.rs
  compile.rs
  prelude.rs
```

**Tests required (CRITICAL — this crate is `unsafe`-containing):** barrier inference (insta snapshot of inferred barriers for a fixed graph), allocator reuse (no leaks between frames), cycle detection.

### 6.4 `hyge-render`

**Purpose:** the runtime renderer (device init, swapchain, bindless table, clustered forward, meshlet culling, post-process).

**Public surface (sketch):**
```rust
pub struct Renderer { /* device, queue, surface, swapchain, bindless, render graph */ }
pub struct RendererConfig { pub backend: Backends, pub vsync: bool, /* ... */ }

pub struct BindlessTable {
    pub meshes: StorageBuffer<GpuMesh>,
    pub materials: StorageBuffer<GpuMaterial>,
    pub textures: TextureArray<Rgba16Float>,
    pub samplers: SamplerArray,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMesh { pub vertex_offset: u32, pub index_offset: u32, pub meshlet_offset: u32, pub meshlet_count: u32, pub aabb_min: [f32;3], pub aabb_max: [f32;3], pub lod_count: u32, _pad: u32 }

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMaterial { pub base_color: u32, pub normal: u32, pub mr: u32, pub occlusion: u32, pub emissive: u32, pub roughness: f32, pub metallic: f32, pub alpha_mode: u32, pub flags: u32 }

pub struct DrawCommand { pub mesh_id: u32, pub material_id: u32, pub first_instance: u32, pub instance_count: u32, pub aabb: Aabb }

pub struct FrameSnapshot {
    pub frame_index: u64,
    pub view: ViewData,
    pub lights: LightGrid,
    pub opaque: Vec<DrawCommand>,
    pub transparent: Vec<DrawCommand>,
    pub skinned: Vec<DrawCommand>,
}

impl Renderer {
    pub fn new(config: RendererConfig, window: &Window) -> HygeResult<Self>;
    pub fn resize(&mut self, w: u32, h: u32);
    pub fn begin_frame(&mut self) -> FrameContext;
    pub fn end_frame(&mut self, snapshot: &FrameSnapshot);
    pub fn bindless(&self) -> &BindlessTable;
    pub fn bindless_mut(&mut self) -> &mut BindlessTable;
}
```

**Subsystems (each in its own module):**
- `device.rs` — wgpu init, surface, swapchain, depth format selection.
- `bindless.rs` — table layout, GPU upload path.
- `clustered.rs` — light grid build (compute or CPU), cluster Z split, tile XY split.
- `forward.rs` — opaque + alpha-test pass, shaders, vertex/fragment layout.
- `shadow.rs` — CSM (4 splits, PCF soft) + per-light atlas.
- `cull.rs` — CPU frustum culling for static, GPU compute for dynamic.
- `meshlet.rs` — meshlet/cluster compute shader, meshlet culling shader, indirect arg write.
- `post.rs` — tonemap, TAA (history reprojection + variance clip), SMAA, bloom (dual Kawase).
- `skinning.rs` — skinning compute pass (matrices from animation), outputs skinned vertex buffer.
- `ibl.rs` — prefilter + irradiance bake.
- `frame.rs` — FrameSnapshot producer/consumer.
- `profiler.rs` — GPU timestamp queries, label scopes.

**File layout:**
```
hyge-render/src/
  lib.rs
  device.rs
  bindless.rs
  clustered.rs
  forward.rs
  shadow.rs
  cull.rs
  meshlet.rs
  post.rs
  skinning.rs
  ibl.rs
  frame.rs
  profiler.rs
  shader/                  # WGSL embedded via include_str!
    common.wgsl
    pbr.wgsl
    clustered_forward.wgsl
    light_grid.wgsl
    meshlet_cull.wgsl
    skinning.wgsl
    shadow_csm.wgsl
    shadow_atlas.wgsl
    tonemap.wgsl
    taa.wgsl
    smaa.wgsl
    bloom.wgsl
    prefilter.wgsl
    irradiance.wgsl
  prelude.rs
```

**WGSL layout:** all shaders import `common.wgsl` for shared bindings (`bindless`, `view`, `light_grid`, `frame`).

**Tests required (CRITICAL):** `bindless.rs` (slot allocation round-trip), `clustered.rs` (light grid determinism), `frame.rs` (snapshot extract cost benchmark).

### 6.5 `hyge-asset`

**Purpose:** content-addressed asset DB, loader registry, hot-reload, GPU upload.

**Public surface (sketch):**
```rust
pub trait Asset: Send + Sync + 'static {
    type Data: Send + Sync + 'static;
    fn hash(data: &Self::Data) -> blake3::Hash;
    fn extensions() -> &'static [&'static str];
    fn load(bytes: &[u8], ctx: &mut LoadContext) -> HygeResult<Self::Data>;
}

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct AssetId(pub [u8; 32]);

pub struct AssetServer { /* loader registry, DB, watcher */ }
pub struct Handle<A: Asset> { id: AssetId, _marker: PhantomData<A> }
pub struct LoadedAsset<A: Asset> { asset: Arc<A>, version: u64 }

pub enum LoadState { Loading, Loaded, Failed(HygeError) }

pub struct AssetEvent { pub id: AssetId, pub version: u64 }

impl AssetServer {
    pub fn new(db: AssetDb, watcher: FileWatcher) -> Self;
    pub fn load<A: Asset>(&self, id: AssetId) -> Handle<A>;
    pub fn get<A: Asset>(&self, handle: Handle<A>) -> Option<LoadedAsset<A>>;
    pub fn register_loader<A: Asset>(&mut self, loader: Box<dyn AssetLoader<A>>);
    pub fn watcher(&mut self) -> &mut FileWatcher;
}

pub struct AssetDb { db: rusqlite::Connection, cache_dir: PathBuf }
impl AssetDb {
    pub fn open(path: &Path) -> HygeResult<Self>;
    pub fn lookup(&self, hash: &AssetId) -> Option<PathBuf>;
    pub fn insert(&mut self, hash: &AssetId, path: &Path) -> HygeResult<()>;
    pub fn dependencies(&self, hash: &AssetId) -> Vec<AssetId>;
    pub fn add_dependency(&mut self, parent: &AssetId, child: &AssetId) -> HygeResult<()>;
}

pub struct FileWatcher { /* notify::RecommendedWatcher */ }
```

**File layout:**
```
hyge-asset/src/
  lib.rs
  asset.rs
  handle.rs
  server.rs
  db.rs
  watcher.rs
  loader.rs
  io.rs
  gpu_upload.rs
  prelude.rs
```

**Hot-reload algorithm:**
1. `notify` watcher emits raw FS event.
2. Resolve event to `AssetId` via DB lookup (path → hash).
3. Schedule re-import on `AsyncComputeTaskPool`.
4. On completion, push new `AssetEvent` to `EventWriter<AssetEvent>`.
5. Systems that hold `Handle<A>` call `server.get(handle)`; refcount bump on new version, decrement on old.

**GpuUploadContext:** when a mesh/asset finishes loading, a `GpuUploadTask` runs on `AsyncComputeTaskPool`, creates `wgpu::Buffer` and registers in `BindlessTable` under a new ID; old ID remains valid until refcount zero.

**Tests required:** `db.rs` (SQLite round-trip with `serial_test`), `loader.rs` (loader registry dispatch), `gpu_upload.rs` (slot allocator under contention).

### 6.6 `hyge-scene`

**Purpose:** glTF import, `.hyge-world`/`.hyge-prefab` format, prefab system, instancing extraction.

**Public surface (sketch):**
```rust
pub struct GltfImporter { /* uses gltf crate */ }
pub struct WorldLoader { /* .hyge-world */ }
pub struct PrefabLoader { /* .hyge-prefab */ }

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct PrefabId(pub AssetId);

pub struct Prefab { pub root: PrefabNode, pub assets: PrefabAssets }
pub struct PrefabNode { pub name: String, pub components: Vec<ComponentOverride>, pub children: Vec<PrefabNode> }
pub struct ComponentOverride { pub type_id: std::any::TypeId, pub value: Box<dyn Reflect> }
pub struct PrefabAssets { pub meshes: Vec<AssetId>, pub materials: Vec<AssetId>, pub scripts: Vec<AssetId> }

pub struct Scene { pub env: Environment, pub root_entities: Vec<Entity> }
pub struct Environment { pub skybox: Option<AssetId>, pub sun: Option<DirectionalLight>, pub fog: Option<FogParams>, pub post: PostProcessProfile }

impl Prefab {
    pub fn instantiate(&self, world: &mut World, transforms: Transform, parent: Option<Entity>) -> Entity;
}

pub mod components {
    #[derive(Component, Reflect, Default)] pub struct Transform { pub translation: Vec3, pub rotation: Quat, pub scale: Vec3 }
    #[derive(Component, Reflect, Default)] pub struct GlobalTransform(pub Mat4);
    #[derive(Component, Reflect, Default)] pub struct StaticMesh { pub mesh: Handle<Mesh>, pub material: Handle<Material> }
    #[derive(Component, Reflect, Default)] pub struct DynamicMesh { pub mesh: Handle<Mesh>, pub material: Handle<Material> }
    #[derive(Component, Reflect, Default)] pub struct SkinnedMesh { pub mesh: Handle<Mesh>, pub skeleton: Handle<Skeleton>, pub material: Handle<Material> }
    #[derive(Component, Reflect, Default)] pub struct Name(pub String);
    #[derive(Component, Reflect, Default)] pub struct PersistOnReload;
    // ... PointLight, SpotLight, DirectionalLight, Camera, AudioSource, ScriptRef, RigidBody, Collider
}
```

**File layout:**
```
hyge-scene/src/
  lib.rs
  components.rs
  gltf_import.rs
  meshlet_bake.rs
  prefab.rs
  world.rs
  env.rs
  extract.rs
  prelude.rs
```

**Meshlet bake algorithm:**
1. Build vertex/index buffers from glTF mesh.
2. For each primitive: `meshopt_buildMeshlets` (target 64 verts, 126 tri cap), compute meshlet bounds (cone, AABB).
3. Generate LOD chain: simplify with `meshopt_simplify` (0.5, 0.25, 0.1 ratios).
4. Serialize to `.hyge-mesh` (custom binary: header + meshlet data + LOD data).

**Scene extract (in `RenderExtract`):**
- For each entity with `StaticMesh`, push `DrawCommand { mesh_id, material_id, instance_count = 1, ... }` to `opaque`.
- Group by `(mesh_id, material_id)` and merge instance counts when `Aabb` overlap tolerance allows.
- Skinned/dynamic: same but with `skinned` bucket.

**Tests required:** `meshlet_bake.rs` (deterministic bake for fixed input), `prefab.rs` (round-trip), `world.rs` (`.hyge-world` deserialization).

### 6.7 `hyge-physics`

**Purpose:** wrap `rapier3d` behind feature flag; expose ECS components and events.

**Public surface (sketch):**
```rust
#[cfg(feature = "physics-rapier")]
pub mod rapier_impl;

pub mod components {
    #[derive(Component, Reflect, Default)] pub struct RigidBody { pub kind: RigidBodyKind, pub ccd: bool, pub gravity_scale: f32, pub linear_damping: f32, pub angular_damping: f32 }
    pub enum RigidBodyKind { Dynamic, Fixed, KinematicPosition, KinematicVelocity }
    #[derive(Component, Reflect, Default)] pub struct Collider { pub shape: ColliderShape, pub density: f32, pub friction: f32, pub restitution: f32, pub is_sensor: bool }
    pub enum ColliderShape { Ball(f32), Cuboid(Vec3), Capsule { half_height: f32, radius: f32 }, Cylinder { half_height: f32, radius: f32 }, Cone { half_height: f32, radius: f32 }, ConvexHull(Vec<Vec3>), Trimesh { vertices: Vec<Vec3>, indices: Vec<u32> }, Heightfield { heights: Vec<f32>, scale: Vec2 } }
    #[derive(Component, Reflect, Default)] pub struct CharacterController { pub max_slope: f32, pub step_height: f32, pub jump: f32 }
}

pub mod events {
    #[derive(Event)] pub struct CollisionEvent { pub entity_a: Entity, pub entity_b: Entity, pub started: bool, pub contact: Option<Contact> }
    #[derive(Event)] pub struct ContactForceEvent { pub entity_a: Entity, pub entity_b: Entity, pub total_force: Vec3, pub total_torque: Vec3 }
}

pub mod query {
    pub struct RayHit { pub entity: Entity, pub toi: f32, pub normal: Vec3 }
    pub struct ShapeHit { pub entity: Entity, pub toi: f32, pub witness1: Vec3, witness2: Vec3, pub normal: Vec3 }
    pub trait SpatialQuery {
        fn cast_ray(&self, origin: Vec3, dir: Vec3, max_toi: f32, filter: QueryFilter) -> Option<RayHit>;
        fn cast_shape(&self, shape: ColliderShape, origin: Vec3, dir: Vec3, max_toi: f32, filter: QueryFilter) -> Option<ShapeHit>;
        fn intersections_with(&self, shape: ColliderShape, origin: Vec3, rotation: Quat, filter: QueryFilter) -> Vec<Entity>;
    }
}

pub struct PhysicsConfig { pub fixed_timestep: f32, pub max_substeps: u32, pub gravity: Vec3, pub num_solver_iterations: usize }
pub struct PhysicsTime { pub accumulator: f32, pub timestep: f32 }
```

**File layout:**
```
hyge-physics/src/
  lib.rs
  components.rs
  events.rs
  query.rs
  config.rs
  step.rs
  rapier_impl/
    mod.rs
    body.rs
    collider.rs
    joint.rs
    island.rs
    char_controller.rs
  prelude.rs
```

**Fixed timestep:**
- `FixedUpdate` schedule runs `physics_step` N times per render frame as needed to consume accumulator (max substeps).
- Worker thread runs island build, returns `Island` to main.

**Tests required:** `components.rs` (Reflect round-trip), `query.rs` (raycast against static box), `step.rs` (deterministic ball drop).

### 6.8 `hyge-audio`

**Purpose:** `kira` backend, spatial 3D, bus mixer, HRTF.

**Public surface (sketch):**
```rust
pub struct AudioServer { manager: kira::AudioManager, mixer: kira::track::TrackHandle, buses: AudioBuses, listener: Listener }
pub struct AudioBuses { master: TrackHandle, sfx: TrackHandle, voice: TrackHandle, ui: TrackHandle, ambient: TrackHandle, music: TrackHandle }

pub mod components {
    #[derive(Component, Reflect, Default)] pub struct AudioSource { pub clip: Handle<AudioClip>, pub bus: AudioBus, pub spatial: bool, pub volume: f32, pub pitch: f32, pub range: f32, pub rolloff: AudioRolloff }
    pub enum AudioBus { Sfx, Voice, Ui, Ambient, Music }
    pub enum AudioRolloff { Linear, Inverse, Logarithmic }
    #[derive(Component, Reflect, Default)] pub struct AudioListener;
}

pub mod events { #[derive(Event)] pub struct PlaySound { pub source: Entity, pub volume: f32, pub pitch: f32 } #[derive(Event)] pub struct StopSound { pub source: Entity } }

#[cfg(feature = "audio-hrtf")]
pub mod hrtf { pub fn init(manager: &mut AudioManager, dataset: HrtfDataset) -> HygeResult<()>; }
```

**File layout:**
```
hyge-audio/src/
  lib.rs
  server.rs
  bus.rs
  spatial.rs
  components.rs
  events.rs
  hrtf.rs
  prelude.rs
```

**Tests required:** `bus.rs` (bus graph, volume inheritance), `spatial.rs` (distance attenuation math), `server.rs` (mock-only — no audio device in CI).

### 6.9 `hyge-window`

**Purpose:** winit event loop, raw input (Windows), surface.

**Public surface (sketch):**
```rust
pub struct WindowPlugin;
impl HygePlugin for WindowPlugin { /* registers Window resource, events */ }

pub struct WindowConfig { pub title: String, pub width: u32, pub height: u32, pub resizable: bool, pub vsync: bool, pub raw_input: bool }

pub struct Window { /* winit::Window wrapper */ }
pub struct RawMouseDelta { pub dx: f32, pub dy: f32 }
pub struct RawKeyState { pub scancode: u32, pub pressed: bool }

pub mod events {
    #[derive(Event)] pub struct WindowResized { pub w: u32, pub h: u32 }
    #[derive(Event)] pub struct WindowCloseRequested;
    #[derive(Event)] pub struct WindowFocused { pub focused: bool }
    #[derive(Event)] pub struct DeviceEvent { pub event: DeviceEventKind }
    pub enum DeviceEventKind { MouseMotion { dx: f32, dy: f32 }, MouseWheel { dx: f32, dy: f32 }, Key { scancode: u32, pressed: bool }, GamepadConnected { id: GamepadId, name: String }, GamepadDisconnected { id: GamepadId } }
}
```

**File layout:**
```
hyge-window/src/
  lib.rs
  plugin.rs
  raw_input.rs
  surface.rs
  events.rs
  prelude.rs
```

**Raw input (Windows):** `RegisterRawInputDevices` for keyboard and mouse, polled in `winit::event_loop::EventLoop` and translated to `DeviceEvent`.

**Tests required:** `events.rs` (event translation table), `raw_input.rs` (Windows-only, `#![cfg(windows)]`).

### 6.10 `hyge-input`

**Purpose:** `Action<T>` abstraction, TOML bindings, hot-reload.

**Public surface (sketch):**
```rust
pub struct ActionMap { pub actions: HashMap<String, Action>, pub bindings: HashMap<String, Vec<Binding>> }
pub enum Action { Button(ButtonAction), Axis(AxisAction), Vec2(Vec2Action) }
pub struct ButtonAction { pub state: bool, pub just_pressed: bool, pub just_released: bool, pub value: f32 }
pub struct AxisAction { pub value: f32, pub raw: f32 }
pub struct Vec2Action { pub value: Vec2, pub raw: Vec2 }

pub enum Binding { Keyboard { scancode: u32, modifier: Modifiers }, MouseButton { button: u32 }, MouseAxis { axis: MouseAxis, scale: f32 }, MouseDelta, GamepadButton { id: u32, button: u32 }, GamepadAxis { id: u32, axis: u32, scale: f32 } }
pub struct Modifiers { pub ctrl: bool, pub shift: bool, pub alt: bool, pub logo: bool }

pub mod events { #[derive(Event)] pub struct InputEvent { /* raw */ } }

pub struct InputConfig { pub binding_file: PathBuf, pub enable_hot_reload: bool }
```

**File layout:**
```
hyge-input/src/
  lib.rs
  action.rs
  binding.rs
  translate.rs
  hot_reload.rs
  prelude.rs
```

**Algorithm — translation:**
- Each frame, `translate` system reads `EventReader<InputEvent>` + `EventReader<DeviceEvent>`, updates `ActionMap.actions` in place.
- Bindings iterate: for each binding, look up the current device state, apply scale/curve, aggregate per-action.
- `just_pressed` / `just_released` derived from edge detection on `state`.

**TOML schema:**
```toml
[actions.move]
type = "vec2"
bindings = [
  { kind = "keyboard", key = "w", axis = "y", scale = 1.0 },
  { kind = "keyboard", key = "s", axis = "y", scale = -1.0 },
  { kind = "keyboard", key = "a", axis = "x", scale = -1.0 },
  { kind = "keyboard", key = "d", axis = "x", scale = 1.0 },
  { kind = "gamepad_axis", id = 0, axis = 0 },
]
```

**Tests required:** `binding.rs` (TOML schema parse), `translate.rs` (edge detection, scale aggregation).

### 6.11 `hyge-script`

**Purpose:** Lua runtime, `bevy_reflect`-driven bindings, hot-reload, sandbox.

**Public surface (sketch):**
```rust
pub struct ScriptEngine { lua: mlua::Lua }
pub struct ScriptState { scripts: HashMap<Entity, mlua::AnyUserData> }
pub mod components {
    #[derive(Component, Reflect, Default)] pub struct ScriptRef { pub path: String, pub table: String, pub enabled: bool }
}
pub mod events { #[derive(Event)] pub struct ScriptError { pub entity: Entity, pub error: HygeError } }

pub trait ScriptApiBuilder { fn register(&self, lua: &mlua::Lua, world_ptr: *mut World) -> HygeResult<()>; }
```

**File layout:**
```
hyge-script/src/
  lib.rs
  engine.rs
  api.rs
  reflect_bind.rs
  hot_reload.rs
  sandbox.rs
  prelude.rs
```

**Sandbox policy:**
- Strip `os`, `io`, `debug`, `package`, `require` raw.
- Provide `hyge.log(level, msg)`, `hyge.world.entity.new()`, `hyge.world.entity.destroy(e)`, `hyge.world.get(e, ComponentName)`, `hyge.world.set(e, ComponentName, table)`, `hyge.event.send(EventName, table)`, `hyge.input.action(name)`, `hyge.audio.play(handle)`, `hyge.time.delta()`.
- Component access is generated at compile time via a procedural macro `#[derive(ScriptApi)]` reading `Reflect` metadata, writing a `mlua::Table` factory.

**Hot-reload:**
- `notify` watches `assets/scripts/`.
- On change, re-parse; if parse OK, replace `ScriptState` for affected entity, calling `on_reload(old_state)`.
- If parse fails, push `ScriptError` event; keep old script running.

**Tests required:** `sandbox.rs` (forbidden stdlib actually stripped), `engine.rs` (Lua round-trip), `reflect_bind.rs` (component get/set).

### 6.12 `hyge-app`

**Purpose:** assemble all plugins, run the engine.

**Public surface (sketch):**
```rust
pub struct App { /* wraps bevy App + window + render thread handle */ }
pub struct AppConfig { pub window: WindowConfig, pub renderer: RendererConfig, pub assets: AssetConfig, pub physics: Option<PhysicsConfig>, pub audio: Option<AudioConfig>, pub input: InputConfig, pub script: Option<ScriptConfig>, pub editor: bool }

pub trait AppBuilder { fn new(config: AppConfig) -> Self; fn add_plugin<P: HygePlugin>(self, p: P) -> Self; fn run(self) -> !; }

pub mod default_plugins { pub fn set() -> Vec<Box<dyn HygePlugin>> { /* all default plugins */ } }
```

**File layout:**
```
hyge-app/src/
  lib.rs
  app.rs
  config.rs
  builder.rs
  default_plugins.rs
  prelude.rs
```

**Run loop:**
1. `winit` event loop starts.
2. `First`, `PreUpdate`, `FixedUpdate` (N×), `Update`, `RenderExtract` run on main.
3. Send `FrameSnapshot` to render thread (channel: `std::sync::mpsc::SyncSender<FrameSnapshot>` with back-pressure).
4. `Last` runs (tracy flush, async task poll).
5. On `WindowCloseRequested`, drain queues, drop window, exit.

**Tests required:** `default_plugins.rs` (all plugins load without conflict).

### 6.13 `hyge-editor`

**Purpose:** Rust editor service consumed by the external PySide6/QML editor.

**Public service surface (sketch):**
```rust
pub struct EditorServer;
pub struct EditorServerConfig { pub bind_address: String, pub session_token: String }
pub struct EditorState { pub project: Option<String>, pub scene: Option<String> }
```

R-080 implements the control boundary as length-prefixed, big-endian JSON
over IPv4 loopback TCP. Every connection must begin with an authenticated
`hello` envelope; requests received before that handshake are rejected. The
wire envelope and message names are owned by `protocol/editor.schema.json` and
shared by `hyge-editor-protocol` and the optional PySide6/QML client. R-080
stores only service session metadata; ECS snapshots, reflected values and
editor commands are added by R-081 through R-084.

**File layout:**
```
hyge-editor/src/
  lib.rs
  server.rs
  state.rs
  commands.rs
  snapshots.rs
  viewport.rs
  hot_reload.rs
  prelude.rs
```

**Tests required:** `undo_redo.rs` (apply/revert pair), `inspector.rs` (reflect-driven field iteration).

### 6.14 `hyge-tools`

**Purpose:** CLI for asset cook, headless render, inspection, server.

**Public surface (sketch):**
```rust
use clap::{Parser, Subcommand};
#[derive(Parser)]
pub struct Cli { #[command(subcommand)] pub cmd: Cmd }
#[derive(Subcommand)]
pub enum Cmd {
    Import { path: PathBuf, out: PathBuf },
    Cook { project: PathBuf, out: PathBuf },
    Headless { scene: PathBuf, camera: CameraSpec, output: PathBuf, width: u32, height: u32, samples: u32 },
    Inspect { hash: String },
    Serve { project: PathBuf, port: u16 },
    Doctor { project: PathBuf },
}
```

**File layout:**
```
hyge-tools/src/
  lib.rs
  main.rs
  cmd/
    mod.rs
    import.rs
    cook.rs
    headless.rs
    inspect.rs
    serve.rs
    doctor.rs
  prelude.rs
```

**Tests required:** `cmd/import.rs` (golden output for fixed glTF), `cmd/doctor.rs` (detects known-bad project).

### 6.15 `hyge-runtime-test`

**Purpose:** shared test utilities for the renderer (headless `wgpu` without a surface).

**Public surface (sketch):**
```rust
pub struct TestRenderer { renderer: Renderer, no_surface: bool }
pub struct ReferenceImage { pub width: u32, pub height: u32, pub bytes: Vec<u8> }

pub fn create_test_renderer() -> TestRenderer;
pub fn capture_frame(renderer: &TestRenderer) -> ReferenceImage;
pub fn assert_image_matches(actual: &ReferenceImage, expected_path: &Path, tolerance: f32);
pub fn hash_image(img: &ReferenceImage) -> [u8; 32];
```

**File layout:**
```
hyge-runtime-test/src/
  lib.rs
  harness.rs
  capture.rs
  compare.rs
```

### 6.16 `examples/hyge-sandbox`

Built in M7. Demonstrates every system: PBR scene with IBL, cascade shadows, post-process, animated character, physics, Lua AI, audio bus, input bindings, editor-driven.

---

## 7. Runtime: Schedules, Threading, FrameSnapshot

### 7.1 Schedules (in execution order)

1. `First` — input edge ingestion, asset hot-reload notifications, time delta compute.
2. `PreUpdate` — fixed-timestep accumulator bookkeeping.
3. `FixedUpdate` (N× per frame as needed, default 60 Hz, max substeps 5) — physics step, deterministic gameplay logic.
4. `Update` — AI, animation, scripting, scene updates, script-driven logic.
5. `RenderExtract` — produce `FrameSnapshot` from World; lock-free via dirty archetype snapshot.
6. `Render` (render thread) — consumes `FrameSnapshot`, builds render graph, records commands, presents.
7. `Last` — frame present ack, profiler flush, async asset upload tail.

### 7.2 Threading model

- **Main thread** — owns `winit::EventLoop`, `Window`, `App`; runs `First`/`PreUpdate`/`Update`/`Last`.
- **AsyncComputeTaskPool** (`bevy_tasks`) — asset decode, meshlet bake, GPU upload, KTX2 transcode.
- **IoTaskPool** (`bevy_tasks`) — filesystem read for source assets, hot-reload watcher.
- **Render thread** (owned, long-lived) — owns `wgpu::Device`/`Queue`; consumes `FrameSnapshot`; produces presentable frame.
- **Worker threads** (default = logical cores − 2) — `par_iter_mut` on systems, physics island build, audio mix.

### 7.3 FrameSnapshot

Immutable `Arc<FrameSnapshot>` per frame:
- `frame_index: u64`
- `view: ViewData` (view + projection + view_proj + camera pos + jitter for TAA)
- `lights: LightGrid` (offsets + counts per (cluster, tile), index list)
- `opaque: Vec<DrawCommand>` (CPU-culled static + instanced)
- `transparent: Vec<DrawCommand>` (sorted back-to-front)
- `skinned: Vec<DrawCommand>` (output of skinning compute)
- `env: EnvironmentData` (sun direction, fog, post profile)
- `profiler_labels: Vec<ProfilingLabel>`

The render thread never holds a `&mut World`; world cloning is avoided by extracting only dirty archetypes and appending to a new `FrameSnapshot` (cheap when archetypes are stable).

### 7.4 Hot-reload coordination

- A `Mutex<ReloadQueue>` shared between watcher thread and main: watcher enqueues `(path, hash_diff)`, main drains in `First` schedule.
- GPU upload tasks for new asset versions run on `AsyncComputeTaskPool`; `LoadedAsset` swap is `AtomicPtr` swap in `BindlessTable` under a brief lock.

---

## 8. Renderer Architecture

### 8.1 Bindless table layout

Single `BindlessDescriptorHeap` for the frame, partitioned by type:

| Slot range | Resource type | Cap | Notes |
|------------|---------------|-----|-------|
| `[0, 0]` | Sampler (linear clamp) | 1 | global |
| `[1, 1]` | Sampler (linear repeat) | 1 | global |
| `[2, 2]` | Sampler (nearest clamp) | 1 | global |
| `[3, 3]` | Sampler (shadow) | 1 | global |
| `[4, 4]` | Storage buffer (meshes) | up to 65536 | `mesh_id` index |
| `[5, 5]` | Storage buffer (materials) | up to 65536 | `material_id` index |
| `[6, 6]` | Storage buffer (instances) | up to 1M | `instance_id` index |
| `[7, 7]` | Storage buffer (lights) | up to 4096 | `light_id` index |
| `[8, 8]` | Storage buffer (light grid) | screen tiles × clusters | per-frame |
| `[9, 9]` | Storage buffer (meshlet visibility) | dynamic | per-frame |
| `[10, 10]` | Storage buffer (draw commands) | dynamic | per-frame |
| `[11, +inf)` | Texture2DArray (color/albedo etc.) | up to 16384 | `texture_id` index, 8 mip levels avg |
| `[+inf, +2inf)` | Texture2DArray (normal/mr/emissive) | up to 16384 | second array |
| `[+2inf, +3inf)` | TextureCube (IBL env) | up to 256 | mip chain |
| `[+3inf, +4inf)` | Texture2DArray (shadow) | up to 4096 | CSM + atlas |

### 8.2 Clustered forward pipeline

Per frame:
1. `UpdateLights` (CPU): gather light components into a CPU array.
2. `BuildLightGrid` (compute, 1 thread per tile): for each tile, iterate lights, add to cluster if AABB intersects frustum slice; write offset/count to `LightGrid`, light indices to `LightIndexList`.
3. `ShadowCSM` (render pass): render scene depth from sun POV for 4 cascades; PCF soft in main pass.
4. `ShadowAtlas` (render pass): for each shadow-casting point/spot light, render depth into a sub-rect of the atlas.
5. `Skinning` (compute, optional): for each skinned mesh, transform vertices, output to skinned VB.
6. `MeshletCull` (compute, 1 thread per meshlet): frustum + LOD selection, output `MultiDrawIndirect` count.
7. `Opaque` (render pass): vertex pulls from bindless, fragment uses PBR + light grid + IBL.
8. `Transparent` (render pass): same as opaque, sorted back-to-front, alpha blend.
9. `Tonemap` → `TAA` (history reprojection) → `SMAA` → `Bloom` (downsample/upsample) → present.

### 8.3 Post-process chain (render graph)

```
ColorBuffer ─┬─→ Tonemap (ACES) → TonemappedColor
             │
             └─→ Bloom (downsample 5 levels + upsample) → BloomBuffer
                                                                   │
TonemappedColor + BloomBuffer ─→ TAA (history) → TaaColor         │
                                                                   │
TaaColor ─→ SMAA (3 passes) ─→ FinalColor ─→ Surface present
```

### 8.4 Meshlet culling shader (WGSL pseudocode)

```wgsl
@group(0) @binding(4) var<storage, read> meshes : array<GpuMesh>;
@group(0) @binding(6) var<storage, read> instances : array<Instance>;
@group(0) @binding(0) var<uniform> view : ViewData;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid : vec3<u32>) {
  let meshlet_id = gid.x;
  if (meshlet_id >= total_meshlets) { return; }
  let mesh = meshes[mesh_id_of(meshlet_id)];
  let meshlet = meshlets[meshlet_id];
  let m = instances[instance_id];
  let aabb = meshlet.aabb * m.scale + m.position;
  if (frustum_cull(view.frustum, aabb)) {
    let lod = select_lod(view, aabb, mesh.lod_error);
    visible_meshlets.append(MeshletDraw{ mesh_id, meshlet_id, lod, instance_id });
  }
}
```

### 8.5 Asset → GPU flow

1. CPU loads `.hyge-asset` (mesh, material, texture, audio) → `Vec<u8>`.
2. BLAKE3 hash computed; entry in `AssetDb`.
3. If new: schedule `GpuUploadTask` on `AsyncComputeTaskPool`; otherwise refcount bump.
4. `GpuUploadTask` creates `wgpu::Buffer`/`Texture`, calls `BindlessTable::register` to allocate slot, returns `LoadedAsset` with `Arc<GpuResource>` and `version: u64`.
5. `RenderExtract` reads `Handle<A>` → `LoadedAsset<A>` → ID for `DrawCommand`.

### 8.6 Profiling

- All passes wrapped in `wgpu::CommandEncoder::push_debug_group` with `tracy`-compatible names.
- `wgpu::QuerySet::Timestamp` per pass; `ProfilerPanel` aggregates per frame.
- FrameStats resource populated each frame: total GPU time, per-pass time, draw call count, instance count, memory.

---

## 9. Asset Pipeline

### 9.1 Source-of-truth formats

- **Models**: glTF 2.0 (`.gltf`/`.glb`) with extensions: `KHR_mesh_quantization`, `KHR_texture_basisu`, `KHR_lights_punctual`, `KHR_materials_specular`, `EXT_mesh_gpu_instancing`. Hyge-specific data in glTF `extras` (meshlet config, prefab refs, light params).
- **Textures**: KTX2 with BasisU (`BC7` for desktop, `ASTC 4×4` for mobile-class). Pipeline: PNG/JPG/EXR → `toktx` → KTX2.
- **Audio**: OGG/Opus (decoded at runtime to f32 PCM by `kira`).
- **Materials**: glTF metallic-roughness PBR; optional `KHR_materials_emissive_strength`, `KHR_materials_ior`, `KHR_materials_volume`.
- **Scripts**: Lua (`.lua`).
- **Bindings**: TOML (`assets/input.bind.toml`).
- **Scenes**: `.hyge-world` (binary, msgpack body + RON metadata header).
- **Prefabs**: `.hyge-prefab` (binary, msgpack).

### 9.2 Importer behavior (`hyge-tools import`)

Input: `<source>.gltf` + optional sidecar `<source>.hyge-meta.json`.

Steps:
1. Parse glTF with `gltf` crate.
2. For each primitive:
   - Decode mesh data; quantize to `i16`/`u16` if not already.
   - Build vertex/index buffers.
   - `meshopt_buildMeshlets` (target 64 verts, 126 tri cap).
   - Compute meshlet bounds (cone + AABB).
   - Generate LOD chain (`meshopt_simplify` ratios 0.5, 0.25, 0.1).
3. For each material:
   - Resolve texture references → upload KTX2 (or transcode PNG → KTX2 via `basis-universal`).
   - Build `GpuMaterial` constant block.
4. Write `.hyge-asset` files:
   - `<blake3>.hyge-mesh` — meshlets, indices, LOD chain, skinning data, AABBs.
   - `<blake3>.hyge-mat` — material constants + texture refs.
   - `<blake3>.ktx2` — uploaded textures.
   - `<blake3>.hyge-meta.json` — name, source path, dependencies, meshlet config.
5. Update `.hyge.db` (SQLite):
   - Insert hash → path.
   - Insert dependencies: `<mesh-hash> → <material-hash>`, `<mesh-hash> → <texture-hash>`, `<material-hash> → <texture-hash>`.

### 9.3 Compression

- LZ4 for small binary blobs (materials, audio, small meshes).
- Zstd for large geometry (level 11, default).
- KTX2 textures already compressed.

### 9.4 Runtime hot-reload

1. `notify` watcher watches `assets/source/` and `assets/scripts/`.
2. Raw FS event → resolve to `AssetId` via `AssetDb` lookup.
3. Re-import on `AsyncComputeTaskPool`.
4. New `AssetEvent { id, version: old+1 }` pushed to `EventWriter`.
5. Render extract reads latest `LoadedAsset<A>`; on version mismatch, increments old version's refcount decrement, atomically swaps `BindlessTable` slot.
6. Re-import cascades: if a mesh's material changes, all scenes using the mesh re-emit `DrawCommand`s with new `material_id`.

### 9.5 Loading API

```rust
let handle: Handle<Mesh> = server.load("meshes/cube.hyge-mesh".into());
match server.get(handle) {
    Some(loaded) => { /* use loaded.asset */ }
    None => { /* Loading */ }
}
```

`Handle<A>` is `Copy` (it is just an `AssetId`); refcount bump is implicit in `get`.

---

## 10. Scene, Prefab, Instancing

### 10.1 Prefab

`.hyge-prefab` is a msgpack-encoded tree:
```
struct Prefab {
    name: String,
    root: PrefabNode,
    assets: PrefabAssets,
}

struct PrefabNode {
    name: String,
    components: Vec<ComponentOverride>,
    children: Vec<PrefabNode>,
    persist: bool,
}

struct ComponentOverride {
    type_name: &'static str,   // looked up in TypeRegistry
    value: Box<dyn Reflect>,   // partial: only overridden fields set
}

struct PrefabAssets {
    meshes: Vec<AssetId>,
    materials: Vec<AssetId>,
    scripts: Vec<AssetId>,
}
```

`Prefab::instantiate(world, transform, parent)` returns root `Entity` (children derived). Already-instantiated prefabs share the same `Mesh`/`Material` asset handles, which become the basis of instancing.

### 10.2 Scene (`.hyge-world`)

```rust
struct World {
    env: Environment,
    root_prefab_instances: Vec<PrefabInstance>,
    post_process: PostProcessProfile,
}

struct PrefabInstance {
    prefab: PrefabId,
    transform: Transform,
    parent: Option<InstanceId>,
    overrides: Vec<ComponentOverride>,
}

struct Environment {
    skybox: Option<AssetId>,
    sun: Option<DirectionalLight>,
    fog: Option<FogParams>,
    ambient: AmbientParams,
}
```

### 10.3 Instancing extraction (RenderExtract)

1. Query `With<StaticMesh>`.
2. For each entity, look up `Mesh` and `Material` handles → resolve to `mesh_id` and `material_id` from bindless.
3. Compute AABB from `GlobalTransform * mesh.aabb`.
4. Group by `(mesh_id, material_id)`.
5. Sort groups by `material_id` (reduces state changes in forward pass).
6. Emit one `DrawCommand { mesh_id, material_id, instance_count = N, ... }` per group.

For `DynamicMesh` and `SkinnedMesh`: same grouping, but tagged for GPU cull / skinning.

### 10.4 Component catalog (canonical)

```rust
Transform, GlobalTransform, Parent, Children, Name, PersistOnReload,
StaticMesh, DynamicMesh, SkinnedMesh,
PointLight, SpotLight, DirectionalLight, AmbientLight,
Camera, EditorCamera, AudioSource, AudioListener,
ScriptRef, RigidBody, Collider, CharacterController, Joint,
PostProcessVolume, FogVolume, Decal, ParticleEmitter, // particle/decal v0.2
```

All implement `Component + Reflect + Default`. All non-trivial ones implement `From<&gltf::...>` for the importer.

---

## 11. Cross-Cutting Subsystems

### 11.1 Time

A single `Time<Update>` resource: `delta_seconds`, `elapsed_seconds`, `frame_count`. `Time<Fixed>`: `timestep`, `accumulator`. Both `bevy`-style.

### 11.2 Determinism

Fixed-timestep physics + integer-hash stable iteration over ECS archetypes = reproducible game logic per frame at given input. v0.1 does not promise full determinism for scripts (Lua floats + map iteration order), but physics is deterministic.

### 11.3 Profiler integration

- `tracing` spans with `tracy` feature flag for CPU.
- `wgpu::QuerySet::Timestamp` + `ProfilerPanel` for GPU.
- Stats resource consumed by `Last` schedule, pushed to `tracy`.

### 11.4 Error policy

- `HygeError` is `thiserror`-derived enum covering all subsystems.
- Asset load failures: `LoadedAsset::Failed(err)`, surfaced as `AssetEvent::Failed`.
- Script errors: `ScriptError` event, error registered in `ScriptEngine` registry, prev state preserved.
- Physics: assertion failures → log + disable offending body (no crash).

---

## 12. Editor (`hyge-editor` + PySide6 frontend)

### 12.1 Layout

- Qt Quick/QML layout with default layout (savable):
  - Left: Hierarchy, ContentBrowser.
  - Right: Inspector.
  - Bottom: Console, Profiler, AssetGraph.
  - Center: Viewport.

### 12.2 Viewport modes

- `Game` — runtime camera, full render graph output.
- `Editor` — editor camera, full render graph output, gizmos.
- `Preview Asset` — single asset displayed against neutral env, used for thumbnails.

### 12.3 Selection

- `Selection` resource: `Vec<Entity>`.
- Click in viewport → raycast → `Selection::select`.
- Multi-select with shift.
- Frame outline: stencil mask pass + outline shader.

### 12.4 Inspector

For each `Component` on selected entity:
- Lookup `Reflect` registration.
- For each field, generate editor:
  - `f32`, `i32`, `u32`, `bool` → slider/drag/check.
  - `Vec3`, `Quat`, `Mat4` → multi-slider with gizmo.
  - `String` → text edit.
  - `Handle<A>` → drag-drop thumbnail + open-in-browser button.
  - nested `Reflect` → collapsible group.
- Modifications emit `Command::EditComponent` for undo/redo.

### 12.5 Content browser

- File tree of `assets/`.
- Thumbnails generated by a `PreviewRenderJob` (headless renderer captures 256×256 of asset on a neutral env).
- Double-click = select; double-click on `.hyge-world` = open scene; on `.hyge-prefab` = instantiate; on `.hyge-mesh` = preview.

### 12.6 Hot-reload of scene

- Watch `.hyge-world` file.
- On change, diff old vs new: identify added / removed / changed `PrefabInstance`s.
- Re-instantiate changed ones; for each new instance, copy `PersistOnReload` components from old (if any).

### 12.7 Undo/redo

- `CommandHistory { undo: Vec<Box<dyn Command>>, redo: Vec<...> }`.
- Commands: `EditComponent`, `Reparent`, `Instantiate`, `Destroy`, `Duplicate`, `AddComponent`, `RemoveComponent`.
- Ctrl+Z pops `undo`, applies `Command::revert`, pushes to `redo`.
- Ctrl+Y opposite.

### 12.8 Profiler panel

- Live graph of frame time, GPU time, draw calls, instance count, memory.
- Sparkline rendered by the Qt frontend from profiler samples.

### 12.9 Console

- Ring buffer of last 1000 log lines.
- Filter by level (`tracing::Level`), by target (crate).
- Click a line to navigate to source.

### 12.10 Theme

- Dark by default, light toggle.
- Uses a QML theme aligned with Hyge brand.

---

## 13. Tools (`hyge-tools`)

See Section 6.14 for the per-crate specification. Invocation examples for the README / mdbook:

```bash
# Cook a single asset
hyge-tools import assets/source/cube.gltf --out assets/cook/

# Cook a full project
hyge-tools cook path/to/project/

# Headless render
hyge-tools headless --scene assets/cook/sandbox.hyge-world --camera "perspective fov=60 pos=0,5,-10 look=0,1,0" --width 1920 --height 1080 --samples 1 --out render.png

# Inspect an asset
hyge-tools inspect abcd1234...ef

# Serve over network
hyge-tools serve path/to/project/ --port 7777

# Diagnose project
hyge-tools doctor path/to/project/
```

`hyge-tools doctor` checks:
- All asset references resolve to existing hashes.
- No orphan cache files.
- DB schema version matches.
- All required plugins present.
- `unsafe` audit: count of unsafe blocks per crate (warn if growing).

---

## 14. Coding Standards & Unsafe Policy

### 14.1 Style

- `rustfmt` with default config + `imports_granularity = "Crate"` + `group_imports = "StdExternalCrate"`.
- `clippy::all`, `clippy::pedantic` (with allowed: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`), `clippy::nursery` (selectively).
- No `unsafe` in any crate except `hyge-render` and `hyge-render-graph`. Inside those, every `unsafe` block has a `// SAFETY: <rationale>` comment on the preceding line(s).

### 14.2 Documentation

- Every public item has rustdoc.
- Crate-level docs in `lib.rs` include: purpose, example, feature flags, quick start.
- Module-level docs explain why the module exists, not what.
- Examples in rustdoc are run as doctests (`cargo test --doc`).

### 14.3 Error handling

- All fallible functions return `HygeResult<T>`.
- No `unwrap` in library code (`clippy::unwrap_used = "deny"`).
- `expect` allowed only when the invariant is provable from context; each `expect` has a comment.
- No `panic!` in library code; only in `#[cfg(test)]` and examples.

### 14.4 Logging

- `tracing` only, with structured fields: `tracing::info!(entity = ?e, "loaded mesh")`.
- `tracing::Level::INFO` is the default; `DEBUG` enabled via `HYGE_LOG=debug` env or `RUST_LOG=hyge=debug`.
- No `println!` / `eprintln!` in library code.

### 14.5 Unsafe policy (binding)

- `hyge-render` and `hyge-render-graph`: unsafe allowed for:
  - `bytemuck::Pod`/`Zeroable` transmutation between CPU/GPU structs.
  - FFI shims to `wgpu` raw APIs.
  - Performance-critical hot loops with provable safety.
- All other crates: `unsafe_code = "forbid"` via `Cargo.toml` lints.

---

## 15. Versioning, Compatibility, Release Channels

### 15.1 Semver

- v0.1.x: pre-1.0; any minor release may break public API.
- v0.2.x: stabilize public API.
- v1.0.x: semver-strict; minor must not break public API.

### 15.2 Workspace version

- All crates share `workspace.package.version = "0.1.0"`.
- Internal crate versions are synchronized; cross-crate compatibility is guaranteed by lockstep versioning.

### 15.3 Release channels

- `main`: stable, tagged.
- `next`: integration branch; v0.2 work-in-progress.
- `feature/*`: feature branches off `main`.
- Tags: `v0.1.0-mN` per milestone; `v0.1.0` for first release.

### 15.4 Changelog

- `CHANGELOG.md` auto-generated by `git-cliff` from conventional commits.
- Required commit types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `breaking`.

### 15.5 MSRV

- Minimum supported Rust version: 1.80 (locked via `rust-toolchain.toml` and `Cargo.toml` `rust-version` field).
- CI tests against MSRV + stable.

---

## 16. Risks & Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|--------|-----------|------------|
| `wgpu` mesh shader / advanced features unavailable | Cannot use bleeding-edge GPU features | Medium | Design abstraction so mesh shader backend is swappable when `wgpu` adds it; v0.1 uses compute meshlet culling |
| `rapier3d` build time | Slows CI | High | Feature flag; pre-built artifact in CI cache; consider `rapier` 0.20+ parallel improvements |
| BLAKE3 import performance on large projects | Slow iteration | Medium | Incremental import: only re-bake changed glTFs; parallel bake on worker pool |
| External Qt editor + render service sync | Stale state in inspector | Medium | Backend publishes versioned snapshots from the same engine state used by render |
| Lua FFI overhead | Hot path regression | Medium | Cache reflected lookups in Lua registry; avoid `Reflect::reflect_*` per-call; expose typed accessors in scripts |
| Raw input on Linux | Loses high-precision mouse | High | Use `evdev` directly behind same `RawInput` trait; design abstraction so backend is swappable |
| Clustered forward memory for light grid | GPU memory pressure | Low | Z clusters × XY tiles sized to scene scale; configurable; default 16×16×16 keeps it small |
| Hot-reload state loss | Bad DX | High | `PersistOnReload` component + script-side `on_reload(old_state)` hook + state diff in scene reload |
| SQLite contention in tests | Flaky CI | Medium | `#[serial_test::serial]` on DB-touching tests; or migrate to `redb` (single-writer MVCC) |
| Editor docking layout persistence | Annoying UX | Low | Serialize the QML layout model to JSON, save in `user_data/editor_layout.json` |
| WGSL portability across `wgpu` backends | Shader compile errors per backend | Medium | Shader validation per backend in CI matrix; naga validator run in pre-CI hook |
| `kira` HRTF dataset licensing | Cannot ship HRTF | Low | Bundle KEMAR-derived (public domain) or skip HRTF (feature default off) |
| Profiling overhead in dev | Slows iteration | Low | Tracy gated by feature flag; off in release |
| Asset DB corruption on crash | Lost work | Low | WAL mode on SQLite; incremental checkpoint |
| Editor viewport tearing vs game window | Bad DX | Medium | Editor runs the same render graph; share frame number with game window when in play mode |

---

## 17. Out of Scope for v0.1 (Explicit)

The following are **explicitly not** in v0.1. They are deferred to v0.2+ unless a new ADR promotes them.

- **Networking**: `hyge-net` crate (v0.2: `quinn`/QUIC transport + ECS replication + snapshot interpolation + client prediction).
- **WASM scripting**: `wasmtime` component model (v0.2).
- **VR/AR/XR**: future.
- **Mobile/console targets**: future (Linux/macOS build works through `wgpu` but no platform QA in v0.1).
- **Ray tracing / path tracing**: future (requires raw DX12/Vulkan; not in `wgpu` API).
- **Global illumination (SSGI / LPV / SSDO / DDGI)**: future (v0.1: IBL only).
- **Custom physics solver**: v0.1 uses `rapier3d` only; alternative solvers deferred.
- **Asset editor for non-glTF formats**: v0.1 supports glTF + KTX2 + OGG/Opus only. FBX/OBJ deferred.
- **Text rendering / localization / fonts**: v0.2.
- **Particle system / VFX graph**: v0.2.
- **Decals**: v0.2.
- **Plugin marketplace**: v0.3+ (after editor stabilizes).
- **Cloud services (cloud build, telemetry)**: v0.3+.
- **Mobile/console certification**: v0.3+ (after platform QA).

---

## 18. Open Questions for Future ADRs

The following items are intentionally **not** decided in this document. They are tracked for future ADRs.

1. **HDR display pipeline** — `Rgb10a2Unorm` / scRGB swapchain formats and ACEScg scene-referred vs sRGB display-referred pipeline boundary. (v0.2)
2. **DX12 Agility SDK runtime** — should we ship the Agility SDK to allow newer DX12 features than the OS-default? `wgpu`'s built-in DX12 backend. (v0.2)
3. **HRTF dataset** — KEMAR-derived (public domain) vs `Kira` built-in vs custom measurement. (v0.2)
4. **Editor docking layout persistence format** — JSON vs `ron`. (M6, decided at impl time)
5. **Script compile-time macros** — whether to ship a procedural macro `#[derive(ScriptApi)]` to generate Lua bindings at compile time, or use a runtime codegen. (M5)
6. **Editor command palette** — quick-action bar inspired by VSCode/Rider. (v0.2)
7. **Multi-window support** — secondary viewports for editor / debug. (v0.2)
8. **Reverse-Z** — for improved depth precision in large view frustums. (v0.1 or v0.2; ADR pending)
9. **Bindless texturing tier** — full bindless (16k textures) vs tiered (3-4 categories). (M3, decided at impl time)
10. **Meshlet size policy** — 64 verts/126 tri (UE-style) vs 128/256 (Nanite-style). (M2)
11. **Default platform priority** — Windows Vulkan vs Windows DX12 as default backend. (M1, decided by benchmark)
12. **Documentation hosting** — GitHub Pages vs custom domain. (M7)
13. **Asset format versioning** — header format for `.hyge-asset` that survives breaking changes. (M2)
14. **Profiling default** — Tracy vs Superluminal vs Perfetto for the runtime default. (M6)
15. **Long-term**: a `hyge-bevy` bridge crate to allow Bevy plugins to be reused? (v0.3+)

---

*End of architecture. Implementation is tracked in `docs/roadmap.toml`. Agent-facing instructions are in `AGENTS.md`.*
