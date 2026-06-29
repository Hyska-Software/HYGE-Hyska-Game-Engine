//! Hyge core: foundational types shared by every other crate.
//!
//! Provides math wrappers over `glam`, color-space conversions, `tracing`-based
//! logging, the `AssetId` BLAKE3 newtype, and the `HygeError` / `HygeResult`
//! error types.
//!
//! See `docs/architecture.md` §6.1 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-010.
