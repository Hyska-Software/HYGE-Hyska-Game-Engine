# R-085 — Hierarchy, Selection and Persistent Scene Editing

**Intent:** Make the editor backend authoritative for hierarchy edits, selection, stable scene identities and persistence.
**Current Behavior:** R-084 mutates the live ECS world, but selection is replace-only and the `.hyge-world` document is not synchronized back from edited ECS hierarchy state.
**Expected Outcome:** Reparent, duplicate, instantiate, destroy, selection and undo/redo preserve reciprocal hierarchy invariants and survive save/reopen through the versioned scene edit layer.
**Target-Perspective Output:** A real editor session can select entities with Shift, edit a loaded scene, receive fresh snapshots, save, reopen and observe the same hierarchy and stable IDs.
**Truth Owner:** `hyge-scene::WorldDocument` owns persisted scene data; `EditorSessionRuntime` owns the active ECS world, selection and transactional history.
**Contract Boundary:** Rust editor service ↔ `protocol/editor.schema.json` ↔ PySide6 client; scene persistence remains owned by `hyge-scene`.
**Cutover:** All hierarchy mutations route through invariant-checked helpers; list-only selection remains only as the default-compatible `shift = false` request.
**Displaced Path:** Direct ad-hoc `Parent`/`Children` mutation and serialization of the original, unedited `LoadedSceneState.document`.
**Value Density:** One real backend path serves snapshots, TCP editing, undo/redo and persisted scene reopen.
**Acceptance Evidence:** Focused editor/scene tests, TCP protocol tests, save/reopen fixture, legacy migration fixture, and workspace quality gates.
**Evidence Lane:** `cargo test` focused first, then Python protocol tests, serial workspace tests, clippy, fmt and docs.
**Kill Criteria:** Do not add a parallel editor scene database, frontend-owned selection truth, or a partial save path that silently drops hierarchy edits.
**Architecture Slice:** `hyge-scene` world format/runtime, `hyge-editor` lifecycle/commands/snapshots/server, protocol schema/client, ADR and roadmap evidence.
**Plan Review Gate:** Approved by the user in the execution request.

## Ordered tasks

1. Add versioned persisted scene identities and edit-layer load/save support in `hyge-scene`.
2. Centralize and harden hierarchy mutation, capture and restore in `hyge-editor`.
3. Add backend-owned Shift selection and command-driven selection updates.
4. Wire protocol/schema/server responses and Python compatibility.
5. Add real persistence, migration, TCP and invariant tests; update ADR/architecture/roadmap.
6. Run focused and workspace evidence gates; report any remaining proof gap explicitly.
