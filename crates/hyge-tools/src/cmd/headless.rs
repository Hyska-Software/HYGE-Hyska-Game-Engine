//! `hyge-tools headless` — render a cooked scene to a PNG without a window.
//!
//! Stub for R-033. The real headless render path ships in a later
//! roadmap item. The function parses the args and returns
//! [`HygeError::Unsupported`] so the dispatcher is exercised
//! end-to-end and the gap is visible to users.

use std::path::Path;

use hyge_core::result::{HygeError, HygeResult};

/// Stub for the `headless` subcommand. Always returns
/// [`HygeError::Unsupported`] pointing at the roadmap item that
/// will deliver the real implementation.
#[allow(clippy::too_many_arguments)]
pub fn run(
    _scene: &Path,
    _camera: &str,
    _out: &Path,
    _width: u32,
    _height: u32,
    _samples: u32,
) -> HygeResult<()> {
    Err(HygeError::Unsupported(
        "hyge-tools headless: deferred to a later roadmap item (see docs/roadmap.toml R-127)"
            .to_string(),
    ))
}
