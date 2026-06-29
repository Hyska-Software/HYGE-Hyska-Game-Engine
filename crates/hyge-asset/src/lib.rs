//! Hyge asset: the content-addressed asset DB, loader registry, and hot-reload
//! pipeline.
//!
//! Every asset is identified by its BLAKE3 hash (`AssetId = [u8; 32]`). The
//! `AssetDb` (SQLite) maps hashes to on-disk cache paths and records the
//! dependency graph. The `FileWatcher` (via `notify`) detects FS events and
//! triggers re-imports on `AsyncComputeTaskPool`.
//!
//! See `docs/architecture.md` §6.5 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-030..R-037.
