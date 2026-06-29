//! Color spaces used by Hyge.
//!
//! - [`LinearRGB`]: physically-meaningful linear-space RGB. Components are
//!   `f32` in `[0, 1]`; HDR values > 1 are allowed for physically-based
//!   light energy.
//! - [`Srgb`]: display-encoded sRGB. Components are `u8` in `[0, 255]`.
//!   Internally treated as IEC 61966-2-1 (the "true" sRGB curve, not the
//!   gamma-2.2 approximation).
//!
//! The round-trip `linear -> srgb -> linear` has a worst-case error of one
//! 8-bit quantum (`1/255 ≈ 0.4%`) per channel, well inside the 0.5% budget
//! required by R-010.

use glam::Vec3;

/// Linear-space RGB color.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinearRGB(pub Vec3);

impl LinearRGB {
    /// Linear-space black (`(0, 0, 0)`).
    pub const BLACK: LinearRGB = LinearRGB(Vec3::ZERO);
    /// Linear-space white (`(1, 1, 1)`).
    pub const WHITE: LinearRGB = LinearRGB(Vec3::ONE);

    /// Constructs from per-channel `f32` values (no clamping).
    #[inline]
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self(Vec3::new(r, g, b))
    }

    /// Encodes to sRGB. Values outside `[0, 1]` are clamped before
    /// quantization to `u8`.
    #[inline]
    pub fn to_srgb(&self) -> Srgb {
        Srgb([
            linear_to_srgb_channel(self.0.x),
            linear_to_srgb_channel(self.0.y),
            linear_to_srgb_channel(self.0.z),
        ])
    }
}

impl From<Srgb> for LinearRGB {
    fn from(s: Srgb) -> Self {
        s.to_linear()
    }
}

/// sRGB-encoded color, ready for storage in PNG / display framebuffer.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Srgb(pub [u8; 3]);

impl Srgb {
    /// Constructs from three 8-bit channels.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b])
    }

    /// Decodes to linear-space RGB.
    #[inline]
    pub fn to_linear(&self) -> LinearRGB {
        LinearRGB(Vec3::new(
            srgb_to_linear_channel(self.0[0]),
            srgb_to_linear_channel(self.0[1]),
            srgb_to_linear_channel(self.0[2]),
        ))
    }
}

impl From<LinearRGB> for Srgb {
    fn from(l: LinearRGB) -> Self {
        l.to_srgb()
    }
}

/// sRGB transfer function (encode). IEC 61966-2-1 piecewise form.
#[inline]
fn linear_to_srgb_channel(c: f32) -> u8 {
    let v = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// sRGB inverse transfer function (decode). IEC 61966-2-1 piecewise form.
#[inline]
fn srgb_to_linear_channel(byte: u8) -> f32 {
    let s = byte as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_within_half_percent() {
        // For 1024 evenly-spaced values in [0, 1], the max error after a
        // linear -> srgb -> linear round-trip must be < 0.5% (= < 0.005).
        // The dominant error source is the 1/255 ≈ 0.39% quantization of
        // the 8-bit channel; the piecewise transfer function itself is
        // well below that.
        let mut max_err = 0.0_f32;
        for i in 0..1024 {
            let v = i as f32 / 1023.0;
            let l = LinearRGB(Vec3::splat(v));
            let s = l.to_srgb();
            let l2 = s.to_linear();
            for c in 0..3 {
                let err = (l.0[c] - l2.0[c]).abs();
                if err > max_err {
                    max_err = err;
                }
            }
        }
        assert!(
            max_err < 0.005,
            "max round-trip error = {max_err}, expected < 0.005"
        );
    }

    #[test]
    fn black_and_white_preserved() {
        let black = LinearRGB::BLACK.to_srgb();
        assert_eq!(black, Srgb([0, 0, 0]));
        let white = LinearRGB::WHITE.to_srgb();
        assert_eq!(white, Srgb([255, 255, 255]));
        assert_eq!(black.to_linear(), LinearRGB::BLACK);
        assert_eq!(white.to_linear(), LinearRGB::WHITE);
    }

    #[test]
    fn srgb_midpoint_is_about_22_percent_linear() {
        // The gamma-corrected midpoint of sRGB (0.5) corresponds to roughly
        // 22% linear intensity. This is the well-known "sRGB midpoint" that
        // makes sRGB-encoded images look natural.
        let mid = Srgb::new(128, 128, 128);
        let lin = mid.to_linear();
        assert!(
            (lin.0.x - 0.2159).abs() < 0.01,
            "expected ~0.216, got {}",
            lin.0.x
        );
    }

    #[test]
    fn from_into_round_trip() {
        let original = LinearRGB(Vec3::new(0.3, 0.6, 0.9));
        let srgb: Srgb = original.into();
        let back: LinearRGB = srgb.into();
        for c in 0..3 {
            assert!((original.0[c] - back.0[c]).abs() < 0.005);
        }
    }
}
