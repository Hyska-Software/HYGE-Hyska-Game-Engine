//! `hyge-tools inspect` — print metadata for a single asset hash.
//!
//! Stub for R-033. The full asset-inspection command ships in a
//! later roadmap item.

use hyge_core::result::{HygeError, HygeResult};

/// Stub for the `inspect` subcommand. Always returns
/// [`HygeError::Unsupported`] pointing at the roadmap item that
/// will deliver the real implementation.
pub fn run(_hash: &str) -> HygeResult<()> {
    Err(HygeError::Unsupported(
        "hyge-tools inspect: deferred to a later roadmap item (see docs/roadmap.toml R-091)"
            .to_string(),
    ))
}
