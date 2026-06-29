//! `hyge-tools doctor` — diagnose a project.
//!
//! Stub for R-033. The diagnostic suite ships in a later roadmap
//! item (R-160). See `docs/architecture.md` §13 for the planned
//! check list (missing assets, orphan cache files, schema version
//! mismatches, plugin presence, unsafe audit).

use std::path::Path;

use hyge_core::result::{HygeError, HygeResult};

/// Stub for the `doctor` subcommand. Always returns
/// [`HygeError::Unsupported`] pointing at the roadmap item that
/// will deliver the real implementation.
pub fn run(_project: &Path) -> HygeResult<()> {
    Err(HygeError::Unsupported(
        "hyge-tools doctor: deferred to a later roadmap item (see docs/roadmap.toml R-160)"
            .to_string(),
    ))
}
