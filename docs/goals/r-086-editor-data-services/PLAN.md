# R-086 — Asset, Console and Profiler Data Services

**Intent:** Expose authoritative AssetDb, tracing and renderer telemetry through bounded editor snapshots.
**Truth Owner:** AssetDb owns assets/dependencies; the Rust session owns console, preview and profiler state.
**Contract Boundary:** `hyge-editor` ↔ `hyge-editor-protocol` ↔ `protocol/editor.schema.json` ↔ PySide6 client.
**Cutover:** All R-086 reads and preview jobs route through session-owned `EditorDataServices`.
**Acceptance Evidence:** Focused bounded-service tests, real TCP round-trip, Python schema tests, workspace clippy/fmt/docs/test gates.

## Implemented slices

1. Bulk deterministic AssetDb asset/edge queries and bounded asset snapshots.
2. Session-owned console ring, tracing layer, profiler history and deterministic preview manager.
3. Additive v1 protocol messages, JSON Schema payload validation and Python request helpers.
4. Real authenticated TCP dispatch and round-trip integration tests.

## Invariants

- No frontend-owned asset, console or profiler truth.
- Asset snapshots never expose paths outside the project root.
- Console history is capped at 1000 lines; profiler history at 240 samples.
- Preview output is content-derived and written atomically.
