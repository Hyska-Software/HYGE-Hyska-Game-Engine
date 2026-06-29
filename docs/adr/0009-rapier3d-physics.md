# ADR-0009: rapier3d Behind Feature Flag for Physics

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

A 3D engine for games needs physics: rigid bodies, colliders, joints, character controllers, spatial queries. The choice has long-term consequences for determinism, performance, and feature coverage.

The physics layer must:

- Cover the basics: dynamic / kinematic / fixed rigid bodies, common collider shapes, joints, raycasts, contact events.
- Be deterministic at fixed timestep (so multiplayer replays are reproducible in v0.2+).
- Run without C/C++ build dependencies.
- Allow swappable backends (we may want a different solver in the future).

## Decision

Adopt **`rapier3d` 0.18+** (pure-Rust, ported from the original Rapier C++/Rust work) as the v0.1 physics solver, exposed through a `hyge-physics` crate with a **`physics-rapier` feature flag**. The flag defaults on; turning it off removes rapier and leaves the crate as pure trait + component definitions (no solver).

## Consequences

### Positive

- **Pure Rust:** no C deps, no CMake, no system libraries. `cargo build` works everywhere.
- **Battle-tested:** Rapier is used in production by several Rust games; the bug surface is well-known.
- **Feature-complete for v0.1:** rigid bodies, common collider shapes, joints, character controllers, spatial queries, contact events.
- **Determinism:** rapier's solver is deterministic at fixed timestep (given identical input).
- **Swappable:** the `physics-rapier` feature flag means we can ship a `physics-physx` or `physics-jolt` later without breaking the public API of `hyge-physics` (just the implementation).

### Negative

- **Compile time:** rapier pulls in a lot of code; the workspace build is slower with `physics-rapier` on. CI caches mitigate this.
- **Solver constraints:** rapier's solver is good but not state-of-the-art; some advanced materials (e.g. fracture, soft bodies) are deferred.
- **Rapier is not a research project:** if the solver needs exotic features in the future, we own the gap until we replace it.

## Alternatives Considered

### PhysX via `physx-rs` (C bindings)

- **Pros:** Industry-standard solver; extensive feature set.
- **Cons:** C deps; bindgen; platform-specific build; harder to contribute to.
- **Rejected for v0.1** because the C dependency conflicts with the "pure Rust toolchain" principle; can be re-introduced as a `physics-physx` feature later.

### Custom solver

- **Pros:** Total control; we could design for our exact needs.
- **Cons:** 6+ months of foundational work before physics is usable; risks re-inventing CCD/contact-manifold bugs.
- **Rejected because:** the productivity loss is incompatible with the v0.1 schedule.

### No physics in v0.1

- **Pros:** Smaller scope.
- **Cons:** "Complete 3D game engine" implies physics; without it, scenes are static.
- **Rejected because:** physics is in the v0.1 feature checklist (see `docs/architecture.md` §15.2).

## References

- `docs/architecture.md` §6.7 (hyge-physics), §11 (cross-cutting)
- `rapier3d` documentation: <https://rapier.rs/docs/>
- ADR-0001 (graphics API), ADR-0002 (bevy_ecs) — physics integrates via ECS components
