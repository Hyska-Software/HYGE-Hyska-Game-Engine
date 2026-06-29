//! Hyge runtime test: shared test utilities for the renderer.
//!
//! R-024 lands the basic test harness:
//!
//! - [`TestRenderer`] — headless `wgpu` without a surface. Used
//!   for snapshot tests that need to render to an off-screen
//!   target without an actual window.
//! - [`capture_frame`] — copies a rendered [`wgpu::Texture`] to
//!   CPU memory and unpads the bytes (wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`
//!   can leave gaps between rows).
//! - [`assert_image_matches`] — pixel-diff comparison with a
//!   tolerance (R-024). The full SSIM implementation lands in
//!   R-092; for now this catches the "did the right pixels get
//!   written" question without requiring a saved reference image.
//! - [`hash_image`] — BLAKE3 hash of a byte buffer, for "did the
//!   rendered output change since last run" sanity checks.
//!
//! The reference snapshot images live in `tests/snapshots/*.png`
//! and are diffed by these helpers.

#![warn(missing_docs)]

pub mod capture;
pub mod compare;
pub mod renderer;

pub use capture::capture_frame;
pub use compare::{assert_image_matches, hash_image};
pub use renderer::TestRenderer;
