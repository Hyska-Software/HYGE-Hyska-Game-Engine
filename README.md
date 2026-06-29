# Hyge

A modern, modular 3D game engine in Rust.

> **Status:** Pre-alpha. The architecture, ADRs, and implementation roadmap are
> in place; engine subsystems are being implemented milestone by milestone.
> See [`docs/architecture.md`](docs/architecture.md) for the design contract
> and [`docs/roadmap.toml`](docs/roadmap.toml) for the implementation backlog.

## Requirements

- **Rust stable 1.80+** (no nightly; see ADR-0014). Pinned via [`rust-toolchain.toml`](rust-toolchain.toml); MSRV declared in `[workspace.package] rust-version` in [`Cargo.toml`](Cargo.toml).
- **Windows 10/11** (primary), or **macOS 12+** / **Linux x86_64** via `wgpu` backends (Metal / Vulkan / DX12).
- A **C/C++ toolchain** is required for a few dependencies that build from C source: `rusqlite` (bundled SQLite), `mlua` (vendored Lua), `meshopt` (meshoptimizer). On Windows, install **Visual Studio Build Tools** with the "Desktop development with C++" workload. On Linux, install `gcc`/`clang` and `cmake`.

## Quick links

- [`docs/architecture.md`](docs/architecture.md) — architecture specification
- [`docs/roadmap.toml`](docs/roadmap.toml) — R-XXX implementation backlog
- [`docs/adr/`](docs/adr) — Architecture Decision Records
- [`docs/ownership.md`](docs/ownership.md) — crate ownership map
- [`AGENTS.md`](AGENTS.md) — development guide for agents

## Build

The toolchain is pinned in `rust-toolchain.toml`; just run:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- --deny warnings
```

See `AGENTS.md` §3 for the full command set.

## Crates

| Crate | Purpose |
|-------|---------|
| `hyge-core` | Math, color, log, asset id, error types |
| `hyge-ecs` | `bevy_ecs` wrapper, `HygePlugin` trait, schedules |
| `hyge-render-graph` | DAG, barrier inference, transient allocator |
| `hyge-render` | `wgpu`, bindless, clustered forward, post-process |
| `hyge-asset` | BLAKE3 DB, hot-reload, GPU upload |
| `hyge-scene` | glTF loader, prefab, instancing, components |
| `hyge-physics` | `rapier3d` wrapper, fixed timestep |
| `hyge-audio` | `kira` backend, spatial 3D, HRTF (feature) |
| `hyge-window` | `winit`, surface, raw input (Windows) |
| `hyge-input` | `Action<T>`, TOML bindings, gamepad |
| `hyge-script` | Lua via `mlua`, `bevy_reflect` bindings, sandbox |
| `hyge-app` | `App` builder, default plugins |
| `hyge-editor` | `egui` panels (viewport, hierarchy, inspector, ...) |
| `hyge-tools` | CLI (`import`, `cook`, `headless`, `inspect`, `serve`, `doctor`) |
| `hyge-runtime-test` | headless `wgpu` test harness |

## License

MIT OR Apache-2.0.
