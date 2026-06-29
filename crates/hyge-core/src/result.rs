//! Engine-wide error type and `Result` alias.
//!
//! Every fallible function in Hyge returns [`HygeResult<T>`]. The error
//! variants are intentionally coarse; the inner string carries any
//! engine-specific context. The intent is that callers can pattern-match
//! on the variant to decide whether to retry, fall back, or propagate,
//! without needing to inspect deeply nested error types from upstream
//! crates.

use thiserror::Error;

/// The single engine-wide error type. New variants may be added in
/// minor versions; existing variants are never renamed or removed without
/// a major version bump (semver).
#[derive(Debug, Error)]
pub enum HygeError {
    /// Wraps any `std::io::Error`. The most common source: file I/O for
    /// asset loading, project cook, headless screenshot output.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic parse / deserialization error. Covers TOML, JSON, msgpack,
    /// RON, glTF, etc. The string carries the underlying error message
    /// (typically already-formatted by the upstream parser).
    #[error("parse error: {0}")]
    Parse(String),

    /// GPU error reported by `wgpu` / `naga`. Wraps the upstream error
    /// message; structured information is in the string.
    #[error("GPU error: {0}")]
    Gpu(String),

    /// The referenced asset (by `AssetId` or path) is not present in the
    /// asset database. The variant is reserved for "missing entirely" and
    /// is distinct from I/O errors during load.
    #[error("asset not found: {0}")]
    AssetNotFound(String),

    /// Two plugins or components attempted to register the same key.
    /// Usually a programming error, surfaced as a hard failure (see
    /// `hyge-app`).
    #[error("plugin conflict: {0}")]
    PluginConflict(String),

    /// Invalid argument to an engine API. The string carries the parameter
    /// name and a description of why it was rejected.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The operation is not supported on the current platform, by the
    /// current feature configuration, or by the active backend.
    #[error("operation not supported: {0}")]
    Unsupported(String),

    /// A cycle was detected in a render-graph DAG during `compile()`.
    /// The string names the offending node (pass or resource label).
    /// Cycles are programmer errors: review the `reads`/`writes`
    /// declarations on your passes.
    #[error("render graph cycle: {0}")]
    RenderGraphCycle(String),
}

impl HygeError {
    /// Constructs a [`HygeError::Parse`] from anything `Into<String>`.
    pub fn parse<S: Into<String>>(msg: S) -> Self {
        HygeError::Parse(msg.into())
    }

    /// Constructs a [`HygeError::Gpu`] from anything `Into<String>`.
    pub fn gpu<S: Into<String>>(msg: S) -> Self {
        HygeError::Gpu(msg.into())
    }

    /// Constructs an [`HygeError::AssetNotFound`] from anything `Into<String>`.
    pub fn asset_not_found<S: Into<String>>(msg: S) -> Self {
        HygeError::AssetNotFound(msg.into())
    }

    /// Constructs a [`HygeError::PluginConflict`] from anything `Into<String>`.
    pub fn plugin_conflict<S: Into<String>>(msg: S) -> Self {
        HygeError::PluginConflict(msg.into())
    }

    /// Constructs an [`HygeError::InvalidArgument`] from anything `Into<String>`.
    pub fn invalid_argument<S: Into<String>>(msg: S) -> Self {
        HygeError::InvalidArgument(msg.into())
    }

    /// Constructs a [`HygeError::Unsupported`] from anything `Into<String>`.
    pub fn unsupported<S: Into<String>>(msg: S) -> Self {
        HygeError::Unsupported(msg.into())
    }

    /// Constructs a [`HygeError::RenderGraphCycle`] from anything `Into<String>`.
    /// The string names the offending node in the render graph DAG.
    pub fn render_graph_cycle<S: Into<String>>(msg: S) -> Self {
        HygeError::RenderGraphCycle(msg.into())
    }
}

/// Engine-wide `Result` alias.
///
/// Use `?` to propagate `HygeError` from any fallible function:
///
/// ```no_run
/// use hyge_core::prelude::*;
///
/// fn load(path: &str) -> HygeResult<Vec<u8>> {
///     let bytes = std::fs::read(path)?;
///     Ok(bytes)
/// }
/// ```
pub type HygeResult<T> = Result<T, HygeError>;
