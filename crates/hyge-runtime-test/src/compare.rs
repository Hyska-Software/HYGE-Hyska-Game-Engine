//! Image comparison + hashing utilities for snapshot tests.
//!
//! R-024 ships a simple pixel-diff comparison
//! ([`assert_image_matches`]); the structural-similarity
//! (SSIM) implementation lands in R-092. [`hash_image`]
//! provides a BLAKE3 hash of a byte buffer for "did the output
//! change since last run" sanity checks.

/// Compares two RGBA8 image buffers using a per-pixel diff
/// with a tolerance.
///
/// `actual` and `expected` must be the same length and are
/// interpreted as row-major RGBA8 (4 bytes per pixel). For
/// every pixel, the maximum per-channel absolute difference
/// is computed. A pixel is considered "matching" if its max
/// diff is `≤ 255 * tolerance`. The image is considered
/// matching if the fraction of non-matching pixels is `≤
/// tolerance`.
///
/// # Panics
///
/// Panics if the buffer lengths differ or if the matching
/// fraction exceeds `tolerance`.
///
/// # Example
///
/// ```should_panic
/// let actual = vec![0u8; 16];      // 2x2 black
/// let expected = vec![255u8; 16];  // 2x2 white
/// hyge_runtime_test::assert_image_matches(&actual, &expected, 2, 2, 0.01);
/// ```
pub fn assert_image_matches(actual: &[u8], expected: &[u8], width: u32, height: u32, tolerance: f32) {
    assert!(
        (0.0..=1.0).contains(&tolerance),
        "tolerance must be in [0, 1], got {tolerance}"
    );
    let expected_len = (width as usize) * (height as usize) * 4;
    assert_eq!(
        actual.len(),
        expected_len,
        "actual image has {} bytes, expected {expected_len}",
        actual.len()
    );
    assert_eq!(
        expected.len(),
        expected_len,
        "expected image has {} bytes, expected {expected_len}",
        expected.len()
    );

    let num_pixels = (width as usize) * (height as usize);
    let max_per_channel = (255.0 * tolerance).ceil() as i32;
    let mut non_matching = 0usize;
    for i in 0..num_pixels {
        let base = i * 4;
        let dr = (actual[base] as i32 - expected[base] as i32).abs();
        let dg = (actual[base + 1] as i32 - expected[base + 1] as i32).abs();
        let db = (actual[base + 2] as i32 - expected[base + 2] as i32).abs();
        let da = (actual[base + 3] as i32 - expected[base + 3] as i32).abs();
        let max = dr.max(dg).max(db).max(da);
        if max > max_per_channel {
            non_matching += 1;
        }
    }

    let diff_frac = non_matching as f32 / num_pixels as f32;
    assert!(
        diff_frac <= tolerance,
        "image mismatch: {non_matching} of {num_pixels} pixels ({:.2}%) exceed tolerance {:.2}%",
        diff_frac * 100.0,
        tolerance * 100.0,
    );
}

/// Returns the BLAKE3 hash of `data` as a `blake3:<hex>` string.
///
/// Useful for "did the rendered output change since last run"
/// sanity checks: print the hash on stderr so a regression
/// shows up in the CI log, and write the hash to a snapshot
/// file when you need a quick reference check.
#[must_use]
pub fn hash_image(data: &[u8]) -> String {
    let hash = blake3::hash(data);
    format!("blake3:{}", hash.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_match() {
        let img = vec![128u8; 4 * 4 * 4]; // 4x4 mid-gray
        assert_image_matches(&img, &img, 4, 4, 0.0);
    }

    #[test]
    fn small_diff_within_tolerance() {
        let actual = vec![0u8; 4 * 4 * 4];
        let mut expected = vec![0u8; 4 * 4 * 4];
        // Move 5 of the 16 pixels by +10 (well within 1% tolerance
        // at 10% per-channel).
        for px in 0..5 {
            expected[px * 4] = 10;
        }
        // 5/16 = 31% mismatched pixels. Tolerance must be >= 0.31.
        assert_image_matches(&actual, &expected, 4, 4, 0.5);
    }

    #[test]
    fn large_diff_fails() {
        let actual = vec![0u8; 4 * 4 * 4];
        let expected = vec![255u8; 4 * 4 * 4];
        // Every pixel differs by 255. With tolerance 0.0,
        // 100% mismatch → panic.
        let result = std::panic::catch_unwind(|| {
            assert_image_matches(&actual, &expected, 4, 4, 0.0);
        });
        assert!(result.is_err());
    }

    #[test]
    fn mismatched_length_panics() {
        let actual = vec![0u8; 4];
        let expected = vec![0u8; 8];
        let result = std::panic::catch_unwind(|| {
            assert_image_matches(&actual, &expected, 1, 1, 0.0);
        });
        assert!(result.is_err());
    }

    #[test]
    fn hash_is_deterministic_and_prefixed() {
        let h1 = hash_image(b"hello world");
        let h2 = hash_image(b"hello world");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("blake3:"));
    }

    #[test]
    fn hash_distinguishes_different_inputs() {
        let h1 = hash_image(b"hello");
        let h2 = hash_image(b"world");
        assert_ne!(h1, h2);
    }
}
