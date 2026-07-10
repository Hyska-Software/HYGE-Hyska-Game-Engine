# ADR-0015: External PySide6/QML Editor

- **Status:** Accepted
- **Date:** 2026-07-10
- **Deciders:** Hyge core team
- **Supersedes:** ADR-0013

## Context

The previous editor plan selected an in-process `egui` UI. The current
engine already owns a `winit` event loop and a `wgpu` renderer, while a
professional editor needs an independent desktop UI lifecycle. Coupling a
second GUI event loop directly to the runtime would make window ownership,
shutdown and renderer lifetime fragile.

## Decision

Use a Rust editor service and a separate Python + PySide6 + Qt Quick/QML
frontend. The service owns the ECS world, reflection registry, renderer,
assets, scene hot-reload and editor commands. The frontend communicates only
through the versioned protocol in `protocol/editor.schema.json`.

Control messages use length-prefixed JSON over loopback TCP. Viewport frames
will use a shared-memory ring buffer so the control protocol is not burdened
with image payloads. The first implementation keeps the render graph in the
Rust process and exposes frame metadata before the final GPU transport lands.

## Consequences

- Qt and `winit` have independent event loops and process lifetimes.
- The engine has one source of truth; Python cannot mutate ECS state directly.
- The editor can be developed and tested independently of the engine ABI.
- Deployment must package Python, PySide6, Qt plugins and QML resources.
- A future native/GPU zero-copy viewport transport can replace shared memory
  without changing editor commands or ECS ownership.
