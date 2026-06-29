# ADR-0001: wgpu as the Graphics API

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Hyge targets modern 3D rendering on PC Windows primary, with future support for macOS and Linux. The graphics API layer must:

- Provide first-class DX12, Vulkan, Metal, and (optionally) WebGPU backends without the engine maintaining each one in parallel.
- Be productive for the development of a bindless, render-graph-based renderer (see ADR-0003).
- Avoid pulling the engine into conflict with raw native API churn (DX12 Agility SDK, Vulkan minor versions).
- Not require a C/C++ toolchain on the build host.

## Decision

Adopt **`wgpu` 0.20+** as the graphics HAL. It satisfies the cross-platform requirement out of the box, exposes a stable Rust API, and is the only Rust-native option that covers DX12, Vulkan, Metal, and WebGPU behind a single trait surface.

## Consequences

### Positive

- Single API surface across Windows (DX12), Linux (Vulkan), macOS (Metal), and web (WebGPU). No per-platform fork of the render crate.
- No C/C++ build dependency; `cargo build` works on every platform.
- Active ecosystem (`wgpu` is used by Bevy, Veloren, etc.) — continued maintenance and shader-translator work via `naga`.
- Built-in `naga` shader translator; WGSL is portable across all backends.

### Negative

- `wgpu` lags behind native APIs in bleeding-edge features (mesh shaders, advanced ray tracing queries). Novel DX12 features may require waiting for upstream support.
- The engine is constrained to the feature set `wgpu` exposes; when `wgpu` lacks a feature, we either polyfill it or defer it (see `docs/architecture.md` §17 out-of-scope list).
- A thin runtime cost per call (mitigated by bindless indexing in ADR-0003, so per-draw work is dominated by GPU work).

## Alternatives Considered

### `ash` (Vulkan) + raw DX12 behind a trait `RHI`

- **Pros:** Full control over both APIs; direct access to Vulkan and DX12 features; no abstraction penalty.
- **Cons:** ~2× the render code; two separate backend paths to test and maintain; no Metal/WebGPU without a third backend. Doubles the work of every render feature.
- **Rejected because:** the productivity cost and ongoing maintenance burden exceeds the feature ceiling we actually need in v0.1.

### Raw DX12 only (Windows-first)

- **Pros:** Minimum abstraction; full feature access on the primary target.
- **Cons:** Locks the engine to Windows; no Linux/macOS support; high maintenance cost; reinventing every primitive (swapchain, barriers, descriptor heaps) that `wgpu` already provides.
- **Rejected because:** contradicts the cross-platform target and the "engine for everyone" principle.

### Custom abstraction layer over multiple backends

- **Pros:** Tailored to our exact needs.
- **Cons:** Reinvents what `wgpu` already provides; years of work to reach parity on every backend; ongoing maintenance.
- **Rejected because:** cost ≫ benefit. `wgpu` already covers the surface we need and is actively maintained.

## References

- `docs/architecture.md` §4 (closed decisions) and §6.4 (hyge-render)
- `wgpu` repository: <https://github.com/gfx-rs/wgpu>
- `naga` (shader translator): <https://github.com/gfx-rs/wgpu/tree/trunk/naga>
