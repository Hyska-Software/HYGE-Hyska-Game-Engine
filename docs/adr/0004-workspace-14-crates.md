# ADR-0004: Workspace of 14 Fat Crates (Bevy-style)

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Hyge is a multi-subsystem engine: render, asset, scene, physics, audio, input, window, script, editor, tools, runtime-test, app, ecs, core. The repository layout must:

- Keep compile times reasonable (parallel compilation across crates).
- Make subsystem boundaries explicit and enforceable.
- Allow one crate to be tested in isolation.
- Be discoverable for new contributors (each crate has a clear purpose).

## Decision

Adopt a **workspace of 14 fat crates**, each with a single primary responsibility, a `prelude` module, and a `HygePlugin` impl (except leaf utility crates). The full crate map lives in `docs/architecture.md` §5.2.

## Consequences

### Positive

- **Build parallelism:** `cargo` compiles independent crates concurrently, cutting wall-clock time vs a monolith.
- **Test isolation:** every crate is `cargo test -p <name>` runnable.
- **Enforced boundaries:** "no leaking types" is mechanical — types not re-exported in `prelude` are not part of the public API.
- **Discoverable:** a new contributor can navigate the repo by crate name and find the relevant subsystem in seconds.
- **Publishable artifacts:** each crate is independently `cargo publish`-able (used internally; not all are public).

### Negative

- More boilerplate per crate (`Cargo.toml`, `lib.rs`, `prelude.rs`).
- Refactors that cross crate boundaries are slightly more expensive (touches more `Cargo.toml`s).
- A few inter-crate trait objects (`Box<dyn HygePlugin>`) are necessary in `hyge-app/default_plugins.rs`.

## Alternatives Considered

### Single crate with modules

- **Pros:** Minimum boilerplate; everything is `pub`.
- **Cons:** Build times balloon as the engine grows; internal types leak; refactors are dangerous because nothing is enforced.
- **Rejected because:** the moment the engine has 50k+ lines, build times and accidental coupling become the bottleneck.

### Workspace of 5–6 god-crates (`hyge-core`, `hyge-render`, `hyge-asset`, `hyge-physics`, `hyge-audio`, `hyge-app`)

- **Pros:** Less boilerplate; faster initial setup.
- **Cons:** `hyge-render` becomes a god-crate with editor, tools, and runtime-test mixed in; `hyge-asset` accumulates editor-side concerns.
- **Rejected because:** god-crates erode the boundary principle as the engine grows.

### Workspace with a plugin registry at runtime (dynamic loading)

- **Pros:** Plugins can be added by third parties without recompiling the engine.
- **Cons:** Massive build/test complexity; ABI stability becomes a hard problem; we are not at that scale in v0.1.
- **Rejected for v0.1:** revisit in v0.3+ (see `docs/architecture.md` §17 out-of-scope).

## References

- `docs/architecture.md` §5 (workspace & crate map)
- `AGENTS.md` §7.2 (crate conventions)
