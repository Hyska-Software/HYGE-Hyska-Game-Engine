# ADR-0008: Lua via mlua + bevy_reflect Bindings

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Game-side logic must be hot-reloadable during development. Compiling-and-restarting Rust is too slow for iteration. The engine needs a scripting layer that:

- Is sandboxed (no `os.execute`, no arbitrary file I/O).
- Can read and write ECS components without losing the Rust-side type system.
- Hot-reloads on file change.
- Has predictable performance (no GC pauses in the hot path).

## Decision

Adopt **Lua via `mlua`** as the v0.1 scripting layer, integrated with the ECS through `bevy_reflect`:

- A `ScriptEngine` wraps `mlua::Lua`, started with restricted standard library (`os`, `io`, `debug`, `package`, raw `require` are stripped).
- Components are exposed to Lua via a `hyge.world.get(e, "Transform")` / `hyge.world.set(e, "Transform", table)` API, with table↔struct conversion driven by `bevy_reflect::TypeRegistry`.
- A `ScriptRef` component holds `{ path, table, enabled }`; a system runs `on_update(dt)` per entity per frame.
- `notify` watches `assets/scripts/`; on change, re-parse; if OK, replace the script state, calling `on_reload(old_state)` if defined. If parse fails, push a `ScriptError` event and keep the old script running.

WASM via `wasmtime` is **deferred to v0.2** (see `docs/architecture.md` §17 out-of-scope).

## Consequences

### Positive

- **Fast iteration:** edit a `.lua` file, the running game picks it up in milliseconds.
- **Type-safe enough:** Reflect is the contract; type mismatches become Lua runtime errors with a clear path back to the Rust type.
- **Sandbox:** restricted stdlib + a controlled `hyge.*` API surface; untrusted scripts cannot escape.
- **No GC pauses in the hot path:** Lua scripts run once per `Update` schedule; per-frame allocation pressure is bounded.

### Negative

- Lua's `f32` is the only number type; `i64` round-trips through Lua as `f64` with precision loss. Mitigated by serializing big numbers as strings when needed.
- Map iteration order is unspecified in Lua; v0.1 does not promise full determinism for scripts (only for physics).
- Reflect-driven FFI has a per-call overhead; mitigated by caching `Reflect::ReflectRef` lookups in the Lua registry.

## Alternatives Considered

### WASM via `wasmtime` (component model)

- **Pros:** Strong sandbox; first-class typed ABI; multi-language support; no GC pauses.
- **Cons:** More plumbing to wire up; component model still settling; cold-start cost; not all engine consumers need it in v0.1.
- **Deferred to v0.2** (see `docs/architecture.md` §17).

### Pure Rust (no scripting)

- **Pros:** Zero overhead; full type safety; no FFI.
- **Cons:** Iteration speed is the bottleneck; no user-modding story.
- **Rejected because:** the whole point of a "complete" engine in v0.1 is to ship iteration tools.

### Python via `pyo3`

- **Pros:** Familiar; large ecosystem; easy to embed.
- **Cons:** GIL; heavy runtime; license ambiguity on bundled interpreter; predictable performance is hard.
- **Rejected because:** the iteration/friction trade is worse than Lua for game logic.

## References

- `docs/architecture.md` §6.11 (hyge-script), §12.2 (cross-cutting: scripting)
- `mlua` documentation: <https://docs.rs/mlua>
- `bevy_reflect` documentation: <https://docs.rs/bevy_reflect>
- ADR-0002 (bevy_ecs) — `bevy_reflect` is the bridge
