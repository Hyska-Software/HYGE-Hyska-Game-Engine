# ADR-0013: Full Visual Editor in v0.1

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

A 3D engine without an editor is not a tool — it is a library. The editor is the surface through which designers, artists, and developers interact with the engine day-to-day. The v0.1 feature checklist (`docs/architecture.md` §15.3) requires:

- A viewport running the same render graph as the runtime, with editor camera and gizmos.
- A hierarchy panel (tree view, drag-drop reparent).
- An inspector panel (reflect-driven component editor).
- A content browser (file tree + thumbnails).
- A console (tracing ring buffer).
- A profiler (frame stats sparkline).
- Undo/redo across all editor actions.
- Hot-reload of scenes with state preservation (`PersistOnReload`).

## Decision

Ship a **full visual editor** in v0.1 as a first-class crate `hyge-editor`, built on `egui` + `egui_dock` + `egui_tiles`, registered as a `HygePlugin` in `hyge-app`'s default plugin set. The editor runs in the same process as the runtime, reading the same ECS world.

## Consequences

### Positive

- **Designer-facing tool from day one:** no "build your own editor" tax for users.
- **Single source of truth:** the editor and the runtime share the same `Component` definitions and the same `Reflect` registry; the inspector cannot drift from the runtime types.
- **Fast iteration:** the editor is itself a Hyge plugin; contributors can extend it with their own panels in the same way they extend the engine.
- **Hot-reload with `PersistOnReload`:** the editor preserves state across asset reloads, which is the daily workflow of every developer using the engine.

### Negative

- **More work in v0.1:** the editor adds 1-2 milestones' worth of work to the schedule.
- **`egui` is not the prettiest UI toolkit:** the visual fidelity is "developer tool", not "AAA game UI". Acceptable for a v0.1 editor; the user-facing game UI uses different systems.
- **Single-process coupling:** the editor must run in the same process as the runtime, which means a `wgpu::Surface` and the renderer must coexist with `egui`. This is well-understood territory (`egui-wgpu` integration).

## Alternatives Considered

### Runtime + CLI only in v0.1; editor in v0.2

- **Pros:** Smaller v0.1 scope; faster release.
- **Cons:** "Engine without an editor" is a hard sell; the user must write their own editor or use the runtime directly, which is a poor developer experience.
- **Rejected because:** the editor is part of the v0.1 promise of a "complete" engine.

### Editor only on top of a webview (Tauri-style)

- **Pros:** Polished UI possible; clean frontend/backend split.
- **Cons:** Network/IPC overhead between editor UI and engine; not what users expect from a desktop engine; extra build complexity.
- **Rejected for v0.1:** consider for v0.3+ if there is demand for a web-based editor.

### `imgui` instead of `egui`

- **Pros:** Mature; many examples.
- **Cons:** Older API; less Rust-idiomatic; not as good for our needs.
- **Rejected because:** `egui` is the modern Rust choice and integrates well with `wgpu`.

## References

- `docs/architecture.md` §6.13 (hyge-editor), §12 (editor), §15.3 (feature checklist)
- `egui` documentation: <https://docs.rs/egui>
- `egui_dock` and `egui_tiles` for docking layout
- ADR-0002 (bevy_ecs), ADR-0007 (prefab) — the editor reuses the same reflection and override machinery
