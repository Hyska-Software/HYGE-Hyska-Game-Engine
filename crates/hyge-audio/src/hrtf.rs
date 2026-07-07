//! HRTF (Head-Related Transfer Function) binaural rendering.
//!
//! Behind `audio-hrtf`, this module provides a real HRTF renderer using the
//! `hrtf` crate. The crate ships with no audio data, so a valid HRIR sphere
//! file must be provided at runtime.
//!
//! When `audio-hrtf` is not active, spatial audio falls back to stereo panning
//! via Kira's built-in spatial sub tracks.

#[cfg(feature = "audio-hrtf")]
use std::path::Path;

#[cfg(feature = "audio-hrtf")]
use hrtf::{HrirSphere, HrtfContext, HrtfProcessor};
use hyge_core::prelude::{HygeError, HygeResult, Vec3};

/// HRTF renderer state, available only when the feature is enabled.
///
/// Owns a loaded HRIR sphere and a pre-allocated processor. The processor
/// expects interleaved stereo float samples.
#[cfg(feature = "audio-hrtf")]
pub struct HrtfRenderer {
    processor: HrtfProcessor,
    prev_left: Vec<f32>,
    prev_right: Vec<f32>,
}

#[cfg(feature = "audio-hrtf")]
impl HrtfRenderer {
    /// Creates a new HRTF renderer from an HRIR sphere.
    ///
    /// `interpolation_steps` and `block_len` control the overlap-save
    /// convolution.
    #[must_use]
    pub fn new(sphere: HrirSphere, interpolation_steps: usize, block_len: usize) -> Self {
        Self {
            processor: HrtfProcessor::new(sphere, interpolation_steps, block_len),
            prev_left: Vec::new(),
            prev_right: Vec::new(),
        }
    }

    /// Loads an HRIR sphere from a file.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Parse`] if the file cannot be loaded or parsed.
    pub fn from_file(path: &Path, sample_rate: u32) -> HygeResult<Self> {
        let sphere = HrirSphere::from_file(path, sample_rate).map_err(|e| {
            HygeError::parse(format!(
                "failed to load HRIR sphere from {}: {e:?}",
                path.display()
            ))
        })?;
        Ok(Self::new(sphere, 8, 128))
    }

    /// Processes a mono source into interleaved stereo output using HRTF.
    ///
    /// `source` is the mono input buffer, `output` receives interleaved stereo
    /// frames `(left, right)`.
    pub fn process_stereo(
        &mut self,
        source: &[f32],
        output: &mut [(f32, f32)],
        position: Vec3,
        prev_position: Vec3,
        gain: f32,
    ) {
        let context = HrtfContext {
            source,
            output,
            new_sample_vector: hrtf::Vec3 {
                x: position.x,
                y: position.y,
                z: position.z,
            },
            prev_sample_vector: hrtf::Vec3 {
                x: prev_position.x,
                y: prev_position.y,
                z: prev_position.z,
            },
            prev_left_samples: &mut self.prev_left,
            prev_right_samples: &mut self.prev_right,
            new_distance_gain: gain,
            prev_distance_gain: gain,
        };
        self.processor.process_samples(context);
    }
}

/// HRTF mode: enabled (with a loaded renderer) or disabled.
#[derive(Default)]
pub enum HrtfMode {
    /// No HRTF available; use stereo panning via Kira.
    #[default]
    Disabled,
    /// HRTF is active with a loaded renderer.
    #[cfg(feature = "audio-hrtf")]
    Enabled(Box<HrtfRenderer>),
}

impl HrtfMode {
    /// Attempts to load an HRIR sphere from a file path.
    ///
    /// On success returns [`HrtfMode::Enabled`], on failure returns `Disabled`
    /// with a diagnostic log.
    #[cfg(feature = "audio-hrtf")]
    #[must_use]
    pub fn from_file(path: &Path, sample_rate: u32) -> Self {
        match HrirSphere::from_file(path, sample_rate) {
            Ok(sphere) => Self::Enabled(Box::new(HrtfRenderer::new(sphere, 8, 128))),
            Err(e) => {
                tracing::warn!(
                    "failed to load HRIR sphere from {}, HRTF disabled: {e:?}",
                    path.display()
                );
                Self::Disabled
            }
        }
    }

    /// Always returns `Disabled` when the feature is not active.
    #[cfg(not(feature = "audio-hrtf"))]
    #[must_use]
    pub fn from_file(_path: &Path, _sample_rate: u32) -> Self {
        Self::Disabled
    }

    /// Returns true when HRTF is active.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        #[cfg(feature = "audio-hrtf")]
        {
            matches!(self, Self::Enabled(_))
        }
        #[cfg(not(feature = "audio-hrtf"))]
        {
            false
        }
    }
}

/// Initializes HRTF state.
///
/// Runtime HRIR loading is performed via [`HrtfMode::from_file`]; this
/// initializer merely verifies that the feature is compiled in.
///
/// # Errors
///
/// Returns an error only if `audio-hrtf` is not enabled.
pub fn init() -> HygeResult<()> {
    #[cfg(feature = "audio-hrtf")]
    {
        Ok(())
    }
    #[cfg(not(feature = "audio-hrtf"))]
    {
        Err(HygeError::unsupported(
            "audio-hrtf feature is disabled; enable it to use HRTF rendering",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_disabled() {
        let mode = HrtfMode::default();
        assert!(!mode.is_enabled());
    }

    #[test]
    fn init_matches_feature_flag() {
        #[cfg(feature = "audio-hrtf")]
        {
            assert!(init().is_ok());
        }
        #[cfg(not(feature = "audio-hrtf"))]
        {
            assert!(init().is_err());
        }
    }

    #[test]
    fn from_missing_file_returns_disabled() {
        let mode = HrtfMode::from_file(std::path::Path::new("/nonexistent/hrir.bin"), 44_100);
        assert!(!mode.is_enabled());
    }
}
