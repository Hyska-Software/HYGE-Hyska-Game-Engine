//! Hyge core: foundational types shared by every other crate.
//!
//! Provides math wrappers over [`glam`], color-space conversions,
//! [`tracing`]-based logging initialization, the [`AssetId`](id::AssetId)
//! BLAKE3 newtype, and the [`HygeError`](result::HygeError) /
//! [`HygeResult`](result::HygeResult) error types.
//!
//! See `docs/architecture.md` §6.1 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-010.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod color;
pub mod id;
pub mod log;
pub mod math;
pub mod result;

pub mod prelude;

/// Engine-wide logging macro. Thin re-export of `tracing`'s level macros
/// under a Hyge-specific name for grep-ability and for future
/// engine-specific enrichment (per-call site, log target, etc.).
///
/// # Examples
///
/// ```no_run
/// use hyge_core::hyge_log;
///
/// hyge_log!(info, "loaded asset {}", "abc");
/// hyge_log!(warn, "falling back to default");
/// ```
#[macro_export]
macro_rules! hyge_log {
    (info,  $($arg:tt)*) => { ::tracing::info!($($arg)*) };
    (warn,  $($arg:tt)*) => { ::tracing::warn!($($arg)*) };
    (error, $($arg:tt)*) => { ::tracing::error!($($arg)*) };
    (debug, $($arg:tt)*) => { ::tracing::debug!($($arg)*) };
    (trace, $($arg:tt)*) => { ::tracing::trace!($($arg)*) };
}
