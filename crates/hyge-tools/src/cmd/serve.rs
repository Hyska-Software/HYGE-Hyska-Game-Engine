//! `hyge-tools serve` — serve a project over HTTP for the editor.
//!
//! Stub for R-033. The asset server ships in a later roadmap item
//! (R-150).

use std::path::Path;

use hyge_core::result::{HygeError, HygeResult};

/// Stub for the `serve` subcommand. Always returns
/// [`HygeError::Unsupported`] pointing at the roadmap item that
/// will deliver the real implementation.
pub fn run(_project: &Path, _port: u16) -> HygeResult<()> {
    Err(HygeError::Unsupported(
        "hyge-tools serve: deferred to a later roadmap item (see docs/roadmap.toml R-150)"
            .to_string(),
    ))
}
