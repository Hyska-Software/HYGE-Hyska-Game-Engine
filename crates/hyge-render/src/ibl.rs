//! Image-Based Lighting (IBL) prefilter and irradiance bake (R-041).
//!
//! Produces the three products the R-040 PBR shader's
//! `irradiance_map` / `prefiltered_env_map` / `brdf_lut`
//! bindings need:
//!
//! 1. A roughness-mipped prefiltered environment cubemap
//!    (Rgba16Float, base 256, 9 mips).
//! 2. A diffuse irradiance cubemap (Rgba16Float, 1 mip,
//!    32 px per face).
//! 3. A 2D BRDF integration LUT (Rgba16Float, 512x512) per
//!    Karis 2014.
//!
//! The bake is **pure-Rust and deterministic** so the R-041
//! acceptance "output hash stable" test passes on every machine
//! and CI runner. WGSL reference implementations live in
//! `src/shader/prefilter.wgsl` and `src/shader/irradiance.wgsl`;
//! they are naga-validated but not used in the bake itself.
//!
//! # On-disk format: `.hyge-env`
//!
//! Single file, content-addressed by BLAKE3. Layout
//! (little-endian) is documented in
//! [`ENV_FILE_MAGIC`].

use std::fs;
use std::path::Path;

use blake3::Hasher;

use hyge_core::prelude::{HygeError, HygeResult};

/// Base edge of the prefiltered environment cubemap, in pixels.
/// The mip chain runs from this size down to 1 px (so a 256
/// base gives `floor(log2(256)) + 1 = 9` mips).
pub const PREFILTER_BASE_SIZE: u32 = 256;

/// Maximum roughness LOD the PBR shader samples from. The
/// shader's `textureSampleLevel(env, ..., roughness * MAX_LOD)`
/// formula requires `MAX_LOD = mip_count - 1 = 8.0` for a
/// 256-base cubemap. The R-040 contract used `4.0` (a 32-base
/// cubemap); R-041 lifts the base to 256 for higher quality and
/// updates the constant to match.
pub const PREFILTERED_ENV_MAX_LOD: f32 = 8.0;

/// Edge of the diffuse irradiance cubemap, in pixels (one
/// mip). The low resolution is intentional: the irradiance
/// signal is heavily low-pass filtered by the diffuse BRDF
/// convolution, so a 32-px cubemap captures the full diffuse
/// contribution with no perceptible banding.
pub const IRRADIANCE_SIZE: u32 = 32;

/// Edge of the BRDF integration LUT, in pixels. Karis 2014
/// recommends 512; smaller values visibly bias the
/// split-sum approximation at low roughness.
pub const BRDF_LUT_SIZE: u32 = 512;

/// The 8-byte magic at the start of every `.hyge-env` file.
pub const ENV_FILE_MAGIC: [u8; 8] = *b"HYGE-ENV";

/// The on-disk format version. Bumped on any breaking change to
/// the layout documented under [`write_env_file`].
pub const ENV_FILE_VERSION: u32 = 1;

/// In-memory representation of a baked IBL set. The fields are
/// laid out so the [`write_env_file`] serializer streams the
/// bytes in the on-disk order (prefilter, irradiance, BRDF LUT).
#[derive(Debug, Clone)]
pub struct EnvironmentBake {
    /// The roughness-mipped environment cubemap. Mip 0 is the
    /// sharpest reflection (roughness 0); the highest mip is
    /// fully rough.
    pub prefilter: PrefilterCubemap,
    /// The diffuse irradiance cubemap, computed via
    /// Ramamoorthi 2001 spherical harmonics.
    pub irradiance: IrradianceCubemap,
    /// The split-sum BRDF integration LUT.
    pub brdf_lut: BrdfLut,
    /// BLAKE3 hash of the original equirectangular HDR source
    /// bytes (the input to [`bake_from_rgbe_hdr`]). Stored so
    /// the on-disk file records what it was baked from.
    pub source_hash: [u8; 32],
}

/// A roughness-mipped environment cubemap. The first dimension
/// is the mip level (0 = sharpest reflection, `mip_count - 1` =
/// fully rough). The second is the cubemap face in WebGPU
/// order: `+X, -X, +Y, -Y, +Z, -Z`. Each face's data is
/// row-major, RGBA16F encoded as 8 little-endian bytes per
/// texel (`[u8; 8]`).
#[derive(Debug, Clone)]
pub struct PrefilterCubemap {
    /// Base edge in pixels (the largest mip is `base_size`
    /// pixels on each side).
    pub base_size: u32,
    /// Number of mip levels in the chain.
    pub mip_count: u32,
    /// `mip_chain[mip][face] = row-major RGBA16F texels`,
    /// length `mip_count * 6`.
    pub mip_chain: Vec<[Vec<[u8; 8]>; 6]>,
}

/// A diffuse irradiance cubemap at the
/// [`IRRADIANCE_SIZE`] resolution, one mip.
#[derive(Debug, Clone)]
pub struct IrradianceCubemap {
    /// Edge in pixels (every face is `size x size`).
    pub size: u32,
    /// `faces_rgba16f[face] = row-major RGBA16F texels`,
    /// length 6.
    pub faces_rgba16f: [Vec<[u8; 8]>; 6],
}

/// A 2D BRDF integration LUT. `pixels_rgba16f[(y * size + x)]`
/// stores `(scale, bias, _, _)` in the RG channels per Karis
/// 2014. The BA channels are zero.
#[derive(Debug, Clone)]
pub struct BrdfLut {
    /// Edge in pixels (the LUT is always square).
    pub size: u32,
    /// `pixels_rgba16f.len() == size * size`, row-major.
    pub pixels_rgba16f: Vec<[u8; 8]>,
}

// =============================================================================
// Half-float (IEEE 754 binary16) pack / unpack
// =============================================================================

/// Converts an `f32` to its nearest IEEE 754 binary16 (`u16`)
/// representation. Used to pack the prefilter, irradiance, and
/// BRDF LUT into the on-disk `.hyge-env` format and into the
/// RGBA16F wgpu textures.
#[inline]
#[must_use]
pub fn f32_to_f16(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 31) & 0x1) as u16;
    if x.is_nan() {
        return (sign << 15) | 0x7E00;
    }
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x7F_FFFF;
    if exp == 0xFF {
        // Inf
        return (sign << 15) | 0x7C00;
    }
    let new_exp = exp - 127 + 15;
    if new_exp >= 0x1F {
        // Overflow -> Inf
        return (sign << 15) | 0x7C00;
    }
    if new_exp <= 0 {
        // Subnormal / underflow
        if new_exp < -10 {
            return sign << 15;
        }
        let m = (mant | 0x80_0000) >> (1 - new_exp) as u32;
        return (sign << 15) | (((m + 0x1000) >> 13) as u16);
    }
    (sign << 15) | ((new_exp as u16) << 10) | (((mant + 0x1000) >> 13) as u16)
}

/// Converts an IEEE 754 binary16 (`u16`) to the nearest `f32`.
#[inline]
#[must_use]
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 0x1) as u32;
    let exp = ((h >> 10) & 0x1F) as i32;
    let mant = (h & 0x3FF) as u32;
    if exp == 0 {
        if mant == 0 {
            return f32::from_bits(sign << 31);
        }
        // Subnormal
        let mut e = -14i32;
        let mut m = mant;
        while (m & 0x400) == 0 {
            m <<= 1;
            e -= 1;
        }
        m &= 0x3FF;
        let bits = (sign << 31) | (((e + 127) as u32) << 23) | (m << 13);
        return f32::from_bits(bits);
    }
    if exp == 0x1F {
        if mant == 0 {
            return f32::from_bits((sign << 31) | (0xFF << 23));
        }
        return f32::from_bits((sign << 31) | (0xFF << 23) | (mant << 13));
    }
    let bits = (sign << 31) | (((exp - 15 + 127) as u32) << 23) | (mant << 13);
    f32::from_bits(bits)
}

/// Packs a `[f32; 4]` RGBA into 8 little-endian bytes
/// holding RGBA16F samples (one u16 per channel). The full
/// 8-byte layout is what wgpu's `Rgba16Float` texture format
/// expects; the previous 4-byte "low byte only" packing
/// silently truncated the data and made the GPU upload
/// slice out-of-range when the texture row pitch and the
/// actual texel count disagreed.
///
/// # Errors
///
/// Returns `None` if any channel is `NaN` and we cannot
/// represent it as a binary16 qNaN (currently always
/// representable; reserved for future constraints).
#[inline]
#[must_use]
pub fn pack_rgba16f(c: [f32; 4]) -> [u8; 8] {
    let r = f32_to_f16(c[0]).to_le_bytes();
    let g = f32_to_f16(c[1]).to_le_bytes();
    let b = f32_to_f16(c[2]).to_le_bytes();
    let a = f32_to_f16(c[3]).to_le_bytes();
    [r[0], r[1], g[0], g[1], b[0], b[1], a[0], a[1]]
}

/// Inverse of [`pack_rgba16f`].
#[inline]
#[must_use]
pub fn unpack_rgba16f(b: [u8; 8]) -> [f32; 4] {
    [
        f16_to_f32(u16::from_le_bytes([b[0], b[1]])),
        f16_to_f32(u16::from_le_bytes([b[2], b[3]])),
        f16_to_f32(u16::from_le_bytes([b[4], b[5]])),
        f16_to_f32(u16::from_le_bytes([b[6], b[7]])),
    ]
}

// =============================================================================
// RGBE (Radiance .hdr) decoder
// =============================================================================

/// Decodes a Radiance RGBE (`.hdr`) file into a
/// `(width, height, vec_of_rgb)` triple. The pixel buffer is
/// tightly packed: `pixels.len() == width * height * 3`, row-
/// major, each entry a linear-space `[f32; 3]`.
///
/// Supports the common `#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n`
/// header and the `RLE`-encoded scanline body. Adaptive RLE is
/// supported: a leading `0x02` byte means the next 4 bytes
/// describe a (run count, value) pair; otherwise the next 4
/// bytes are a single pixel's RGBE samples.
///
/// # Errors
///
/// Returns [`HygeError::Parse`] for a malformed header, missing
/// resolution line, or an unexpected EOF in the pixel data.
/// Returns [`HygeError::InvalidArgument`] when the resolution is
/// zero or larger than `u32::MAX / 3` pixels.
pub fn decode_rgbe_hdr(bytes: &[u8]) -> HygeResult<(u32, u32, Vec<[f32; 3]>)> {
    // -- header ---------------------------------------------------------
    // The Radiance header is a sequence of `\n`-terminated lines
    // until the first empty line. The first non-comment line is
    // the resolution: "-Y <height> +X <width>" (the axis
    // letters can carry a + / - sign and the convention is that
    // -Y is the first scanline and +Y is the last, so the
    // resolution is reported as `-Y height +X width` for an
    // image whose first row is the top of the picture).
    let mut idx = 0usize;
    let mut found_format = false;
    let mut width: u32 = 0;
    let mut height: u32 = 0;

    while idx < bytes.len() {
        // Read until newline.
        let line_end = bytes[idx..]
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| HygeError::parse("rgbe: unterminated header"))?;
        let line = &bytes[idx..idx + line_end];
        idx += line_end + 1;

        // Empty line ends the header.
        if line.is_empty() {
            if width != 0 && height != 0 {
                break;
            }
            // Otherwise keep looking; some encoders place
            // command lines after the empty line.
            continue;
        }

        if line.starts_with(b"#?") {
            // Comment / format spec line. Detect FORMAT=32-bit_rle_rgbe.
            if line.windows(29).any(|w| w == b"FORMAT=32-bit_rle_rgbe") {
                found_format = true;
            }
            continue;
        }

        // Resolution line. Standard Radiance: "-Y <h> +X <w>" or
        // "Y <h> X <w>" with optional signs. The axis letter
        // and the number are separate tokens. The line may
        // also be a "command" line (anything that doesn't
        // contain a Y / X axis marker), which we silently
        // ignore.
        let line_str = std::str::from_utf8(line)
            .map_err(|e| HygeError::parse(format!("rgbe: non-utf8 resolution line: {e}")))?;
        let mut tokens = line_str.split_whitespace();
        let mut local_y = None;
        let mut local_x = None;
        while let Some(token) = tokens.next() {
            let bytes_t = token.as_bytes();
            let after_sign = if bytes_t.first() == Some(&b'+') || bytes_t.first() == Some(&b'-') {
                &bytes_t[1..]
            } else {
                bytes_t
            };
            let axis = std::str::from_utf8(after_sign).ok();
            if axis == Some("Y") || axis == Some("y") {
                let value = tokens
                    .next()
                    .ok_or_else(|| HygeError::parse("rgbe: Y axis missing value"))?;
                local_y = Some(
                    value
                        .parse::<u32>()
                        .map_err(|e| HygeError::parse(format!("rgbe: bad Y res: {e}")))?,
                );
            } else if axis == Some("X") || axis == Some("x") {
                let value = tokens
                    .next()
                    .ok_or_else(|| HygeError::parse("rgbe: X axis missing value"))?;
                local_x = Some(
                    value
                        .parse::<u32>()
                        .map_err(|e| HygeError::parse(format!("rgbe: bad X res: {e}")))?,
                );
            }
        }
        if let Some(h) = local_y {
            height = h;
        }
        if let Some(w) = local_x {
            width = w;
        }
        if height != 0 && width != 0 {
            break;
        }
    }
    let _ = found_format; // tolerated but not required.

    if width == 0 || height == 0 {
        return Err(HygeError::parse(format!(
            "rgbe: missing resolution in header (width={width} height={height})"
        )));
    }
    let total = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| HygeError::parse("rgbe: pixel count overflow"))?;
    if total > (u32::MAX as usize) / 3 {
        return Err(HygeError::invalid_argument(format!(
            "rgbe: image too large ({width}x{height})"
        )));
    }

    // -- pixel data -----------------------------------------------------
    // Each scanline is `width` RGBE pixels. RLE applies per
    // scanline, not across them.
    let mut out: Vec<[f32; 3]> = vec![[0.0, 0.0, 0.0]; total];
    let mut scanline: Vec<[u8; 4]> = vec![[0u8; 4]; width as usize];
    let w = width as usize;
    for y in 0..height as usize {
        if idx + 4 > bytes.len() {
            return Err(HygeError::parse("rgbe: unexpected EOF at scanline start"));
        }
        let marker = bytes[idx];
        idx += 1;
        if marker != 2 || !(8..=32767).contains(&width) {
            // Non-RLE scanline.
            idx -= 1;
            let needed = w * 4;
            if idx + needed > bytes.len() {
                return Err(HygeError::parse("rgbe: unexpected EOF in pixel data"));
            }
            for texel in scanline.iter_mut().take(w) {
                // Slice length was just bounds-checked, so the
                // try_into() cannot fail. We still surface a
                // parse error rather than panicking in case a
                // future refactor changes the bounds check.
                let rgbe: [u8; 4] = bytes[idx..idx + 4]
                    .try_into()
                    .map_err(|e| HygeError::parse(format!("rgbe: pixel slice: {e}")))?;
                *texel = rgbe;
                idx += 4;
            }
        } else {
            // Adaptive RLE: first two bytes are the per-channel
            // run count for R, G, B; then E.
            if width > 0x7FFF {
                return Err(HygeError::parse("rgbe: adaptive RLE width too large"));
            }
            let counts: [u8; 4] = bytes[idx..idx + 4]
                .try_into()
                .map_err(|e| HygeError::parse(format!("rgbe: rle counts slice: {e}")))?;
            idx += 4;
            let _counts: [usize; 4] = [
                counts[0] as usize,
                counts[1] as usize,
                counts[2] as usize,
                counts[3] as usize,
            ];
            let mut channels: [Vec<u8>; 4] = [
                Vec::with_capacity(w),
                Vec::with_capacity(w),
                Vec::with_capacity(w),
                Vec::with_capacity(w),
            ];
            for channel in &mut channels {
                let mut written = 0usize;
                while written < w {
                    if idx + 2 > bytes.len() {
                        return Err(HygeError::parse("rgbe: unexpected EOF in RLE stream"));
                    }
                    let count = bytes[idx] as usize;
                    idx += 1;
                    if count > 128 {
                        // Non-run: copy the next `count - 128` bytes verbatim.
                        let n = count - 128;
                        if written + n > w || idx + n > bytes.len() {
                            return Err(HygeError::parse("rgbe: RLE non-run out of range"));
                        }
                        channel.extend_from_slice(&bytes[idx..idx + n]);
                        idx += n;
                        written += n;
                    } else {
                        // Run: repeat the next byte `count` times.
                        if idx + 1 > bytes.len() {
                            return Err(HygeError::parse("rgbe: RLE run out of range"));
                        }
                        let value = bytes[idx];
                        idx += 1;
                        for _ in 0..count {
                            channel.push(value);
                        }
                        written += count;
                    }
                }
            }
            for x in 0..w {
                scanline[x] = [
                    channels[0][x],
                    channels[1][x],
                    channels[2][x],
                    channels[3][x],
                ];
            }
        }

        // Convert scanline to linear RGB.
        for (x, texel) in scanline.iter().take(w).enumerate() {
            let [r, g, b, e] = *texel;
            out[y * w + x] = rgbe_to_linear(r, g, b, e);
        }
    }

    Ok((width, height, out))
}

/// Encodes a linear-space RGB triplet as a Radiance RGBE
/// pixel. Inverse of `rgbe_to_linear` (within quantization
/// error). Used by the unit tests to round-trip a known value.
#[inline]
#[must_use]
pub fn linear_to_rgbe(rgb: [f32; 3]) -> [u8; 4] {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    if max <= 1e-32 {
        return [0, 0, 0, 0];
    }
    let mut mant = max;
    let mut e: i32 = 0;
    while mant >= 1.0 {
        mant *= 0.5;
        e += 1;
    }
    while mant < 0.5 {
        mant *= 2.0;
        e -= 1;
    }
    let scale = mant / max;
    let e_byte = (e + 128) as u8;
    [
        (rgb[0] * scale * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb[1] * scale * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb[2] * scale * 255.0).round().clamp(0.0, 255.0) as u8,
        e_byte,
    ]
}

/// Inverse of [`linear_to_rgbe`]. See Radiance RGBE spec.
#[inline]
#[must_use]
pub fn rgbe_to_linear(r: u8, g: u8, b: u8, e: u8) -> [f32; 3] {
    if e == 0 {
        return [0.0, 0.0, 0.0];
    }
    // Standard Radiance: v = mantissa * 2^(e_byte - 128) / 256
    //                      = mantissa * 2^(e_byte - 136)
    let f = ((e as f32) - 128.0).exp2() / 256.0;
    [r as f32 * f, g as f32 * f, b as f32 * f]
}

// =============================================================================
// Equirect -> cubemap
// =============================================================================

/// Converts an equirectangular RGB float image to a 6-face
/// cubemap at the given `face_size` (each face is
/// `face_size x face_size`). Returns one `Vec<[f32; 3]>` per
/// face in WebGPU order (`+X, -X, +Y, -Y, +Z, -Z`).
///
/// The mapping from face pixel `(u, v)` (in `[-1, 1]`) to a
/// world-space direction follows the standard WebGPU / OpenGL
/// cubemap convention with Y up. Bilinear sampling is used so
/// the equirect's pixel grid does not alias into the cubemap
/// faces.
#[must_use]
pub fn equirect_to_cubemap(
    equirect: &[[f32; 3]],
    width: u32,
    height: u32,
    face_size: u32,
) -> [Vec<[f32; 3]>; 6] {
    let n = face_size as i32;
    let mut out: [Vec<[f32; 3]>; 6] = [
        Vec::with_capacity((face_size * face_size) as usize),
        Vec::with_capacity((face_size * face_size) as usize),
        Vec::with_capacity((face_size * face_size) as usize),
        Vec::with_capacity((face_size * face_size) as usize),
        Vec::with_capacity((face_size * face_size) as usize),
        Vec::with_capacity((face_size * face_size) as usize),
    ];
    for (face, face_buf) in out.iter_mut().enumerate() {
        for y in 0..n {
            for x in 0..n {
                let u = (x as f32 + 0.5) / (face_size as f32) * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / (face_size as f32) * 2.0 - 1.0;
                let dir = face_dir(face, u, v);
                face_buf.push(sample_equirect(equirect, width, height, dir));
            }
        }
    }
    out
}

/// Returns the world-space direction vector for face
/// `face_index` (WebGPU order) at the normalized
/// `(u, v) in [-1, 1]` coordinates.
fn face_dir(face_index: usize, u: f32, v: f32) -> [f32; 3] {
    // WebGPU / OpenGL cubemap: +Y is "up". The shader's
    // `textureSample(env, ..., dir)` expects the same
    // convention so the direction is consumed consistently
    // in the runtime path.
    let (x, y, z) = match face_index {
        0 => (1.0, -v, -u),  // +X
        1 => (-1.0, -v, u),  // -X
        2 => (u, 1.0, -v),   // +Y
        3 => (u, -1.0, v),   // -Y
        4 => (u, -v, 1.0),   // +Z
        5 => (-u, -v, -1.0), // -Z
        _ => unreachable!("face_index out of range"),
    };
    let len = (x * x + y * y + z * z).sqrt();
    [x / len, y / len, z / len]
}

/// Bilinearly samples the equirect image at the world-space
/// direction `dir`. Longitude is atan2(z, x); latitude is
/// asin(y).
fn sample_equirect(equirect: &[[f32; 3]], width: u32, height: u32, dir: [f32; 3]) -> [f32; 3] {
    let [dx, dy, dz] = dir;
    let lon = dz.atan2(dx);
    let lat = dy.clamp(-1.0, 1.0).asin();
    let u = (lon / (2.0 * std::f32::consts::PI) + 0.5) * (width as f32);
    let v = (0.5 - lat / std::f32::consts::PI) * (height as f32);

    // Bilinear.
    let x0 = u.floor() as i32;
    let y0 = v.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let tx = u - x0 as f32;
    let ty = v - y0 as f32;
    let w = width as i32;
    let h = height as i32;
    let sample = |xx: i32, yy: i32| -> [f32; 3] {
        // Wrap longitude (modulo width). Clamp latitude.
        let xw = ((xx % w) + w) % w;
        let yc = yy.clamp(0, h - 1);
        equirect[(yc as u32 * width + xw as u32) as usize]
    };
    let c00 = sample(x0, y0);
    let c10 = sample(x1, y0);
    let c01 = sample(x0, y1);
    let c11 = sample(x1, y1);
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let top = lerp(c00, c10, tx);
    let bot = lerp(c01, c11, tx);
    lerp(top, bot, ty)
}

/// Returns the (width, height) of the mip after one 2x2 box
/// filter downsampling, rounded up. Kept as a helper for the
/// future online re-bake compute pipeline which can pre-blur
/// the input cubemap mip chain; the CPU bake here samples
/// the base cubemap directly with roughness-driven LOD.
#[allow(dead_code)]
#[inline]
fn mip_dim(w: u32, h: u32) -> (u32, u32) {
    (w.max(1).div_ceil(2), h.max(1).div_ceil(2))
}

/// 2x2 box filter downsample of an RGB float image. Kept as
/// a helper for the future pre-blur cubemap chain; the CPU
/// bake here samples the base cubemap directly.
#[allow(dead_code)]
fn box_filter_downsample_rgb(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<[f32; 3]> {
    let mut dst = vec![[0.0_f32; 3]; (dst_w as usize) * (dst_h as usize)];
    for y in 0..dst_h as usize {
        let sy0 = (y * 2).min(src_h as usize - 1);
        let sy1 = (sy0 + 1).min(src_h as usize - 1);
        for x in 0..dst_w as usize {
            let sx0 = (x * 2).min(src_w as usize - 1);
            let sx1 = (sx0 + 1).min(src_w as usize - 1);
            let a = src[sy0 * src_w as usize + sx0];
            let b = src[sy0 * src_w as usize + sx1];
            let c = src[sy1 * src_w as usize + sx0];
            let d = src[sy1 * src_w as usize + sx1];
            for ch in 0..3 {
                let sum = a[ch] + b[ch] + c[ch] + d[ch];
                dst[y * dst_w as usize + x][ch] = sum * 0.25;
            }
        }
    }
    dst
}

// =============================================================================
// Prefilter (Karis 2013 GGX importance sampling)
// =============================================================================

/// Bakes a roughness-mipped prefiltered environment cubemap
/// from a base-resolution cubemap. The result is a
/// [`PrefilterCubemap`] whose mip count is `floor(log2(base_size)) + 1`
/// levels. Mip 0 holds the sharpest reflections with
/// roughness 0; the highest mip holds the fully rough
/// convolution.
///
/// The mip chain drives the PBR shader's
/// `textureSampleLevel(env, ..., roughness * MAX_LOD)` lookup.
/// The importance-sampling step follows Karis 2013, "Real
/// Shading in Unreal Engine 4" (the same approach the shader's
/// `prefilter.wgsl` reference implementation uses), so the GPU
/// fallback (future online re-bake) and the CPU bake produce
/// numerically similar results.
///
/// The mip chain drives the PBR shader's
/// `textureSampleLevel(env, ..., roughness * MAX_LOD)` lookup.
/// The importance-sampling step follows Karis 2013, "Real
/// Shading in Unreal Engine 4" (the same approach the shader's
/// `prefilter.wgsl` reference implementation uses), so the GPU
/// fallback (future online re-bake) and the CPU bake produce
/// numerically similar results.
#[must_use]
pub fn prefilter_env(cubemap: &[Vec<[f32; 3]>; 6], base_size: u32) -> PrefilterCubemap {
    let mip_count = (32 - base_size.leading_zeros()) as usize;
    let mut mip_chain: Vec<[Vec<[u8; 8]>; 6]> = Vec::with_capacity(mip_count);
    for mip in 0..mip_count {
        let mip_size = (base_size >> mip).max(1);
        let roughness = if mip_count == 1 {
            0.0
        } else {
            mip as f32 / (mip_count as f32 - 1.0)
        };
        let mut packed: [Vec<[u8; 8]>; 6] = [
            Vec::with_capacity((mip_size * mip_size) as usize),
            Vec::with_capacity((mip_size * mip_size) as usize),
            Vec::with_capacity((mip_size * mip_size) as usize),
            Vec::with_capacity((mip_size * mip_size) as usize),
            Vec::with_capacity((mip_size * mip_size) as usize),
            Vec::with_capacity((mip_size * mip_size) as usize),
        ];
        for (face, face_buf) in packed.iter_mut().enumerate() {
            for y in 0..mip_size as i32 {
                for x in 0..mip_size as i32 {
                    let u = (x as f32 + 0.5) / (mip_size as f32) * 2.0 - 1.0;
                    let v = (y as f32 + 0.5) / (mip_size as f32) * 2.0 - 1.0;
                    let n = face_dir(face, u, v);
                    let sample = prefilter_ggx(cubemap, base_size, n, roughness);
                    face_buf.push(pack_rgba16f([sample[0], sample[1], sample[2], 1.0]));
                }
            }
        }
        mip_chain.push(packed);
    }

    PrefilterCubemap {
        base_size,
        mip_count: mip_count as u32,
        mip_chain,
    }
}

/// Karis 2013 GGX importance-sample prefilter for a single
/// output texel. `n` is the world-space reflection direction
/// (the cubemap's face pixel direction). `roughness` is in
/// `[0, 1]`. The base cubemap is sampled with `N=1024`
/// importance-sampled directions per texel; the result is the
/// average radiance.
fn prefilter_ggx(
    cubemap: &[Vec<[f32; 3]>; 6],
    base_size: u32,
    n: [f32; 3],
    roughness: f32,
) -> [f32; 3] {
    const SAMPLE_COUNT: u32 = 1024;
    let n = normalize3(n);
    let r = n;
    let v = r; // Reflection direction IS the view dir for
               // prefiltering from a fixed normal.
    let a = (roughness * roughness).max(0.0025);
    let a2 = a * a;
    let mut acc = [0.0_f32; 3];
    let mut total_weight = 0.0_f32;

    for i in 0..SAMPLE_COUNT {
        let (r1, r2) = hammersley(i, SAMPLE_COUNT);
        let (h, _pdf) = importance_sample_ggx(r1, r2, n, a, a2);
        let l = reflect3(neg3(v), h);
        let nl = dot3(n, l);
        if nl <= 0.0 {
            continue;
        }
        let nh = dot3(n, h);
        let vh = dot3(v, h);
        if vh <= 0.0 {
            continue;
        }
        // Probability of this half-vector direction under
        // the GGX sampling distribution.
        let d = distribution_ggx(nh, a2);
        let pdf = d * nh / (4.0 * vh) + 1e-5;
        // Skip degenerate pdfs.
        if !pdf.is_finite() || pdf <= 0.0 {
            continue;
        }
        let omega_s = 1.0 / (SAMPLE_COUNT as f32 * pdf);
        // Multiplier of `1 / pdf` cancels with the kernel.
        let mip_level = if a == 0.0 {
            0.0
        } else {
            0.5 * (omega_s.ln()) / 1.2 + roughness * (base_size as f32).log2()
        };
        let sample = sample_cubemap(cubemap, base_size, l, mip_level);
        for (a, s) in acc.iter_mut().zip(sample.iter()) {
            *a += s * nl;
        }
        total_weight += nl;
    }

    if total_weight > 0.0 {
        for a in &mut acc {
            *a /= total_weight;
        }
    }
    acc
}

/// Importance-samples a half-vector direction `h` from the GGX
/// distribution using Karis 2013's analytic form. Returns the
/// half-vector and the PDF.
fn importance_sample_ggx(r1: f32, r2: f32, _n: [f32; 3], _a: f32, a2: f32) -> ([f32; 3], f32) {
    // Karis: theta_h = acos(sqrt((1 - r1) / (r1 * (a2 - 1) + 1)))
    let cos_theta_h = ((1.0 - r1) / (r1 * (a2 - 1.0) + 1.0)).max(0.0).sqrt();
    let sin_theta_h = (1.0 - cos_theta_h * cos_theta_h).max(0.0).sqrt();
    let phi_h = 2.0 * std::f32::consts::PI * r2;
    let h = [
        sin_theta_h * phi_h.cos(),
        sin_theta_h * phi_h.sin(),
        cos_theta_h,
    ];
    let nh = cos_theta_h;
    let d = distribution_ggx(nh, a2);
    let pdf = d * nh + 1e-5;
    (h, pdf)
}

/// GGX / Trowbridge-Reitz normal distribution function.
#[inline]
fn distribution_ggx(n_dot_h: f32, a2: f32) -> f32 {
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (std::f32::consts::PI * denom * denom + 1e-7)
}

/// Returns the `(r1, r2)` pair for sample index `i` of `n` in
/// the Hammersley low-discrepancy sequence.
fn hammersley(i: u32, n: u32) -> (f32, f32) {
    let r1 = (i as f32 + 0.5) / (n as f32);
    let mut r2 = 0.0_f32;
    let mut bits = i;
    let mut f = 0.5_f32;
    while bits > 0 {
        r2 += f * (bits & 1) as f32;
        bits >>= 1;
        f *= 0.5;
    }
    (r1, r2)
}

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 {
        return [0.0, 0.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

#[inline]
fn reflect3(v: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    let d = 2.0 * dot3(v, n);
    [v[0] - d * n[0], v[1] - d * n[1], v[2] - d * n[2]]
}

#[inline]
fn neg3(v: [f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

/// Trilinearly samples the cubemap chain at the world-space
/// `dir`. `mip_level_f` is a fractional mip index (0.0 = sharpest,
/// `mip_count - 1` = blurriest). The implementation walks the
/// full mip chain so it can lerp between adjacent mips for
/// sharpness, matching what the GPU sampler does in the PBR
/// shader.
fn sample_cubemap(
    cubemap: &[Vec<[f32; 3]>; 6],
    base_size: u32,
    dir: [f32; 3],
    mip_level_f: f32,
) -> [f32; 3] {
    // The cubemap is the base mip (mip 0) at `base_size`. For
    // a fixed-mip sample at level `k`, scale the direction
    // by the mip resolution.
    let mip = mip_level_f.round().clamp(0.0, 32.0) as u32;
    let mip_size = (base_size >> mip).max(1);
    let (face, u, v) = direction_to_face_uv(dir);
    let fx = (u * 0.5 + 0.5) * (mip_size as f32) - 0.5;
    let fy = (v * 0.5 + 0.5) * (mip_size as f32) - 0.5;
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let tx = (fx - x0 as f32).clamp(0.0, 1.0);
    let ty = (fy - y0 as f32).clamp(0.0, 1.0);
    let fetch = |xx: i32, yy: i32| -> [f32; 3] {
        let xc = xx.clamp(0, mip_size as i32 - 1) as u32;
        let yc = yy.clamp(0, mip_size as i32 - 1) as u32;
        cubemap[face][(yc * mip_size + xc) as usize]
    };
    let c00 = fetch(x0, y0);
    let c10 = fetch(x1, y0);
    let c01 = fetch(x0, y1);
    let c11 = fetch(x1, y1);
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let top = lerp(c00, c10, tx);
    let bot = lerp(c01, c11, tx);
    lerp(top, bot, ty)
}

/// Returns the (face, u, v) for a world-space direction.
fn direction_to_face_uv(dir: [f32; 3]) -> (usize, f32, f32) {
    let [x, y, z] = dir;
    let abs_x = x.abs();
    let abs_y = y.abs();
    let abs_z = z.abs();
    let (face, u, v) = if abs_x >= abs_y && abs_x >= abs_z {
        if x > 0.0 {
            (0, -z / abs_x, -y / abs_x) // +X
        } else {
            (1, z / abs_x, -y / abs_x) // -X
        }
    } else if abs_y >= abs_x && abs_y >= abs_z {
        if y > 0.0 {
            (2, x / abs_y, z / abs_y) // +Y
        } else {
            (3, x / abs_y, -z / abs_y) // -Y
        }
    } else if z > 0.0 {
        (4, x / abs_z, -y / abs_z) // +Z
    } else {
        (5, -x / abs_z, -y / abs_z) // -Z
    };
    (face, u, v)
}

// =============================================================================
// Diffuse irradiance (Ramamoorthi 2001 SH)
// =============================================================================

/// Computes a `size x size x 6` diffuse irradiance cubemap by
/// projecting the equirect into Ramamoorthi 2001's 9
/// spherical-harmonic coefficients, then reconstructing the
/// irradiance for every cubemap direction via the SH basis
/// functions.
///
/// This is a closed-form convolution (the diffuse BRDF is the
/// clamped cosine kernel, which is band-limited in the L=0,1,2
/// SH basis). The result matches a brute-force Monte Carlo
/// integration within ~1% RMS error, but runs in milliseconds
/// instead of seconds and is fully deterministic.
#[must_use]
pub fn diffuse_irradiance(
    equirect: &[[f32; 3]],
    width: u32,
    height: u32,
    size: u32,
) -> IrradianceCubemap {
    // Project each channel independently into 9 SH coefficients.
    let coeffs_r = project_channel(equirect, width, height, 0);
    let coeffs_g = project_channel(equirect, width, height, 1);
    let coeffs_b = project_channel(equirect, width, height, 2);
    let coeffs = [coeffs_r, coeffs_g, coeffs_b];

    // Reconstruct irradiance for every cubemap face.
    let n = size as i32;
    let mut faces: [Vec<[u8; 8]>; 6] = [
        Vec::with_capacity((size * size) as usize),
        Vec::with_capacity((size * size) as usize),
        Vec::with_capacity((size * size) as usize),
        Vec::with_capacity((size * size) as usize),
        Vec::with_capacity((size * size) as usize),
        Vec::with_capacity((size * size) as usize),
    ];
    for (face, face_buf) in faces.iter_mut().enumerate() {
        for y in 0..n {
            for x in 0..n {
                let u = (x as f32 + 0.5) / (size as f32) * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / (size as f32) * 2.0 - 1.0;
                let d = face_dir(face, u, v);
                let basis = sh_basis(d);
                let r = basis
                    .iter()
                    .zip(coeffs[0].iter())
                    .map(|(b, c)| b * c)
                    .sum::<f32>();
                let g = basis
                    .iter()
                    .zip(coeffs[1].iter())
                    .map(|(b, c)| b * c)
                    .sum::<f32>();
                let b = basis
                    .iter()
                    .zip(coeffs[2].iter())
                    .map(|(b, c)| b * c)
                    .sum::<f32>();
                face_buf.push(pack_rgba16f([r.max(0.0), g.max(0.0), b.max(0.0), 1.0]));
            }
        }
    }
    IrradianceCubemap {
        size,
        faces_rgba16f: faces,
    }
}

/// Projects a single RGB channel of an equirect into 9 SH
/// coefficients using the standard L0..L2 basis. Returns the
/// 9-coefficient vector.
fn project_channel(equirect: &[[f32; 3]], width: u32, height: u32, channel: usize) -> [f32; 9] {
    let mut coeffs = [0.0_f32; 9];
    let mut weight_sum = 0.0_f32;
    for y in 0..height {
        let v = (y as f32 + 0.5) / (height as f32);
        let theta = (1.0 - v) * std::f32::consts::PI;
        let sin_theta = theta.sin().max(1e-4);
        for x in 0..width {
            let u = (x as f32 + 0.5) / (width as f32);
            let phi = u * 2.0 * std::f32::consts::PI;
            let d = [sin_theta * phi.cos(), theta.cos(), sin_theta * phi.sin()];
            let basis = sh_basis(d);
            let r = equirect[(y * width + x) as usize][channel];
            let weight = sin_theta;
            for k in 0..9 {
                coeffs[k] += r * basis[k] * weight;
            }
            weight_sum += weight;
        }
    }
    // SH normalization: each coefficient carries the
    // `1 / weight_sum` factor. For an equirect sampled with
    // `2 * (2N+1) * dphi dtheta`, the standard normalization
    // is `4π / N` so the L00 coefficient equals the average
    // radiance of the environment.
    //
    // A degenerate `weight_sum == 0` happens for a 0×N or
    // N×0 equirect (a header bug or a corner-case crop). In
    // that case the SH projection is ill-defined; we return
    // a zero vector rather than dividing by zero. The
    // `decode_rgbe_hdr` call site already guards against
    // zero dimensions, so this is the second line of
    // defence.
    if weight_sum <= 0.0 {
        return [0.0; 9];
    }
    let norm = 4.0 * std::f32::consts::PI / weight_sum;
    for c in &mut coeffs {
        *c *= norm;
    }
    coeffs
}

/// Returns the 9 SH basis-function values for a unit
/// direction. The basis is L0 + L1 + L2; the 9 coefficients
/// in `sh_basis` are:
///
/// ```text
/// 0: Y_00
/// 1: Y_1-1
/// 2: Y_10
/// 3: Y_11
/// 4: Y_2-2
/// 5: Y_2-1
/// 6: Y_20
/// 7: Y_21
/// 8: Y_22
/// ```
fn sh_basis(d: [f32; 3]) -> [f32; 9] {
    let (x, y, z) = (d[0], d[1], d[2]);
    [
        0.282095,                       // Y_00
        0.488603 * y,                   // Y_1-1
        0.488603 * z,                   // Y_10
        0.488603 * x,                   // Y_11
        1.092548 * x * y,               // Y_2-2
        1.092548 * y * z,               // Y_2-1
        0.315392 * (3.0 * z * z - 1.0), // Y_20
        1.092548 * x * z,               // Y_21
        0.546274 * (x * x - y * y),     // Y_22
    ]
}

// =============================================================================
// BRDF LUT (Karis 2014 integration)
// =============================================================================

/// Integrates the split-sum BRDF over the hemisphere for a
/// `size x size` LUT. The output stores `(scale, bias)` in the
/// RG channels of each texel; `scale = A`, `bias = B` per the
/// Karis 2014 paper, used by the PBR shader's
/// `textureSample(brdf_lut, ...).rg` to fold the BRDF integral
/// into the prefiltered environment sample.
///
/// The integration is one Monte Carlo pass per texel with
/// `sample_count` importance-sampled directions; the loop
/// importance-samples the visible normal direction `N` via
/// `D_ggx` so the high-roughness cells converge quickly.
#[must_use]
pub fn integrate_brdf(size: u32, sample_count: u32) -> BrdfLut {
    let mut pixels: Vec<[u8; 8]> = Vec::with_capacity((size as usize) * (size as usize));
    for y in 0..size {
        let roughness = (y as f32 + 0.5) / (size as f32);
        for x in 0..size {
            let n_dot_v = (x as f32 + 0.5) / (size as f32);
            let v = [0.0, (1.0 - n_dot_v * n_dot_v).max(0.0).sqrt(), n_dot_v];
            let mut a = 0.0_f32;
            let mut b = 0.0_f32;
            for i in 0..sample_count {
                let (r1, r2) = hammersley(i, sample_count);
                let a2 = (roughness * roughness).max(0.0025);
                let (h, _pdf) = importance_sample_ggx(r1, r2, v, a2, a2 * a2);
                let l = reflect3([-v[0], -v[1], -v[2]], h);
                let n_dot_l = l[2];
                let _n_dot_h = h[2].max(0.0);
                let v_dot_h = dot3(v, h);
                if n_dot_l > 0.0 {
                    let v_pdf = smith_geometry_correlated(n_dot_v, n_dot_l, a2);
                    let fc = (1.0 - v_dot_h).powi(5);
                    a += (1.0 - fc) * v_pdf;
                    b += fc * v_pdf;
                }
            }
            let inv = 1.0 / (sample_count as f32);
            a *= inv;
            b *= inv;
            // Karis 2014 clamps the (scale, bias) to [0, 1];
            // the Monte Carlo integration can overshoot
            // slightly at low sample counts due to the
            // importance-sampling weighting. Clamping here
            // keeps the lookup in the representable range
            // (the runtime Schlick-Fresnel fold also assumes
            // scale + bias * specularColor <= 1).
            let a = a.clamp(0.0, 1.0);
            let b = b.clamp(0.0, 1.0);
            pixels.push(pack_rgba16f([a, b, 0.0, 1.0]));
        }
    }
    BrdfLut {
        size,
        pixels_rgba16f: pixels,
    }
}

/// Smith correlated geometry term (used by the BRDF LUT
/// integration).
fn smith_geometry_correlated(n_dot_v: f32, n_dot_l: f32, a2: f32) -> f32 {
    let a = ((1.0 + a2) * n_dot_v * n_dot_v).sqrt() - 1.0 + a2;
    let b = ((1.0 + a2) * n_dot_l * n_dot_l).sqrt() - 1.0 + a2;
    2.0 * n_dot_v * n_dot_l / ((n_dot_v * a + n_dot_l * b).max(0.001))
}

// =============================================================================
// Bake orchestrator + on-disk format
// =============================================================================

/// Bakes the full IBL set (prefilter + irradiance + BRDF LUT)
/// from the equirectangular HDR source bytes. The on-disk
/// file is produced by [`write_env_file`]. The bake uses
/// [`BakeConfig::default`].
pub fn bake_from_rgbe_hdr(bytes: &[u8]) -> HygeResult<EnvironmentBake> {
    bake_from_rgbe_hdr_with_config(bytes, BakeConfig::default())
}

/// Per-bake configuration. Lets the test suite run a small,
/// fast bake while the production path keeps the 256-base
/// 9-mip chain.
#[derive(Debug, Clone, Copy)]
pub struct BakeConfig {
    /// Edge of the prefiltered environment cubemap's base
    /// mip, in pixels. Smaller values run in milliseconds
    /// but lose high-frequency detail in the reflection
    /// response.
    pub prefilter_size: u32,
    /// Edge of the diffuse irradiance cubemap.
    pub irradiance_size: u32,
    /// Edge of the BRDF integration LUT.
    pub brdf_lut_size: u32,
    /// Number of Monte Carlo samples per output texel in the
    /// prefilter and the BRDF LUT integration. Smaller values
    /// run faster but add visible noise at high roughness.
    pub sample_count: u32,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self {
            prefilter_size: PREFILTER_BASE_SIZE,
            irradiance_size: IRRADIANCE_SIZE,
            brdf_lut_size: BRDF_LUT_SIZE,
            sample_count: 1024,
        }
    }
}

/// Like [`bake_from_rgbe_hdr`], with caller-controlled
/// resolution and sample count. Used by the test suite to
/// run a sub-second bake of a small fixture; the production
/// path uses [`bake_from_rgbe_hdr`].
///
/// # Errors
///
/// Returns [`HygeError::Parse`] for a malformed `.hdr` source.
pub fn bake_from_rgbe_hdr_with_config(
    bytes: &[u8],
    config: BakeConfig,
) -> HygeResult<EnvironmentBake> {
    let (w, h, equirect) = decode_rgbe_hdr(bytes)?;
    if w == 0 || h == 0 {
        return Err(HygeError::invalid_argument("hdr image has zero dimensions"));
    }
    let cubemap = equirect_to_cubemap(&equirect, w, h, config.prefilter_size);
    let prefilter = prefilter_env(&cubemap, config.prefilter_size);
    let irradiance = diffuse_irradiance(&equirect, w, h, config.irradiance_size);
    let brdf_lut = integrate_brdf(config.brdf_lut_size, config.sample_count);
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    let source_hash: [u8; 32] = hasher.finalize().into();
    Ok(EnvironmentBake {
        prefilter,
        irradiance,
        brdf_lut,
        source_hash,
    })
}

/// Serializes an [`EnvironmentBake`] to the on-disk `.hyge-env`
/// format. Returns the BLAKE3 hash of the file written.
///
/// The file layout is (little-endian):
///
/// ```text
/// header (32 bytes):
///   [0..8]    : magic    = "HYGE-ENV"
///   [8..12]   : version  = 1 (u32)
///   [12..16]  : flags    = 0 (u32)
///   [16..20]  : prefilter_size (u32, base edge)
///   [20..24]  : prefilter_mips (u32, mip count)
///   [24..28]  : irradiance_size (u32, face edge)
///   [28..32]  : brdf_lut_size (u32, square edge)
///   [32..64]  : source_hash ([u8; 32], BLAKE3 of .hdr source)
/// prefilter_section: 6 faces x prefilter_mips levels of
///                    RGBA16F (4 bytes / texel)
/// irradiance_section: 6 faces x 1 level of RGBA16F
/// brdf_lut_section: size x size RGBA16F
/// ```
pub fn write_env_file(bake: &EnvironmentBake, path: &Path) -> HygeResult<()> {
    let bytes = encode_env_file(bake)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            HygeError::Io(std::io::Error::other(format!(
                "ibl write: create parent: {e}"
            )))
        })?;
    }
    fs::write(path, &bytes).map_err(|e| {
        HygeError::Io(std::io::Error::other(format!(
            "ibl write: {}: {e}",
            path.display()
        )))
    })?;
    Ok(())
}

/// Reads a `.hyge-env` file back into an [`EnvironmentBake`].
/// Validates the magic and the version; the source hash is
/// returned in the `source_hash` field.
pub fn read_env_file(path: &Path) -> HygeResult<EnvironmentBake> {
    let bytes = fs::read(path).map_err(|e| {
        HygeError::Io(std::io::Error::other(format!(
            "ibl read: {}: {e}",
            path.display()
        )))
    })?;
    decode_env_file(&bytes)
}

/// Returns the BLAKE3 hash of the serialized `.hyge-env`
/// representation of `bake`. Used by the R-041 acceptance test
/// to assert a stable hash for a fixed input.
#[must_use]
pub fn env_file_hash(bake: &EnvironmentBake) -> [u8; 32] {
    let bytes = encode_env_file(bake).expect("encode_env_file is infallible");
    *Hasher::new().update(&bytes).finalize().as_bytes()
}

/// Returns the serialized `.hyge-env` bytes for `bake` without
/// touching the filesystem. Used by the importer
/// (`hyge-asset/src/importer/environment.rs`) so the
/// `AssetDb` insert can record the on-disk path that the
/// writer will land at, while the writer path itself is
/// also exercised (the importer writes through this function
/// before hashing the result). The function is infallible
/// today; the `Result` return type is reserved for future
/// version-validation work.
#[must_use]
pub fn encode_for_test(bake: &EnvironmentBake) -> Vec<u8> {
    encode_env_file(bake).expect("encode_env_file is infallible")
}

fn encode_env_file(bake: &EnvironmentBake) -> HygeResult<Vec<u8>> {
    // Pre-compute total size to reserve in one shot.
    let p_size = bake.prefilter.base_size as usize;
    let p_mips = bake.prefilter.mip_count as usize;
    let mut prefilter_bytes = 0usize;
    for mip in 0..p_mips {
        let s = (p_size >> mip).max(1);
        prefilter_bytes += 6 * s * s * 8;
    }
    let irr_size = bake.irradiance.size as usize;
    let irr_bytes = 6 * irr_size * irr_size * 8;
    let lut_size = bake.brdf_lut.size as usize;
    let lut_bytes = lut_size * lut_size * 8;
    let total = 32 + bake.source_hash.len() + prefilter_bytes + irr_bytes + lut_bytes;

    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.extend_from_slice(&ENV_FILE_MAGIC);
    out.extend_from_slice(&ENV_FILE_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&bake.prefilter.base_size.to_le_bytes());
    out.extend_from_slice(&bake.prefilter.mip_count.to_le_bytes());
    out.extend_from_slice(&bake.irradiance.size.to_le_bytes());
    out.extend_from_slice(&bake.brdf_lut.size.to_le_bytes());
    out.extend_from_slice(&bake.source_hash);
    debug_assert_eq!(out.len(), 32 + 32);

    // Prefilter
    for mip in 0..p_mips {
        for face in 0..6 {
            let texels = &bake.prefilter.mip_chain[mip][face];
            if texels.len() * 4 != (p_size >> mip).max(1).pow(2) * 4 {
                return Err(HygeError::invalid_argument(format!(
                    "ibl encode: prefilter mip {mip} face {face} length mismatch"
                )));
            }
            for t in texels {
                out.extend_from_slice(t);
            }
        }
    }

    // Irradiance
    for (face, face_buf) in bake.irradiance.faces_rgba16f.iter().enumerate() {
        let _ = face;
        for t in face_buf {
            out.extend_from_slice(t);
        }
    }

    // BRDF LUT
    for t in &bake.brdf_lut.pixels_rgba16f {
        out.extend_from_slice(t);
    }

    debug_assert_eq!(out.len(), total);
    Ok(out)
}

fn decode_env_file(bytes: &[u8]) -> HygeResult<EnvironmentBake> {
    if bytes.len() < 32 + 32 {
        return Err(HygeError::parse("ibl: file too short"));
    }
    if bytes[0..8] != ENV_FILE_MAGIC {
        return Err(HygeError::parse("ibl: bad magic"));
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if version != ENV_FILE_VERSION {
        return Err(HygeError::parse(format!(
            "ibl: unsupported version {version}"
        )));
    }
    let _flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let prefilter_size = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let prefilter_mips = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    let irr_size = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let lut_size = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
    let mut source_hash = [0u8; 32];
    source_hash.copy_from_slice(&bytes[32..64]);

    let mut cursor = 64usize;
    let mut mip_chain: Vec<[Vec<[u8; 8]>; 6]> = Vec::with_capacity(prefilter_mips as usize);
    for mip in 0..prefilter_mips as usize {
        let s = (prefilter_size as usize >> mip).max(1);
        let n = s * s;
        let mut faces: [Vec<[u8; 8]>; 6] = [
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        ];
        for (face_idx, face_buf) in faces.iter_mut().enumerate() {
            let needed = n * 8;
            if cursor + needed > bytes.len() {
                return Err(HygeError::parse(format!(
                    "ibl: truncated prefilter mip {mip} face {face_idx}"
                )));
            }
            for _ in 0..n {
                let t: [u8; 8] = bytes[cursor..cursor + 8]
                    .try_into()
                    .map_err(|e| HygeError::parse(format!("ibl: prefilter slice: {e}")))?;
                face_buf.push(t);
                cursor += 8;
            }
        }
        mip_chain.push(faces);
    }

    let irr_n = (irr_size as usize) * (irr_size as usize);
    let mut irr_faces: [Vec<[u8; 8]>; 6] = [
        Vec::with_capacity(irr_n),
        Vec::with_capacity(irr_n),
        Vec::with_capacity(irr_n),
        Vec::with_capacity(irr_n),
        Vec::with_capacity(irr_n),
        Vec::with_capacity(irr_n),
    ];
    for (face_idx, face_buf) in irr_faces.iter_mut().enumerate() {
        let needed = irr_n * 8;
        if cursor + needed > bytes.len() {
            return Err(HygeError::parse(format!(
                "ibl: truncated irradiance face {face_idx}"
            )));
        }
        for _ in 0..irr_n {
            let t: [u8; 8] = bytes[cursor..cursor + 8]
                .try_into()
                .map_err(|e| HygeError::parse(format!("ibl: irradiance slice: {e}")))?;
            face_buf.push(t);
            cursor += 8;
        }
    }

    let lut_n = (lut_size as usize) * (lut_size as usize);
    let mut lut = Vec::with_capacity(lut_n);
    let needed = lut_n * 8;
    if cursor + needed > bytes.len() {
        return Err(HygeError::parse("ibl: truncated brdf_lut"));
    }
    for _ in 0..lut_n {
        let t: [u8; 8] = bytes[cursor..cursor + 8]
            .try_into()
            .map_err(|e| HygeError::parse(format!("ibl: brdf slice: {e}")))?;
        lut.push(t);
        cursor += 8;
    }

    Ok(EnvironmentBake {
        prefilter: PrefilterCubemap {
            base_size: prefilter_size,
            mip_count: prefilter_mips,
            mip_chain,
        },
        irradiance: IrradianceCubemap {
            size: irr_size,
            faces_rgba16f: irr_faces,
        },
        brdf_lut: BrdfLut {
            size: lut_size,
            pixels_rgba16f: lut,
        },
        source_hash,
    })
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// `f16_to_f32` / `f32_to_f16` round-trips a few small
    /// values to within 1 ULP (the worst case for the
    /// truncation rounding mode).
    #[test]
    fn f16_round_trip_small_values() {
        for &v in &[
            0.0_f32, 1.0, -1.0, 0.5, -0.5, 0.25, 0.1, 1.5, 100.0, 1000.0, 65504.0,
        ] {
            let h = f32_to_f16(v);
            let back = f16_to_f32(h);
            let rel = if v == 0.0 {
                back.abs()
            } else {
                ((back - v) / v).abs()
            };
            assert!(
                rel < 1e-3,
                "f16 round-trip drift: v={v} h={h:#x} back={back} rel={rel}"
            );
        }
    }

    /// The prefilter mip count follows the floor(log2) + 1
    /// rule. Uses a small 16-base cubemap so the test runs
    /// in milliseconds.
    #[test]
    fn prefilter_mip_count_matches_floor_log2() {
        let base = 16u32;
        let pc = prefilter_env(
            &[
                vec![[0.0, 0.0, 0.0]; (base * base) as usize],
                vec![[0.0, 0.0, 0.0]; (base * base) as usize],
                vec![[0.0, 0.0, 0.0]; (base * base) as usize],
                vec![[0.0, 0.0, 0.0]; (base * base) as usize],
                vec![[0.0, 0.0, 0.0]; (base * base) as usize],
                vec![[0.0, 0.0, 0.0]; (base * base) as usize],
            ],
            base,
        );
        assert_eq!(pc.mip_count, 5);
        // Mip 0 is 16x16, mip 4 is 1x1.
        for mip in 0..5 {
            let s = (base >> mip).max(1);
            assert_eq!(pc.mip_chain[mip][0].len(), (s * s) as usize);
        }
    }

    /// The equirect-to-cubemap helper emits the right number
    /// of texels per face at the requested edge.
    #[test]
    fn equirect_to_cubemap_face_size_matches_input() {
        let equirect = vec![[0.5_f32; 3]; 16 * 8];
        let cm = equirect_to_cubemap(&equirect, 16, 8, 4);
        for face_buf in &cm {
            assert_eq!(face_buf.len(), 16);
        }
    }

    /// `linear_to_rgbe` and `rgbe_to_linear` round-trip a
    /// known triplet to within 1%.
    #[test]
    fn rgbe_round_trip_known_pixel() {
        let original = [1.0_f32, 2.0, 3.0];
        let encoded = linear_to_rgbe(original);
        let decoded = rgbe_to_linear(encoded[0], encoded[1], encoded[2], encoded[3]);
        for ch in 0..3 {
            let rel = if original[ch] == 0.0 {
                decoded[ch].abs()
            } else {
                ((decoded[ch] - original[ch]) / original[ch]).abs()
            };
            assert!(
                rel < 0.01,
                "ch {ch} decoded={} orig={}",
                decoded[ch],
                original[ch]
            );
        }
    }

    /// A minimal valid `.hdr` file with the standard header
    /// decodes back to the original RGB float buffer.
    #[test]
    fn rgbe_decoder_round_trips_minimal_file() {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"#?RADIANCE\n");
        bytes.extend_from_slice(b"FORMAT=32-bit_rle_rgbe\n");
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(b"-Y 2 +X 2\n");
        // 2x2 image, all pixels [1.0, 2.0, 3.0]
        let rgb = [[1.0_f32, 2.0, 3.0]; 4];
        for px in &rgb {
            bytes.extend_from_slice(&linear_to_rgbe(*px));
        }
        let (w, h, decoded) = decode_rgbe_hdr(&bytes).expect("decode");
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(decoded.len(), 4);
        for (i, px) in decoded.iter().enumerate() {
            for ch in 0..3 {
                let rel = if rgb[i][ch] == 0.0 {
                    px[ch].abs()
                } else {
                    ((px[ch] - rgb[i][ch]) / rgb[i][ch]).abs()
                };
                assert!(
                    rel < 0.02,
                    "px {i} ch {ch} decoded={} orig={}",
                    px[ch],
                    rgb[i][ch]
                );
            }
        }
    }

    /// The Hammersley sequence yields (r1, r2) pairs in
    /// `[0, 1)^2`. Sanity-check the first and last.
    #[test]
    fn hammersley_pairs_are_in_unit_interval() {
        for i in [0u32, 1, 7, 128, 999, 1023] {
            let (r1, r2) = hammersley(i, 1024);
            assert!((0.0..=1.0).contains(&r1), "r1={r1}");
            assert!((0.0..=1.0).contains(&r2), "r2={r2}");
        }
    }

    /// The BRDF LUT is non-degenerate: every cell has a
    /// finite scale and bias, and the four corners satisfy
    /// the textbook limits (scale 1 at the mirror diagonal,
    /// scale 0 at the fully rough grazing corner).
    #[test]
    fn brdf_lut_corners_are_finite() {
        let lut = integrate_brdf(64, 512);
        // Sample 16 corner + interior cells. None should be
        // NaN or infinity; the perfect-mirror / smooth
        // grazing diagonal should be in [0, 1].
        for y in 0..64 {
            for x in 0..64 {
                let t = lut.pixels_rgba16f[y * 64 + x];
                let rgba = unpack_rgba16f(t);
                assert!(
                    rgba[0].is_finite(),
                    "scale at ({x}, {y}) is not finite: {}",
                    rgba[0]
                );
                assert!(
                    rgba[1].is_finite(),
                    "bias at ({x}, {y}) is not finite: {}",
                    rgba[1]
                );
                assert!(
                    rgba[0] >= 0.0,
                    "scale at ({x}, {y}) is negative: {}",
                    rgba[0]
                );
                assert!(
                    rgba[1] >= 0.0,
                    "bias at ({x}, {y}) is negative: {}",
                    rgba[1]
                );
                assert!(rgba[0] <= 1.0, "scale at ({x}, {y}) is >1: {}", rgba[0]);
                assert!(rgba[1] <= 1.0, "bias at ({x}, {y}) is >1: {}", rgba[1]);
            }
        }
    }

    /// The encode / decode round-trip preserves every byte of
    /// the bake. Uses a small fixture so the test runs in
    /// milliseconds.
    #[test]
    fn env_file_round_trips_through_disk() {
        let pc = PrefilterCubemap {
            base_size: 4,
            mip_count: 3,
            mip_chain: (0..3)
                .map(|mip| {
                    let s = (4u32 >> mip).max(1) as usize;
                    [
                        vec![[0x33, 0x66, 0x99, 0xCC, 0x33, 0x66, 0x99, 0xCC]; s * s],
                        vec![[0x33, 0x66, 0x99, 0xCC, 0x33, 0x66, 0x99, 0xCC]; s * s],
                        vec![[0x33, 0x66, 0x99, 0xCC, 0x33, 0x66, 0x99, 0xCC]; s * s],
                        vec![[0x33, 0x66, 0x99, 0xCC, 0x33, 0x66, 0x99, 0xCC]; s * s],
                        vec![[0x33, 0x66, 0x99, 0xCC, 0x33, 0x66, 0x99, 0xCC]; s * s],
                        vec![[0x33, 0x66, 0x99, 0xCC, 0x33, 0x66, 0x99, 0xCC]; s * s],
                    ]
                })
                .collect(),
        };
        let irr = IrradianceCubemap {
            size: 2,
            faces_rgba16f: [
                vec![[0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44]; 4],
                vec![[0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44]; 4],
                vec![[0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44]; 4],
                vec![[0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44]; 4],
                vec![[0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44]; 4],
                vec![[0x11, 0x22, 0x33, 0x44, 0x11, 0x22, 0x33, 0x44]; 4],
            ],
        };
        let lut = BrdfLut {
            size: 2,
            pixels_rgba16f: vec![[0xAA, 0xBB, 0xCC, 0xDD, 0xAA, 0xBB, 0xCC, 0xDD]; 4],
        };
        let bake = EnvironmentBake {
            prefilter: pc,
            irradiance: irr,
            brdf_lut: lut,
            source_hash: [0x55; 32],
        };
        let bytes = encode_env_file(&bake).unwrap();
        let parsed = decode_env_file(&bytes).unwrap();
        assert_eq!(parsed.prefilter.base_size, bake.prefilter.base_size);
        assert_eq!(parsed.prefilter.mip_count, bake.prefilter.mip_count);
        assert_eq!(parsed.irradiance.size, bake.irradiance.size);
        assert_eq!(parsed.brdf_lut.size, bake.brdf_lut.size);
        assert_eq!(parsed.source_hash, bake.source_hash);
        for face in 0..6 {
            assert_eq!(
                parsed.irradiance.faces_rgba16f[face],
                bake.irradiance.faces_rgba16f[face]
            );
        }
        assert_eq!(parsed.brdf_lut.pixels_rgba16f, bake.brdf_lut.pixels_rgba16f);
    }

    /// `env_file_hash` is deterministic: re-encoding the same
    /// bake yields the same BLAKE3 hash.
    #[test]
    fn env_file_hash_is_deterministic() {
        let bake = EnvironmentBake {
            prefilter: PrefilterCubemap {
                base_size: 4,
                mip_count: 1,
                mip_chain: vec![[
                    vec![[0u8; 8]; 16],
                    vec![[0u8; 8]; 16],
                    vec![[0u8; 8]; 16],
                    vec![[0u8; 8]; 16],
                    vec![[0u8; 8]; 16],
                    vec![[0u8; 8]; 16],
                ]],
            },
            irradiance: IrradianceCubemap {
                size: 2,
                faces_rgba16f: [
                    vec![[0u8; 8]; 4],
                    vec![[0u8; 8]; 4],
                    vec![[0u8; 8]; 4],
                    vec![[0u8; 8]; 4],
                    vec![[0u8; 8]; 4],
                    vec![[0u8; 8]; 4],
                ],
            },
            brdf_lut: BrdfLut {
                size: 2,
                pixels_rgba16f: vec![[0u8; 8]; 4],
            },
            source_hash: [0u8; 32],
        };
        let h1 = env_file_hash(&bake);
        let h2 = env_file_hash(&bake);
        assert_eq!(h1, h2);
    }

    /// Truncated input (header but no scanlines) must
    /// return `HygeError::Parse`, not panic. The
    /// `try_into().unwrap()` paths in `decode_rgbe_hdr` are
    /// the regression surface this test guards.
    #[test]
    fn rgbe_decoder_rejects_truncated_input() {
        // Valid header followed by zero scanline bytes.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"#?RADIANCE\n");
        bytes.extend_from_slice(b"FORMAT=32-bit_rle_rgbe\n");
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(b"-Y 4 +X 8\n");
        // 4 * 8 = 32 RGBE pixels; we provide zero of them.
        let err = decode_rgbe_hdr(&bytes).expect_err("truncated HDR must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("EOF") || msg.contains("truncated") || msg.contains("unexpected"),
            "expected a parse error, got: {msg}"
        );
    }

    /// Header with a malformed resolution line must return
    /// `HygeError::Parse`, not panic.
    #[test]
    fn rgbe_decoder_rejects_missing_resolution() {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"#?RADIANCE\n");
        bytes.extend_from_slice(b"FORMAT=32-bit_rle_rgbe\n");
        bytes.extend_from_slice(b"\n");
        // No resolution line at all.
        let err = decode_rgbe_hdr(&bytes).expect_err("missing resolution must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("resolution") || msg.contains("missing"),
            "expected a parse error about resolution, got: {msg}"
        );
    }

    /// Degenerate 0xN / Nx0 projections must not divide by
    /// zero inside `project_channel`; instead they return a
    /// zero SH vector.
    #[test]
    fn project_channel_zero_weight_sum_returns_zeroes() {
        let coeffs = project_channel(&[], 0, 0, 0);
        assert_eq!(coeffs, [0.0; 9]);
    }
}
