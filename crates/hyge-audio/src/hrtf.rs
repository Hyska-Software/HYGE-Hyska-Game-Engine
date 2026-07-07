//! HRTF integration placeholder.

use hyge_core::prelude::{HygeError, HygeResult};

/// Initializes HRTF output when the dataset/backend is available.
///
/// # Errors
///
/// Always returns unsupported until the HRTF dataset is selected in R-073+.
pub fn init() -> HygeResult<()> {
    Err(HygeError::unsupported(
        "audio-hrtf is gated until a distributable HRTF dataset is selected",
    ))
}
