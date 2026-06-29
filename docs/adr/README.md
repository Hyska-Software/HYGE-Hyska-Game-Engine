# Architecture Decision Records (ADRs)

This directory contains the Architecture Decision Records for the Hyge engine. Each ADR captures a single significant architectural decision, the context that led to it, the consequences, and the alternatives that were considered.

## Process

1. When proposing a new decision, create a new file with the next available number (e.g. `0015-new-topic.md`).
2. Use the template in `TEMPLATE.md` (if present) or follow the structure of an existing ADR.
3. Set the ADR's status to **Proposed** in the PR.
4. After review and acceptance, set the status to **Accepted** and merge.

## Status taxonomy

- **Proposed** — under discussion; do not implement on top of it.
- **Accepted** — locked; deviations require a new ADR that supersedes this one.
- **Superseded by ADR-NNNN** — replaced; see the new ADR for current guidance.
- **Deprecated** — explicitly rejected; do not implement.

## Index

| # | Title | Status | Topic |
|---|-------|--------|-------|
| [0001](0001-wgpu-as-graphics-api.md) | wgpu as the graphics API | Accepted | Rendering |
| [0002](0002-bevy-ecs.md) | bevy_ecs as the ECS | Accepted | Runtime |
| [0003](0003-render-stack.md) | Bindless + Render Graph + Clustered Forward | Accepted | Rendering |
| [0004](0004-workspace-14-crates.md) | Workspace of 14 fat crates | Accepted | Build |
| [0005](0005-dedicated-render-thread.md) | Dedicated render thread + FrameSnapshot | Accepted | Runtime |
| [0006](0006-blake3-asset-pipeline.md) | BLAKE3 content-addressed asset pipeline | Accepted | Assets |
| [0007](0007-scene-prefab-system.md) | BLAKE3-keyed hierarchical prefabs | Accepted | Scene |
| [0008](0008-lua-via-mlua.md) | Lua via mlua + bevy_reflect bindings | Accepted | Scripting |
| [0009](0009-rapier3d-physics.md) | rapier3d behind feature flag | Accepted | Physics |
| [0010](0010-kira-audio.md) | kira + spatial 3D + HRTF optional | Accepted | Audio |
| [0011](0011-windowing-input.md) | winit + gilrs + raw input + TOML | Accepted | Platform |
| [0012](0012-networking-deferred.md) | Networking out of scope for v0.1 | Accepted | Scope |
| [0013](0013-full-editor-v0.1.md) | Full visual editor in v0.1 | Accepted | Editor |
| [0014](0014-stable-msrv-1.80.md) | Stable only, MSRV 1.80 | Accepted | Build |

## Conventions

- File name: `NNNN-short-kebab-title.md` (zero-padded four digits).
- One decision per ADR; do not bundle.
- Decisions are immutable once Accepted. Change them by writing a new ADR that supersedes the old one.
- Reference ADRs from PR descriptions and from `docs/architecture.md` §4 (closed decisions).
