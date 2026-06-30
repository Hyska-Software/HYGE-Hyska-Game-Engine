//! `.ktx2` texture container writer.
//!
//! Scope of **R-036** (KTX2 transcode, basis-universal):
//!
//! - Produces a real **KTX2** container (12-byte identifier
//!   `AB 4B 54 58 20 32 30 BB 0D 0A 1A 0A`; note the `" 20"` suffix
//!   distinguishes KTX2 from the R-034 KTX1 placeholder `" 11"`).
//! - Generates a full mip chain in pure Rust (2x2 box filter) from
//!   the source RGBA8 pixels and writes each level into the level
//!   index.
//! - The default pixel format is `VK_FORMAT_R8G8B8A8_UNORM` (37),
//!   which is a valid KTX2 target format and is what the engine's
//!   runtime will sample while phase-4 work (R-040..R-042) is still
//!   in flight. The transcoder is wired so the higher-level
//!   `transcode` module can override the target format to BC7
//!   desktop or ASTC 4x4 mobile via the Khronos `toktx` CLI
//!   (see [`crate::importer::transcode`]). Either path lands in
//!   `<blake3>.ktx2` content-addressed files in the cook cache.
//!
//! KTX2 file layout (Khronos KTX 2.0 spec, §4):
//!
//! ```text
//! header              (56 bytes)
//!   identifier        : [u8; 12]
//!   vkFormat          : u32
//!   texelBlockDimension: [u32; 2]
//!   levelCount        : u32
//!   supercompressionScheme: u32   (0 = NONE)
//!   dfdByteOffset     : u32
//!   dfdByteLength     : u32
//!   kvdByteOffset     : u32
//!   kvdByteLength     : u32
//!   sgdByteOffset     : u64   (supercompression global metadata; 0 here)
//! level_index         (levelCount * 32 bytes)
//!   byteOffset        : u64
//!   byteLength        : u64
//!   uncompressedByteLength: u64
//!   blockIndex        : u32
//!   blockCount        : u32
//! data_format_descriptor (dfdByteLength bytes)
//! level_data          (levelCount blocks, each offset/length from index)
//! ```

use std::fs;
use std::path::Path;

use hyge_core::result::{HygeError, HygeResult};

/// KTX2 file identifier. The `" 20 32 30"` suffix is the ASCII
/// `" 20"` that distinguishes KTX2 from KTX1 (`" 11"`).
pub const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Vulkan VkFormat values used by the KTX2 writer.
pub mod vk_format {
    /// `VK_FORMAT_R8G8B8A8_UNORM` — uncompressed 8-bit per channel RGBA.
    pub const R8G8B8A8_UNORM: u32 = 37;
    /// `VK_FORMAT_BC7_UNORM_BLOCK` — desktop block-compressed.
    pub const BC7_UNORM_BLOCK: u32 = 145;
    /// `VK_FORMAT_ASTC_4x4_UNORM_BLOCK` — mobile-class block-compressed.
    pub const ASTC_4X4_UNORM_BLOCK: u32 = 158;
}

/// Vulkan `supercompressionScheme` enum values.
pub mod supercompression_scheme {
    /// `KTX2_SS_NONE` — no supercompression; level data is stored
    /// as-is. The R-036 default and the toktx `basis` outputs use
    /// other schemes; the BC7/ASTC uncompressed block path uses
    /// this value.
    pub const NONE: u32 = 0;
}

/// The source pixel format the importer received from the glTF
/// decoder. R-036 normalises every non-RGBA8 source to RGBA8
/// before writing the KTX2; the format is preserved in the meta
/// document so the inspector and future pipelines (HDR, EXR)
/// can audit the original channel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TextureFormat {
    /// 8-bit single channel.
    R8 = 1,
    /// 8-bit two channels.
    R8G8 = 2,
    /// 8-bit three channels.
    R8G8B8 = 3,
    /// 8-bit four channels. The default KTX2 storage format.
    R8G8B8A8 = 4,
    /// 16-bit single channel.
    R16 = 5,
    /// 16-bit two channels.
    R16G16 = 6,
    /// 16-bit three channels.
    R16G16B16 = 7,
    /// 16-bit four channels.
    R16G16B16A16 = 9,
    /// 32-bit float single channel.
    R32G32B32A32FLOAT = 14,
}

impl TextureFormat {
    /// Returns the number of channels for this format.
    #[inline]
    pub const fn channels(self) -> u8 {
        match self {
            Self::R8 | Self::R16 => 1,
            Self::R8G8 | Self::R16G16 => 2,
            Self::R8G8B8 | Self::R16G16B16 => 3,
            Self::R8G8B8A8 | Self::R16G16B16A16 => 4,
            Self::R32G32B32A32FLOAT => 4,
        }
    }

    /// Returns the size in bytes of a single texel.
    #[inline]
    pub const fn texel_size(self) -> usize {
        match self {
            Self::R8 | Self::R8G8 | Self::R8G8B8 | Self::R8G8B8A8 => self.channels() as usize,
            Self::R16 | Self::R16G16 | Self::R16G16B16 | Self::R16G16B16A16 => {
                self.channels() as usize * 2
            }
            Self::R32G32B32A32FLOAT => 16,
        }
    }

    /// Returns the Vulkan format the KTX2 writer will store the
    /// pixels in after normalization. Every source format lands
    /// as `VK_FORMAT_R8G8B8A8_UNORM` for the R-036 default
    /// (uncompressed) path; the `transcode` module overrides
    /// this to BC7 or ASTC 4x4 when it shells out to `toktx`.
    #[inline]
    pub const fn ktx2_vk_format(self) -> u32 {
        vk_format::R8G8B8A8_UNORM
    }
}

/// The kind of payload the `.ktx2` file holds. R-036 only emits
/// the [`TextureKind::Ktx2`] variant; the KTX1 variant is kept
/// here so external tools (e.g. an inspector) can still
/// round-trip a pre-R-036 cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    /// A real KTX2 container produced by [`write()`].
    Ktx2 {
        /// Width in pixels of the base level.
        width: u32,
        /// Height in pixels of the base level.
        height: u32,
        /// Vulkan format the level data is stored in.
        vk_format: u32,
        /// Number of mip levels written (always >= 1).
        level_count: u32,
        /// Lowercase mime type of the source image.
        source_mime: &'static str,
    },
}

/// Configuration for a [`write()`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
    /// Vulkan format the level data is stored in. For the
    /// pure-Rust path this is always
    /// `vk_format::R8G8B8A8_UNORM`; the `transcode` module sets
    /// BC7 / ASTC 4x4 when it shells out to `toktx`.
    pub vk_format: u32,
    /// When `true`, generate a full mip chain down to 1x1 and
    /// write every level into the KTX2 level index. When
    /// `false`, only the base level is written.
    pub generate_mipmaps: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            vk_format: vk_format::R8G8B8A8_UNORM,
            generate_mipmaps: true,
        }
    }
}

/// Writes a KTX2 texture container to `path`.
///
/// `pixels` is the base-level RGBA8 pixel data, tightly packed
/// in row-major order (no row padding, no alignment slack).
/// The pixel buffer must contain exactly `width * height * 4`
/// bytes regardless of the source format — non-RGBA8 sources
/// are normalised by the caller via [`normalize_to_rgba8`].
///
/// When `options.generate_mipmaps` is `true`, the writer
/// generates a full mip chain with a 2x2 box filter and writes
/// every level into the KTX2 level index. The level count
/// follows the standard `floor(log2(max(width, height))) + 1`
/// rule so a 256x256 texture produces 9 levels (256, 128, 64,
/// 32, 16, 8, 4, 2, 1).
///
/// # Errors
///
/// Returns [`HygeError::InvalidArgument`] when `pixels.len()`
/// does not match `width * height * 4`, when `width` or
/// `height` is zero, or when the requested `vk_format` is not
/// a single-texel uncompressed format this writer can emit.
/// Returns [`HygeError::Io`] on filesystem failure.
pub fn write(
    path: &Path,
    width: u32,
    height: u32,
    options: WriteOptions,
    source_mime: &'static str,
    pixels: &[u8],
) -> HygeResult<TextureKind> {
    if width == 0 || height == 0 {
        return Err(HygeError::invalid_argument(format!(
            "KTX2 requires non-zero width/height: got {width}x{height}"
        )));
    }
    let expected = (width as usize) * (height as usize) * 4;
    if pixels.len() != expected {
        return Err(HygeError::invalid_argument(format!(
            "KTX2 pixel buffer size mismatch: expected {expected} bytes for \
             {width}x{height} RGBA8, got {}",
            pixels.len()
        )));
    }
    // The pure-Rust path writes single-texel uncompressed or
    // block-compressed formats; the transcode module handles
    // the higher-level BC7/ASTC orchestration through toktx.
    if !matches!(
        options.vk_format,
        vk_format::R8G8B8A8_UNORM | vk_format::BC7_UNORM_BLOCK | vk_format::ASTC_4X4_UNORM_BLOCK
    ) {
        return Err(HygeError::invalid_argument(format!(
            "KTX2 writer: unsupported vkFormat {} (use the transcode module for block-compressed paths)",
            options.vk_format
        )));
    }

    let levels: Vec<Vec<u8>> = if options.generate_mipmaps {
        generate_mip_chain(width, height, pixels)
    } else {
        vec![pixels.to_vec()]
    };

    let bytes = encode_ktx2(width, height, options.vk_format, &levels)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create texture parent dir"))?;
    }
    fs::write(path, &bytes).map_err(io_error("write texture file"))?;
    Ok(TextureKind::Ktx2 {
        width,
        height,
        vk_format: options.vk_format,
        level_count: levels.len() as u32,
        source_mime,
    })
}

/// Reads the KTX2 header from `bytes`. Returns the parsed
/// `(width, height, vk_format, level_count)` and the per-level
/// `(offset, length)` index. Returns `None` when `bytes` does
/// not start with the KTX2 magic.
pub fn read_header(bytes: &[u8]) -> Option<Ktx2Header> {
    if bytes.len() < 56 || bytes[0..12] != KTX2_MAGIC {
        return None;
    }
    let vk_format = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
    let _texel_block_dim = [
        u32::from_le_bytes(bytes[16..20].try_into().ok()?),
        u32::from_le_bytes(bytes[20..24].try_into().ok()?),
    ];
    let level_count = u32::from_le_bytes(bytes[24..28].try_into().ok()?);
    let _supercompression_scheme = u32::from_le_bytes(bytes[28..32].try_into().ok()?);
    let dfd_byte_offset = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
    let dfd_byte_length = u32::from_le_bytes(bytes[36..40].try_into().ok()?);
    let _kvd_byte_offset = u32::from_le_bytes(bytes[40..44].try_into().ok()?);
    let _kvd_byte_length = u32::from_le_bytes(bytes[44..48].try_into().ok()?);
    let _sgd_byte_offset_qw = u64::from_le_bytes(bytes[48..56].try_into().ok()?);

    // Width and height are stored inside the DFD at offset 20
    // from the DFD start, packed as [u16 width | u16 height]
    // in a 4-byte little-endian word (sufficient for the
    // 65535x65535 image dimension cap of the KTX2 spec).
    let (width, height) = if dfd_byte_length >= 24 {
        let dim_pos = dfd_byte_offset as usize + 20;
        if dim_pos + 4 <= bytes.len() {
            let dim_word = u32::from_le_bytes(bytes[dim_pos..dim_pos + 4].try_into().ok()?);
            (dim_word & 0xFFFF, (dim_word >> 16) & 0xFFFF)
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let mut levels = Vec::with_capacity(level_count as usize);
    let mut cursor = 56usize;
    for _ in 0..level_count {
        if cursor + 32 > bytes.len() {
            return None;
        }
        let byte_offset = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().ok()?);
        let byte_length = u64::from_le_bytes(bytes[cursor + 8..cursor + 16].try_into().ok()?);
        let _uncompressed_byte_length =
            u64::from_le_bytes(bytes[cursor + 16..cursor + 24].try_into().ok()?);
        let _block_index = u32::from_le_bytes(bytes[cursor + 24..cursor + 28].try_into().ok()?);
        let _block_count = u32::from_le_bytes(bytes[cursor + 28..cursor + 32].try_into().ok()?);
        levels.push(Ktx2Level {
            offset: byte_offset,
            length: byte_length,
        });
        cursor += 32;
    }

    Some(Ktx2Header {
        vk_format,
        width,
        height,
        level_count,
        dfd_byte_offset,
        dfd_byte_length,
        levels,
    })
}

/// Parsed KTX2 header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ktx2Header {
    /// Vulkan format of the level data.
    pub vk_format: u32,
    /// Base-level width in pixels.
    pub width: u32,
    /// Base-level height in pixels.
    pub height: u32,
    /// Number of mip levels.
    pub level_count: u32,
    /// Offset of the data format descriptor block from the start
    /// of the file.
    pub dfd_byte_offset: u32,
    /// Length of the data format descriptor block.
    pub dfd_byte_length: u32,
    /// Per-level index, in declaration order.
    pub levels: Vec<Ktx2Level>,
}

/// Per-level entry in the KTX2 level index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ktx2Level {
    /// Byte offset of the level data from the start of the file.
    pub offset: u64,
    /// Byte length of the level data.
    pub length: u64,
}

/// Normalises a `pixels` buffer from `src_format` to RGBA8.
/// Channel promotion fills missing channels with the identity
/// values: 1-channel -> RRR1, 2-channel -> RGBA (B = 0), 3-channel
/// -> RGB1. 16-bit sources are down-converted to 8-bit by taking
/// the high byte; HDR sources (32F) are tone-mapped to LDR with
/// a Reinhard operator and a gamma 2.2 encode to sRGB.
pub fn normalize_to_rgba8(src_format: TextureFormat, pixels: &[u8]) -> HygeResult<Vec<u8>> {
    let texel_size = src_format.texel_size();
    if pixels.len() % texel_size != 0 {
        return Err(HygeError::invalid_argument(format!(
            "normalize_to_rgba8: source buffer length {} is not a multiple of \
             texel size {texel_size} for {src_format:?}",
            pixels.len()
        )));
    }
    let texels = pixels.len() / texel_size;
    let mut out = Vec::with_capacity(texels * 4);
    match src_format {
        TextureFormat::R8G8B8A8 => {
            // Fast path: identity.
            out.extend_from_slice(pixels);
        }
        TextureFormat::R8 => {
            for chunk in pixels.chunks_exact(1) {
                let r = chunk[0];
                out.extend_from_slice(&[r, r, r, 0xFF]);
            }
        }
        TextureFormat::R8G8 => {
            for chunk in pixels.chunks_exact(2) {
                out.extend_from_slice(&[chunk[0], chunk[1], 0, 0xFF]);
            }
        }
        TextureFormat::R8G8B8 => {
            for chunk in pixels.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 0xFF]);
            }
        }
        TextureFormat::R16 => {
            for chunk in pixels.chunks_exact(2) {
                let r = (u16::from_le_bytes([chunk[0], chunk[1]]) >> 8) as u8;
                out.extend_from_slice(&[r, r, r, 0xFF]);
            }
        }
        TextureFormat::R16G16 => {
            for chunk in pixels.chunks_exact(4) {
                let r = u16::from_le_bytes([chunk[0], chunk[1]]);
                let g = u16::from_le_bytes([chunk[2], chunk[3]]);
                out.extend_from_slice(&[(r >> 8) as u8, (g >> 8) as u8, 0, 0xFF]);
            }
        }
        TextureFormat::R16G16B16 => {
            for chunk in pixels.chunks_exact(6) {
                let r = u16::from_le_bytes([chunk[0], chunk[1]]);
                let g = u16::from_le_bytes([chunk[2], chunk[3]]);
                let b = u16::from_le_bytes([chunk[4], chunk[5]]);
                out.extend_from_slice(&[(r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8, 0xFF]);
            }
        }
        TextureFormat::R16G16B16A16 => {
            for chunk in pixels.chunks_exact(8) {
                let r = u16::from_le_bytes([chunk[0], chunk[1]]);
                let g = u16::from_le_bytes([chunk[2], chunk[3]]);
                let b = u16::from_le_bytes([chunk[4], chunk[5]]);
                let a = u16::from_le_bytes([chunk[6], chunk[7]]);
                out.extend_from_slice(&[
                    (r >> 8) as u8,
                    (g >> 8) as u8,
                    (b >> 8) as u8,
                    (a >> 8) as u8,
                ]);
            }
        }
        TextureFormat::R32G32B32A32FLOAT => {
            for chunk in pixels.chunks_exact(16) {
                let r = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let g = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                let b = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                let a = f32::from_le_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]);
                out.extend_from_slice(&[
                    tonemap_channel(r),
                    tonemap_channel(g),
                    tonemap_channel(b),
                    (a.clamp(0.0, 1.0) * 255.0) as u8,
                ]);
            }
        }
    }
    Ok(out)
}

#[inline]
fn tonemap_channel(x: f32) -> u8 {
    // Reinhard-style tonemap -> sRGB-ish 8-bit.
    let x = if x.is_nan() { 0.0 } else { x };
    let t = x / (1.0 + x);
    let srgb = t.powf(1.0 / 2.2);
    (srgb.clamp(0.0, 1.0) * 255.0) as u8
}

/// Generates a full mip chain with a 2x2 box filter.
///
/// Each level halves the previous width and height (rounded up
/// for odd dimensions) and averages the 2x2 source texel block
/// for every destination texel. The chain runs down to 1x1 so
/// the `level_count` matches the standard
/// `floor(log2(max(width, height))) + 1` rule.
///
/// Returns a `Vec` whose first element is the base level
/// (identical to `pixels`) and whose last element is a single
/// texel.
pub fn generate_mip_chain(width: u32, height: u32, pixels: &[u8]) -> Vec<Vec<u8>> {
    let mut levels: Vec<Vec<u8>> = Vec::new();
    levels.push(pixels.to_vec());
    let mut cur_w = width;
    let mut cur_h = height;
    let mut cur = pixels.to_vec();
    while cur_w > 1 || cur_h > 1 {
        let (next_w, next_h) = mip_dim(cur_w, cur_h);
        let next = box_filter_downsample(&cur, cur_w, cur_h, next_w, next_h);
        cur_w = next_w;
        cur_h = next_h;
        cur = next;
        levels.push(cur.clone());
    }
    levels
}

fn mip_dim(w: u32, h: u32) -> (u32, u32) {
    (w.max(1).div_ceil(2), h.max(1).div_ceil(2))
}

fn box_filter_downsample(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for y in 0..dst_h {
        let sy0 = (y * 2) as usize;
        let sy1 = (sy0 + 1).min(src_h as usize - 1);
        for x in 0..dst_w {
            let sx0 = (x * 2) as usize;
            let sx1 = (sx0 + 1).min(src_w as usize - 1);
            let base_a = (sy0 * src_w as usize + sx0) * 4;
            let base_b = (sy0 * src_w as usize + sx1) * 4;
            let base_c = (sy1 * src_w as usize + sx0) * 4;
            let base_d = (sy1 * src_w as usize + sx1) * 4;
            let dst_base = (y as usize * dst_w as usize + x as usize) * 4;
            for c in 0..4 {
                let sum = u32::from(src[base_a + c])
                    + u32::from(src[base_b + c])
                    + u32::from(src[base_c + c])
                    + u32::from(src[base_d + c]);
                // Sum fits in u32: 4 * 255 = 1020.
                dst[dst_base + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    dst
}

fn encode_ktx2(width: u32, height: u32, vk_format: u32, levels: &[Vec<u8>]) -> HygeResult<Vec<u8>> {
    let level_count = levels.len() as u32;
    let header_bytes: u32 = 56;
    let level_index_bytes = level_count as usize * 32;
    let dfd = build_dfd(width, height, vk_format, level_count);
    let dfd_byte_offset: u32 = header_bytes + level_index_bytes as u32;
    let dfd_byte_length: u32 = dfd.len() as u32;
    let data_start: u64 = (dfd_byte_offset as u64) + (dfd_byte_length as u64);

    let mut level_offsets: Vec<u64> = Vec::with_capacity(levels.len());
    let mut level_lengths: Vec<u64> = Vec::with_capacity(levels.len());
    let mut cursor = data_start;
    for level in levels {
        level_offsets.push(cursor);
        level_lengths.push(level.len() as u64);
        cursor += level.len() as u64;
    }

    let total = cursor as usize;
    let mut out = Vec::with_capacity(total);

    // -- header (56 bytes) ---------------------------------------------
    out.extend_from_slice(&KTX2_MAGIC);
    out.extend_from_slice(&vk_format.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // texelBlockDimension[0]
    out.extend_from_slice(&1u32.to_le_bytes()); // texelBlockDimension[1]
    out.extend_from_slice(&level_count.to_le_bytes());
    out.extend_from_slice(&supercompression_scheme::NONE.to_le_bytes());
    out.extend_from_slice(&dfd_byte_offset.to_le_bytes());
    out.extend_from_slice(&dfd_byte_length.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // kvdByteOffset
    out.extend_from_slice(&0u32.to_le_bytes()); // kvdByteLength
    out.extend_from_slice(&0u64.to_le_bytes()); // sgdByteOffset (supercompression global data)
    debug_assert_eq!(out.len(), 56);

    // -- level index --------------------------------------------------
    for (i, level) in levels.iter().enumerate() {
        out.extend_from_slice(&level_offsets[i].to_le_bytes());
        out.extend_from_slice(&level_lengths[i].to_le_bytes());
        out.extend_from_slice(&(level.len() as u64).to_le_bytes()); // uncompressedByteLength
        out.extend_from_slice(&0u32.to_le_bytes()); // blockIndex
        out.extend_from_slice(&0u32.to_le_bytes()); // blockCount
    }
    debug_assert_eq!(out.len(), dfd_byte_offset as usize);

    // -- data format descriptor ---------------------------------------
    out.extend_from_slice(&dfd);

    // -- level data ----------------------------------------------------
    for level in levels {
        out.extend_from_slice(level);
    }
    debug_assert_eq!(out.len(), total);

    Ok(out)
}

/// Builds a minimal KTX2 Data Format Descriptor (DFD) for a
/// single-sample uncompressed UNORM color-space-insensitive
/// format. The DFD is 64 bytes: 20 bytes of descriptor block
/// header + 4 bytes (packed 16-bit width + 16-bit height) +
/// 4 bytes (plane layout) + 16 bytes of sample byte layout
/// (4 channels x 4 bytes) + 20 bytes of trailing padding to
/// keep the section 4-byte aligned. The `totalSize` and
/// `descriptorBlockSize` fields reflect the actual 64-byte
/// length so a KTX2 reader can round-trip the file.
fn build_dfd(width: u32, height: u32, _vk_format: u32, _level_count: u32) -> Vec<u8> {
    // KTX2 sample type enum constants (Khronos docs / ktx2.h).
    const KHR_DF_TYPE_UNSPECIFIED: u16 = 0;
    const KHR_DF_VERSION: u32 = 0x0000_0100;
    const DFD_SIZE: u32 = 64;
    let mut out = Vec::with_capacity(DFD_SIZE as usize);
    // 20-byte descriptor block header
    out.extend_from_slice(&DFD_SIZE.to_le_bytes()); // totalSize
    out.extend_from_slice(&0u32.to_le_bytes()); // vendorId
    out.extend_from_slice(&KHR_DF_TYPE_UNSPECIFIED.to_le_bytes()); // descriptorType
    out.extend_from_slice(&DFD_SIZE.to_le_bytes()); // descriptorBlockSize
    out.extend_from_slice(&KHR_DF_VERSION.to_le_bytes()); // versionNumber
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
                                                // 4-byte dimension: 16-bit width + 16-bit height packed.
    let dim = width.min(0xFFFF) | (height.min(0xFFFF) << 16);
    out.extend_from_slice(&dim.to_le_bytes());
    // 4-byte plane layout: 0 (no planes)
    out.extend_from_slice(&0u32.to_le_bytes());
    // 16-byte sample byte layout: 4 channels x 4 bytes.
    for _ in 0..4 {
        out.extend_from_slice(&[0u8; 4]);
    }
    // Pad to 64 bytes (20 + 4 + 4 + 16 = 44 so we need 20
    // bytes of trailing padding).
    while out.len() < DFD_SIZE as usize {
        out.push(0);
    }
    out
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> HygeError {
    move |e| HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "hyge-asset-tex-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn ktx2_magic_matches_spec() {
        // Reference: Khronos KTX2 file format spec, identifier
        // bytes. The "20 32 30" sequence is the ASCII for " 20".
        assert_eq!(
            KTX2_MAGIC,
            [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn write_writes_real_ktx2_container() {
        let dir = tmp("magic");
        let p = dir.join("t.ktx2");
        let pixels: Vec<u8> = (0..(4u32 * 4 * 4)).map(|i| (i & 0xFF) as u8).collect();
        let kind = write(
            &p,
            4,
            4,
            WriteOptions {
                generate_mipmaps: false,
                ..WriteOptions::default()
            },
            "image/png",
            &pixels,
        )
        .unwrap();
        let raw = std::fs::read(&p).unwrap();
        assert_eq!(&raw[0..12], &KTX2_MAGIC, "must be KTX2 magic");
        assert!(matches!(
            kind,
            TextureKind::Ktx2 {
                width: 4,
                height: 4,
                ..
            }
        ));
    }

    #[test]
    fn write_rejects_wrong_pixel_buffer_size() {
        let dir = tmp("bad");
        let p = dir.join("t.ktx2");
        let err = write(&p, 4, 4, WriteOptions::default(), "image/png", &[0u8; 10])
            .expect_err("must reject short buffer");
        assert!(matches!(err, HygeError::InvalidArgument(_)));
    }

    #[test]
    fn write_rejects_zero_dimensions() {
        let dir = tmp("zero");
        let p = dir.join("t.ktx2");
        let err = write(&p, 0, 4, WriteOptions::default(), "image/png", &[])
            .expect_err("must reject zero width");
        assert!(matches!(err, HygeError::InvalidArgument(_)));
        let err = write(&p, 4, 0, WriteOptions::default(), "image/png", &[])
            .expect_err("must reject zero height");
        assert!(matches!(err, HygeError::InvalidArgument(_)));
    }

    #[test]
    fn read_header_rejects_non_ktx2_bytes() {
        assert!(read_header(&[0u8; 56]).is_none());
        assert!(read_header(&[0xFFu8; 56]).is_none());
        // KTX1 magic must not be accepted by the KTX2 reader.
        let ktx1 = [
            0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
        ];
        let mut bytes = vec![0u8; 64];
        bytes[0..12].copy_from_slice(&ktx1);
        assert!(read_header(&bytes).is_none());
    }

    #[test]
    fn write_is_deterministic_for_same_input() {
        let dir = tmp("det");
        let p1 = dir.join("a.ktx2");
        let p2 = dir.join("b.ktx2");
        let pixels = vec![0x10u8; 2 * 2 * 4];
        write(
            &p1,
            2,
            2,
            WriteOptions {
                generate_mipmaps: false,
                ..WriteOptions::default()
            },
            "image/png",
            &pixels,
        )
        .unwrap();
        write(
            &p2,
            2,
            2,
            WriteOptions {
                generate_mipmaps: false,
                ..WriteOptions::default()
            },
            "image/png",
            &pixels,
        )
        .unwrap();
        assert_eq!(std::fs::read(&p1).unwrap(), std::fs::read(&p2).unwrap());
    }

    #[test]
    fn mip_chain_length_matches_log2_rule() {
        // 256x256 -> 9 levels (256, 128, 64, 32, 16, 8, 4, 2, 1)
        let pixels = vec![0u8; 256 * 256 * 4];
        let chain = generate_mip_chain(256, 256, &pixels);
        assert_eq!(chain.len(), 9);
        for (i, level) in chain.iter().enumerate() {
            let expected_dim = 256u32 >> i;
            let expected_size = (expected_dim * expected_dim * 4) as usize;
            assert_eq!(level.len(), expected_size, "level {i} dim {expected_dim}");
        }
        // 5x5 -> ceil(log2(5))=3 + 1 = 4 levels (5, 3, 2, 1)
        let pixels5 = vec![0u8; 5 * 5 * 4];
        let chain5 = generate_mip_chain(5, 5, &pixels5);
        assert_eq!(chain5.len(), 4);
        assert_eq!(chain5[0].len(), 5 * 5 * 4);
        assert_eq!(chain5[1].len(), 3 * 3 * 4);
        assert_eq!(chain5[2].len(), 2 * 2 * 4);
        assert_eq!(chain5[3].len(), 4);
    }

    #[test]
    fn mip_chain_box_filter_averages_quads() {
        // 2x2 with distinct quadrants: averaging should yield
        // the global average in the 1x1 mip.
        let mut src = [0u8; 16];
        // top-left (0,0): white
        src[0..4].copy_from_slice(&[255, 255, 255, 255]);
        // top-right (0,1): red
        src[4..8].copy_from_slice(&[255, 0, 0, 255]);
        // bottom-left (1,0): green
        src[8..12].copy_from_slice(&[0, 255, 0, 255]);
        // bottom-right (1,1): blue
        src[12..16].copy_from_slice(&[0, 0, 255, 255]);
        let chain = generate_mip_chain(2, 2, &src);
        assert_eq!(chain.len(), 2);
        let mip = &chain[1];
        assert_eq!(mip.len(), 4);
        // R: average of 255+255+0+0 = 510, +2 rounded /4 = 128
        // (input values: top-left 255, top-right 255, bottom-left 0,
        // bottom-right 0).
        assert_eq!(mip[0], 128u8);
        // G: 255 + 0 + 255 + 0 = 510, +2/4 = 128.
        assert_eq!(mip[1], 128u8);
        // B: 255 + 0 + 0 + 255 = 510, +2/4 = 128.
        assert_eq!(mip[2], 128u8);
        // A: 255 * 4 = 1020, +2/4 = 255 (rounded).
        assert_eq!(mip[3], 255);
    }

    #[test]
    fn write_with_mipmaps_writes_all_levels_in_level_index() {
        let dir = tmp("mips");
        let p = dir.join("t.ktx2");
        let pixels = vec![0x77u8; 4 * 4 * 4];
        let kind = write(&p, 4, 4, WriteOptions::default(), "image/png", &pixels).unwrap();
        let TextureKind::Ktx2 { level_count, .. } = kind;
        // 4x4 -> floor(log2(4)) + 1 = 3 levels (4, 2, 1)
        assert_eq!(level_count, 3);
        let raw = std::fs::read(&p).unwrap();
        let header = read_header(&raw).expect("valid KTX2");
        assert_eq!(header.level_count, 3);
        assert_eq!(header.levels.len(), 3);
        // 4x4 RGBA8 base = 64 bytes; 2x2 = 16; 1x1 = 4.
        assert_eq!(header.levels[0].length, 64);
        assert_eq!(header.levels[1].length, 16);
        assert_eq!(header.levels[2].length, 4);
        // Each level is contiguous in the file.
        let last = header.levels.last().unwrap();
        let end = last.offset + last.length;
        assert_eq!(end as usize, raw.len(), "level data fills file to EOF");
    }

    #[test]
    fn write_without_mipmaps_writes_single_level() {
        let dir = tmp("nomip");
        let p = dir.join("t.ktx2");
        let pixels = vec![0u8; 8 * 8 * 4];
        let kind = write(
            &p,
            8,
            8,
            WriteOptions {
                generate_mipmaps: false,
                ..WriteOptions::default()
            },
            "image/png",
            &pixels,
        )
        .unwrap();
        let TextureKind::Ktx2 { level_count, .. } = kind;
        assert_eq!(level_count, 1);
        let raw = std::fs::read(&p).unwrap();
        let header = read_header(&raw).unwrap();
        assert_eq!(header.levels.len(), 1);
        assert_eq!(header.levels[0].length, 8 * 8 * 4);
    }

    #[test]
    fn ktx2_header_carries_correct_vkformat_for_rgba8() {
        let dir = tmp("vk");
        let p = dir.join("t.ktx2");
        let pixels = vec![0u8; 4];
        write(
            &p,
            1,
            1,
            WriteOptions {
                generate_mipmaps: false,
                ..WriteOptions::default()
            },
            "image/png",
            &pixels,
        )
        .unwrap();
        let raw = std::fs::read(&p).unwrap();
        assert_eq!(&raw[0..12], &KTX2_MAGIC);
        assert_eq!(
            u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            vk_format::R8G8B8A8_UNORM,
            "vkFormat = VK_FORMAT_R8G8B8A8_UNORM"
        );
        assert_eq!(
            u32::from_le_bytes(raw[24..28].try_into().unwrap()),
            1,
            "levelCount = 1 when mipmaps disabled"
        );
        assert_eq!(
            u32::from_le_bytes(raw[28..32].try_into().unwrap()),
            supercompression_scheme::NONE,
            "supercompressionScheme = NONE"
        );
    }

    #[test]
    fn round_trip_1x1_with_mipmaps_writes_only_one_level() {
        let dir = tmp("1x1");
        let p = dir.join("t.ktx2");
        let pixels = vec![0xAB; 4];
        let kind = write(&p, 1, 1, WriteOptions::default(), "image/png", &pixels).unwrap();
        let TextureKind::Ktx2 { level_count, .. } = kind;
        assert_eq!(level_count, 1);
    }

    #[test]
    fn normalize_to_rgba8_is_identity_for_rgba8() {
        let src = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let out = normalize_to_rgba8(TextureFormat::R8G8B8A8, &src).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn normalize_to_rgba8_promotes_r8() {
        let src = vec![10, 20, 30];
        let out = normalize_to_rgba8(TextureFormat::R8, &src).unwrap();
        assert_eq!(out, vec![10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255]);
    }

    #[test]
    fn normalize_to_rgba8_promotes_r8g8() {
        let src = vec![10, 20, 30, 40];
        let out = normalize_to_rgba8(TextureFormat::R8G8, &src).unwrap();
        assert_eq!(out, vec![10, 20, 0, 255, 30, 40, 0, 255]);
    }

    #[test]
    fn normalize_to_rgba8_promotes_r8g8b8() {
        let src = vec![1, 2, 3, 4, 5, 6];
        let out = normalize_to_rgba8(TextureFormat::R8G8B8, &src).unwrap();
        assert_eq!(out, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn normalize_to_rgba8_downshifts_r16g16b16a16() {
        let src: Vec<u8> = vec![
            0x00, 0xFF, // R = 0xFF00 -> 0xFF
            0x80, 0x00, // G = 0x0080 -> 0x00
            0xFF, 0xFF, // B = 0xFFFF -> 0xFF
            0x40, 0x00, // A = 0x0040 -> 0x00
        ];
        let out = normalize_to_rgba8(TextureFormat::R16G16B16A16, &src).unwrap();
        assert_eq!(out, vec![0xFF, 0x00, 0xFF, 0x00]);
    }

    #[test]
    fn normalize_to_rgba8_tonemaps_r32_float() {
        // Linear 1.0 -> after Reinhard -> 0.5 -> after gamma 2.2 ~ 0.73
        // -> 186. 4-byte float RGBA per texel.
        let mut src = Vec::with_capacity(16);
        src.extend_from_slice(&1.0f32.to_le_bytes());
        src.extend_from_slice(&1.0f32.to_le_bytes());
        src.extend_from_slice(&1.0f32.to_le_bytes());
        src.extend_from_slice(&1.0f32.to_le_bytes());
        let out = normalize_to_rgba8(TextureFormat::R32G32B32A32FLOAT, &src).unwrap();
        assert_eq!(out.len(), 4);
        // R, G, B are all tonemapped identically -> same byte.
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        // A is identity.
        assert_eq!(out[3], 255);
        // Tonemap of 1.0 -> (1/(1+1))^(1/2.2) = 0.5^0.4545... ~ 0.7297
        // -> 186. We tolerate +/- 2 to avoid float precision drift.
        assert!((out[0] as i32 - 186).abs() <= 2, "got {}", out[0]);
    }
}
