# ADR-0002: bevy_ecs as the ECS

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

The Entity Component System is the spine of the engine: every game-side system, every renderer input, every script reads and writes through the ECS. The choice has long-term consequences for ergonomics, parallel scheduling, and the scripting bridge.

The ECS must provide:

- Archetypal storage (cache-friendly, modern).
- Parallel system scheduling with conflict detection.
- Change detection (so the renderer can extract only dirty data per frame).
- Observers and relations (modern ECS features for decoupling).
- Reflection for scripting integration (see ADR-0008).

## Decision

Adopt **`bevy_ecs` 0.14+** as the only ECS. All other crates interact with the world through `bevy_ecs::prelude::*` re-exported from `hyge-ecs`.

## Consequences

### Positive

- Archetypal storage, parallel schedule with `par_iter_mut`, `IntoSystemConfigs` (`.chain()/.before()/.after()`).
- First-class **change detection** (`Added`, `Changed`, `Mut<T>::is_changed()`) — the renderer's `RenderExtract` relies on it.
- **Observers** decouple cross-cutting reactions (asset reloaded, script errored) from the systems that produce them.
- **Relations** express typed parent/child, owned-by, attaches-to without bespoke component graphs.
- **`bevy_reflect`** provides the metadata layer that drives the Lua scripting bridge (ADR-0008) and the editor inspector (ADR-0013).
- Largest Rust ECS ecosystem; ample reference material; `bevy_tasks` integrates naturally.

### Negative

- The engine's ECS API surface is dictated by `bevy_ecs`'s release cadence (major versions every ~3-6 months historically).
- We re-export `bevy_ecs` so end users do not name it directly; renaming or restructuring inside `bevy_ecs` is absorbed inside `hyge-ecs`.
- `bevy_ecs` brings a sizeable dependency footprint (compile-time cost). Mitigated by isolating it in `hyge-ecs` and never leaking types past that crate boundary.

## Alternatives Considered

### `shipyard`

- **Pros:** Fast unique-storage ECS; no required traits on components; very low overhead.
- **Cons:** No equivalent of `bevy_ecs` relations or observers; smaller ecosystem; no `bevy_reflect` integration out of the box; API less ergonomic for our use cases.
- **Rejected because:** the missing relations/observers/reflect would force us to write a parallel metadata layer, defeating the "one ECS" principle.

### Custom ECS (Flecs-style: archetypal + sparse + relations)

- **Pros:** Full control; could be designed exactly for our needs.
- **Cons:** Adds months of foundational work before any pixel is rendered; risks re-inventing bugs already fixed in `bevy_ecs`.
- **Rejected because:** the productivity loss is incompatible with the v0.1 schedule.

### Pure-SoA libraries (`soa-rs`, `specs`)

- **Pros:** Even tighter cache behavior for some workloads.
- **Cons:** Less mature; missing modern ECS features; weaker ecosystem.
- **Rejected because:** the cache wins are not worth the ergonomic cost and the missing features.

## References

- `docs/architecture.md` §4 (closed decisions) and §6.2 (hyge-ecs)
- `bevy_ecs` documentation: <https://docs.rs/bevy_ecs>
- ADR-0008 (scripting) — depends on `bevy_reflect`
- ADR-0013 (editor) — depends on `bevy_reflect` for the inspector
