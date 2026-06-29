# ADR-0006: BLAKE3 Content-Addressed Asset Pipeline

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Game assets change constantly during development (DCC edits, material tweaks, mesh re-exports). The engine must:

- Detect changes without hashing entire files for every check.
- Re-import only the assets that actually changed, and cascade the invalidation to dependents.
- Provide a stable identity for every asset that survives file moves and renames.
- Support hot-reload during development.

## Decision

Adopt a **BLAKE3 content-addressed** asset pipeline:

- Every asset's identity is the BLAKE3 hash of its content (`AssetId` = `[u8; 32]`).
- A SQLite database (`.hyge.db`) maps `AssetId → cache path` and records the dependency graph.
- Source-of-truth formats: glTF 2.0 (models), KTX2/BasisU (textures), OGG/Opus (audio), Lua (scripts), TOML (input bindings), RON (metadata).
- The CLI tool `hyge-tools import` produces a deterministic cache: `<blake3>.hyge-mesh`, `<blake3>.hyge-mat`, `<blake3>.ktx2`, `<blake3>.hyge-meta.json`.
- The runtime `notify` watcher detects FS events, resolves them to `AssetId`s, and re-imports on `AsyncComputeTaskPool`. The bindless slot is atomically swapped.

## Consequences

### Positive

- **Stable identity:** two assets with the same content have the same `AssetId`, regardless of where they live on disk.
- **Cache reuse:** identical assets from different sources share the same on-disk cache; no re-import.
- **Incremental hot-reload:** only changed assets and their dependents are re-baked; everything else stays GPU-resident.
- **DB-backed dependency graph:** material changes invalidate every mesh using that material automatically; no manual bookkeeping.
- **BLAKE3 is fast:** hashing is ~1 GB/s/core; per-asset hashing is not a bottleneck.

### Negative

- Building a hash for every load is mandatory; an unoptimized path is a perf footgun.
- The SQLite DB is a coordination point; tests touching it need `#[serial_test::serial]` to avoid contention.
- The DB must be wiped or migrated on breaking schema changes; we mitigate with a `db_version` table and a migration runner.

## Alternatives Considered

### Path-based asset identity

- **Pros:** Trivial to implement.
- **Cons:** Renames break everything; two copies of the same file are different assets; hot-reload cannot detect "same content, different path".
- **Rejected because:** breaks the hot-reload and reuse story.

### UUID assigned at import time

- **Pros:** Stable across file moves; unique.
- **Cons:** Two identical files get different UUIDs; the DB is the only source of truth; merging two checkouts is painful.
- **Rejected because:** the content-addressed approach gives both uniqueness and content-equality for free.

### SHA-256

- **Pros:** Ubiquitous; well-known.
- **Cons:** Slower than BLAKE3 (typically 2-4×); no streaming API as clean.
- **Rejected because:** BLAKE3 is the modern choice and is faster on every measurable workload.

## References

- `docs/architecture.md` §6.5 (hyge-asset), §9 (asset pipeline)
- BLAKE3: <https://github.com/BLAKE3-team/BLAKE3>
- `notify` crate (file system events): <https://docs.rs/notify>
- ADR-0007 (scene/prefab) — prefab identity is also BLAKE3
