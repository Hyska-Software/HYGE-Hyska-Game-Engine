//! Post-process utilities for the M4 render chain.
//!
//! The shader sources in `src/shader/` implement the GPU path.  This
//! module also exposes deterministic CPU equivalents used by unit tests
//! and by benchmark sanity checks.

use hyge_core::prelude::Mat4;

/// WGSL source for ACES tonemapping.
pub const TONEMAP_SHADER_SOURCE: &str = include_str!("shader/tonemap.wgsl");
/// WGSL source for temporal anti-aliasing.
pub const TAA_SHADER_SOURCE: &str = include_str!("shader/taa.wgsl");
/// WGSL source for SMAA.
pub const SMAA_SHADER_SOURCE: &str = include_str!("shader/smaa.wgsl");
/// WGSL source for dual-Kawase bloom.
pub const BLOOM_SHADER_SOURCE: &str = include_str!("shader/bloom.wgsl");

/// Bloom configuration shared by CPU tests and the renderer config.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BloomConfig {
    /// Additive bloom intensity.
    pub intensity: f32,
    /// Bright-pass threshold in linear luma.
    pub threshold: f32,
    /// Number of downsample levels.
    pub levels: u32,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self {
            intensity: 0.2,
            threshold: 1.0,
            levels: 5,
        }
    }
}

/// Post-process feature toggles.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PostProcessConfig {
    /// Enables temporal AA.
    pub taa: bool,
    /// Enables SMAA.
    pub smaa: bool,
    /// Enables bloom.
    pub bloom: BloomConfig,
    /// Exposure multiplier used before ACES.
    pub exposure: f32,
}

impl Default for PostProcessConfig {
    fn default() -> Self {
        Self {
            taa: true,
            smaa: true,
            bloom: BloomConfig::default(),
            exposure: 1.0,
        }
    }
}

/// Narkowicz ACES approximation in linear space.
#[must_use]
pub fn aces_filmic(x: f32) -> f32 {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
}

/// Converts one linear color channel to sRGB.
#[must_use]
pub fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Computes the arithmetic mean luma of a one-channel test image.
#[must_use]
pub fn mean_luma(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().sum::<f32>() / samples.len() as f32
}

/// Computes the variance of a one-channel signal.
#[must_use]
pub fn variance(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean = mean_luma(samples);
    samples
        .iter()
        .map(|x| {
            let d = x - mean;
            d * d
        })
        .sum::<f32>()
        / samples.len() as f32
}

/// Applies a small deterministic 1D bloom approximation for tests.
#[must_use]
pub fn apply_bloom_1d(input: &[f32], config: BloomConfig) -> Vec<f32> {
    if input.is_empty() || config.intensity <= 0.0 || config.levels == 0 {
        return input.to_vec();
    }
    let mut bright: Vec<f32> = input
        .iter()
        .map(|v| (v - config.threshold).max(0.0))
        .collect();
    for _ in 0..config.levels {
        bright = blur_1d(&bright);
    }
    input
        .iter()
        .zip(bright)
        .map(|(base, bloom)| base + bloom * config.intensity)
        .collect()
}

/// Smooths hard edges with a simple SMAA-like neighborhood resolve.
#[must_use]
pub fn smaa_smooth_1d(input: &[f32], threshold: f32) -> Vec<f32> {
    if input.len() < 3 {
        return input.to_vec();
    }
    let mut out = input.to_vec();
    for i in 1..input.len() - 1 {
        let edge = (input[i + 1] - input[i - 1]).abs();
        if edge > threshold {
            out[i] = (input[i - 1] + input[i] + input[i + 1]) / 3.0;
        }
    }
    out
}

fn blur_1d(input: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; input.len()];
    for (i, value) in out.iter_mut().enumerate() {
        let left = input.get(i.wrapping_sub(1)).copied().unwrap_or(input[i]);
        let centre = input[i];
        let right = input.get(i + 1).copied().unwrap_or(input[i]);
        *value = left * 0.25 + centre * 0.5 + right * 0.25;
    }
    out
}

/// Persistent TAA history for one-channel deterministic tests.
#[derive(Clone, Debug)]
pub struct TaaHistory {
    history: Vec<f32>,
    previous_view: Option<Mat4>,
    cut_threshold: f32,
}

impl TaaHistory {
    /// Creates a history buffer with `pixel_count` samples.
    #[must_use]
    pub fn new(pixel_count: usize) -> Self {
        Self {
            history: vec![0.0; pixel_count],
            previous_view: None,
            cut_threshold: 10.0,
        }
    }

    /// Resolves a frame with 1/8 history and 7/8 current blend.
    #[must_use]
    pub fn resolve(&mut self, current: &[f32], view: Mat4) -> Vec<f32> {
        let camera_cut = self
            .previous_view
            .map(|prev| matrix_delta(prev, view) > self.cut_threshold)
            .unwrap_or(true);
        self.previous_view = Some(view);
        if camera_cut || self.history.len() != current.len() {
            self.history = current.to_vec();
            return current.to_vec();
        }
        let resolved: Vec<f32> = current
            .iter()
            .zip(&self.history)
            .map(|(c, h)| (h * 0.125 + c * 0.875).clamp(c.min(*h), c.max(*h)))
            .collect();
        self.history.clone_from(&resolved);
        resolved
    }
}

fn matrix_delta(a: Mat4, b: Mat4) -> f32 {
    let ac = a.to_cols_array();
    let bc = b.to_cols_array();
    ac.iter().zip(bc).map(|(x, y)| (x - y).abs()).sum()
}
