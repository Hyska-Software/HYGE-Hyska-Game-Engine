# ADR-0012: Networking Out of Scope for v0.1

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Networking is a major subsystem: transport (UDP/QUIC), replication, snapshot interpolation, client prediction, lag compensation, server authority, NAT traversal. Including it in v0.1 would:

- Add 2-3 months of work to the schedule.
- Force design choices across the ECS (every component must be `Serialize + Deserialize`).
- Compete with the render, asset, and editor subsystems for attention.

The v0.1 engine is single-player local; the user can run a server-side authoritative model or a peer-to-peer model on top of a future `hyge-net` crate.

## Decision

**Networking is out of scope for v0.1.** The roadmap defers it to v0.2 with a dedicated crate `hyge-net` and these features:

- Transport: `quinn` (QUIC) for UDP-with-TLS, NAT-traversal-friendly.
- Replication: ECS-aware snapshot diff/apply, with `Serialize + Deserialize` derives on relevant components.
- Client-side: snapshot interpolation for remote entities, client prediction for local input.
- Server-side: tick-rate decoupling, fixed-timestep authoritative simulation.

Networking is explicitly listed in `docs/architecture.md` §17 as out-of-scope for v0.1.

## Consequences

### Positive

- v0.1 ships on schedule; the engine is complete for single-player games and tooling.
- The ECS design is not compromised by premature `Serialize` derives everywhere; only the components that need it get it.
- We can pick a networking crate after the v0.1 release based on real ecosystem state.

### Negative

- Users who want multiplayer in v0.1 cannot get it; they must wait for v0.2 or implement their own.
- The "complete 3D game engine" claim is qualified: it is complete for single-player local in v0.1, with multiplayer in v0.2.

## Alternatives Considered

### Include networking in v0.1 (server-tick authoritative)

- **Pros:** Ships multiplayer from day one.
- **Cons:** 2-3 months of additional work; risk of slipping the v0.1 release; forces design compromises across the engine.
- **Rejected because:** the cost is not justified for the target use case (single-player local).

### Just transport primitives in v0.1

- **Pros:** Users can roll their own replication.
- **Cons:** Brings in a network dependency without a clean API; the engine is in a half-finished state for multiplayer.
- **Rejected because:** the half-built state is worse than no state; v0.2 will deliver it properly.

## References

- `docs/architecture.md` §17 (out-of-scope for v0.1)
- `quinn` (QUIC for Rust): <https://github.com/quinn-rs/quinn>
- `bevy_replicon` (precedent for ECS replication; we may build on it or write our own in v0.2)
