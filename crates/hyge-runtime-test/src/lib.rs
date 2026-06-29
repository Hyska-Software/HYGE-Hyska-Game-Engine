//! Hyge runtime test: shared test utilities for the renderer.
//!
//! Provides `TestRenderer` (headless `wgpu` without a surface),
//! `capture_frame`, `assert_image_matches` (SSIM), and `hash_image` (BLAKE3).
//! Used by every crate that needs GPU tests; the snapshot tests in
//! `tests/snapshots/*.png` are diffed by this crate's helpers.
//!
//! See `docs/architecture.md` §6.15 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-090, R-092.
