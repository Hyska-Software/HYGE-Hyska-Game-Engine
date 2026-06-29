# ADR-0007: BLAKE3-Keyed Hierarchical Prefabs and Instancing

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

A 3D engine without prefabs forces users to either duplicate entities or write runtime code to compose them. Without instancing, the renderer cannot reach 10k+ visible instances. The two features are deeply linked: a prefab's instances share asset handles, which is exactly what the renderer's instancing extraction needs.

The prefab system must:

- Be a content-addressed asset like any other (BLAKE3-keyed, in the asset DB).
- Express a tree of entities with typed component overrides.
- Allow per-instance overrides (e.g. one of 100 identical enemies has a different starting health).
- Be serializable to a binary format and round-trip cleanly.

## Decision

Adopt a **BLAKE3-keyed hierarchical prefab system**:

- `.hyge-prefab` is a msgpack-encoded tree: `Prefab { name, root: PrefabNode, assets }`.
- `PrefabNode { name, components: Vec<ComponentOverride>, children, persist }`.
- `ComponentOverride { type_name, value: Box<dyn Reflect> }` — partial override; only specified fields are set on the instance.
- `Prefab::instantiate(world, transform, parent) -> Entity` recursively builds entities.
- `PersistOnReload` component on any node survives scene hot-reload.
- The renderer groups instances by `(mesh_id, material_id)` and emits a single `DrawCommand` per group with `instance_count = N`.

## Consequences

### Positive

- **Content-addressed prefabs:** two prefabs with identical structure are the same `PrefabId`; no duplication.
- **Hierarchical:** natural mapping to scene-graph concepts (parent/child transforms).
- **Override via Reflect:** the same reflection used by Lua (ADR-0008) and the editor inspector (ADR-0013) drives the override system — no parallel type system.
- **Instancing falls out for free:** instances share the same `Handle<Mesh>` and `Handle<Material>`, which is exactly the renderer's instancing key.
- **`PersistOnReload`:** scene hot-reload preserves the bits that matter (player position, undo state) and re-creates the rest.

### Negative

- Reflect-based overrides are powerful but not type-safe at the call site: a typo in `type_name` becomes a runtime error, not a compile-time error. We mitigate with `TypeRegistry` lookups and unit tests.
- Deeply nested prefabs (>10 levels) hit recursion limits; we cap nesting at 16 with a clear error.
- Asset references in prefabs are stored by `AssetId`; a missing asset is a `LoadedAsset::Failed` at runtime, not at import time.

## Alternatives Considered

### Text-based prefabs (RON, JSON, YAML)

- **Pros:** Human-readable; easy to diff.
- **Cons:** Slower to parse; larger on disk; not binary-portable.
- **Rejected because:** the message size adds up at scene scale; msgpack with a small RON metadata header is the better trade.

### Prefab without overrides (copy-only)

- **Pros:** Simpler.
- **Cons:** Every variation needs a separate prefab; no "5 of these plus 1 with a different weapon".
- **Rejected because:** overrides are a daily need; without them, users reinvent the feature per project.

### Scene without instancing

- **Pros:** Trivial to implement.
- **Cons:** Does not reach 10k+ instances; perf target fails.
- **Rejected because:** instancing is non-optional for the scale we target.

## References

- `docs/architecture.md` §6.6 (hyge-scene), §10 (scene, prefab, instancing)
- ADR-0002 (bevy_ecs) — `bevy_reflect` is the override mechanism
- ADR-0008 (scripting) — same reflection powers Lua bindings
- ADR-0013 (editor) — inspector reuses the same override machinery
