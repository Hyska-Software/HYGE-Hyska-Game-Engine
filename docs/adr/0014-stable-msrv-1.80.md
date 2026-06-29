# ADR-0014: Stable Channel Only, MSRV 1.80

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

A modern Rust engine can lean on nightly features (GAT stabilization, type-level integers, const generics improvements). The trade is well-known:

- **Nightly:** newer features available; rapid iteration on the engine.
- **Stable:** reproducible builds; users do not need a specific toolchain; CI is faster and less brittle; ecosystem crates are stable.

The engine targets game studios and indie developers who need a stable, predictable toolchain. A "you must use nightly-2026-04-12" requirement is a real adoption blocker.

## Decision

- **Stable channel only.** `rust-toolchain.toml` pins `channel = "stable"`.
- **MSRV 1.80.** `Cargo.toml` declares `rust-version = "1.80"`; CI tests against 1.80 + current stable.
- **No nightly features.** If a feature is only on nightly, we wait for stabilization or work around it.
- **No `unsafe_code` in library code** except `hyge-render` and `hyge-render-graph` (where it is gated by `// SAFETY:` comments and audited by `miri` nightly).

## Consequences

### Positive

- **Predictable builds:** any user with `rustup` can build the engine today, in 6 months, in 2 years.
- **Reproducible CI:** no nightly churn breaking builds.
- **Ecosystem alignment:** every crate we depend on (bevy_ecs, wgpu, kira, rapier3d, mlua, egui) targets stable.
- **Cleaner unsafe story:** the unsafe surface is small and audited.

### Negative

- **No nightly-only language features** in v0.1. If we want GAT stabilization or similar, we wait or work around.
- **`miri` runs on nightly only**; our unsafe audit is a separate nightly CI job (see `AGENTS.md` §3.5).
- **Newer toolchain features** (e.g. recent clippy lints) require us to track the toolchain ourselves; we accept this.

## Alternatives Considered

### Nightly for the engine, stable for users (dual toolchain)

- **Pros:** Engine can use nightly features; users can stay on stable.
- **Cons:** Cargo metadata differs across toolchains; unpredictable MSRV; some features need `#![feature(...)]` at the crate root, leaking nightly into dependent crates.
- **Rejected because:** the complexity outweighs the benefit; the stable-only approach is good enough for v0.1.

### MSRV 1.70 (older, broader compatibility)

- **Pros:** Supports more users.
- **Cons:** Misses several 1.71-1.80 stabilizations (clippy lints, cargo resolver = "2" improvements, generic associated types in stable form for some libraries).
- **Rejected because:** 1.80 is a sensible floor that doesn't lock out anyone we care about, and it gives us access to the recent stabilizations.

### No MSRV declaration

- **Pros:** Maximum flexibility.
- **Cons:** Users on older toolchains get cryptic errors; CI can't easily test the floor.
- **Rejected because:** declaring the MSRV is a contract; the contract is a feature, not a constraint.

## References

- `docs/architecture.md` §4 (closed decisions) and §15.5 (quality)
- `AGENTS.md` §3 (build, test, lint, format)
- `rust-toolchain.toml`, `Cargo.toml` `rust-version` field
- ADR-0009 (rapier3d) — also pure Rust, also stable-only
