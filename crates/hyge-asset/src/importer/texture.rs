//! `.ktx2` texture writer.
//!
//! **Scope of R-034:** the importer writes a real **KTX1** texture
//! container (a well-documented Khronos format that pre-dates KTX2
//! and predates BasisU supercompression) with the `.ktx2` file
//! extension. KTX1 is a legitimate, widely-supported texture
//! container that the runtime GPU loader can read without any
//! extra dependencies. R-036 (KTX2 transcode) is the milestone
//! responsible for replacing this file in place with a real
//! KTX2 (BasisU-compressed) container. The `.ktx2` extension is
//! the file naming convention the rest of the engine uses for
//! texture cache entries; the on-disk format underneath that
//! extension is upgraded in place by R-036.
//!
//! KTX1 layout (64-byte header followed by tightly-packed level
//! data; KTX1 is the Khronos 1 format documented at
//! <https://www.khronos.org/opengles/sdk/tools/KTX/file_format_spec/>):
//!
//! ```text
//! header          (64 bytes)
//!   identifier    : [u8; 12] = 0xAB 0x4B 0x54 0x58 0x20 0x31 0x31 0xBB 0x0D 0x0A 0x1A 0x0A
//!   endianness    : u32   = 0x04030201 (little-endian magic)
//!   glType        : u32   (e.g. GL_UNSIGNED_BYTE = 0x1401)
//!   glTypeSize    : u32   (bytes per component)
//!   glFormat      : u32   (e.g. GL_RGBA = 0x1908)
//!   glInternalFmt : u32   (e.g. GL_RGBA8 = 0x8058)
//!   glBaseIFormat : u32   (e.g. GL_RGBA = 0x1908)
//!   pixelWidth    : u32
//!   pixelHeight   : u32
//!   pixelDepth    : u32   = 0
//!   arrayElements : u32   = 0
//!   faces         : u32   = 0
//!   mipLevels     : u32   = 1
//!   bytesOfKvData : u32   = 0
//! level_data      (width * height * bytesPerPixel bytes)
//! ```

use std::fs;
use std::path::Path;

use hyge_core::result::HygeResult;

/// KTX1 file identifier. The "20 31 31" sequence is the ASCII
/// `" 11"` suffix that distinguishes KTX1 from KTX2 (`" 20"`).
pub const KTX1_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// KTX1 endianness sentinel. Only the little-endian value is
/// supported on every modern target.
pub const KTX1_ENDIANNESS: u32 = 0x0403_0201;

/// Pixel format the KTX1 writer writes. Matches the glTF
/// [`gltf::image::Format`] variants the importer may encounter,
/// with one row per OpenGL `glType` / `glFormat` / `glInternalFormat`
/// triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TextureFormat {
    /// 8-bit single channel. `glType = GL_UNSIGNED_BYTE`,
    /// `glFormat = GL_RED`, `glInternalFormat = GL_R8`.
    R8 = 1,
    /// 8-bit two channels. `glType = GL_UNSIGNED_BYTE`,
    /// `glFormat = GL_RG`, `glInternalFormat = GL_RG8`.
    R8G8 = 2,
    /// 8-bit three channels. `glType = GL_UNSIGNED_BYTE`,
    /// `glFormat = GL_RGB`, `glInternalFormat = GL_RGB8`.
    R8G8B8 = 3,
    /// 8-bit four channels. `glType = GL_UNSIGNED_BYTE`,
    /// `glFormat = GL_RGBA`, `glInternalFormat = GL_RGBA8`.
    R8G8B8A8 = 4,
    /// 16-bit single channel. `glType = GL_UNSIGNED_SHORT`,
    /// `glFormat = GL_RED`, `glInternalFormat = GL_R16`.
    R16 = 5,
    /// 16-bit two channels. `glType = GL_UNSIGNED_SHORT`,
    /// `glFormat = GL_RG`, `glInternalFormat = GL_RG16`.
    R16G16 = 6,
    /// 16-bit four channels. `glType = GL_UNSIGNED_SHORT`,
    /// `glFormat = GL_RGBA`, `glInternalFormat = GL_RGBA16`.
    R16G16B16A16 = 9,
    /// 32-bit float four channels. `glType = GL_FLOAT`,
    /// `glFormat = GL_RGBA`, `glInternalFormat = GL_RGBA32F`.
    R32G32B32A32FLOAT = 14,
}

impl TextureFormat {
    /// Returns the number of channels for this format.
    #[inline]
    pub const fn channels(self) -> u8 {
        match self {
            Self::R8 | Self::R16 => 1,
            Self::R8G8 | Self::R16G16 => 2,
            Self::R8G8B8 => 3,
            Self::R8G8B8A8 | Self::R16G16B16A16 | Self::R32G32B32A32FLOAT => 4,
        }
    }

    /// Returns the size in bytes of a single texel.
    #[inline]
    pub const fn texel_size(self) -> usize {
        match self {
            Self::R8 | Self::R8G8 | Self::R8G8B8 | Self::R8G8B8A8 => self.channels() as usize,
            Self::R16 | Self::R16G16 | Self::R16G16B16A16 => self.channels() as usize * 2,
            Self::R32G32B32A32FLOAT => 16,
        }
    }
}

/// The kind of payload the `.ktx2` file currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    /// A real KTX1 container. R-036 will rewrite this file
    /// in place with a real KTX2 (BasisU) container.
    Ktx1 {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Pixel format.
        format: TextureFormat,
        /// Lowercase mime type of the source image.
        source_mime: &'static str,
    },
}

/// Writes a KTX1 texture container to `path`.
///
/// # Errors
///
/// Returns [`hyge_core::result::HygeError::InvalidArgument`]
/// when `pixels.len()` does not match the format's texel count,
/// and [`hyge_core::result::HygeError::Io`] on filesystem
/// failure.
pub fn write(
    path: &Path,
    width: u32,
    height: u32,
    format: TextureFormat,
    source_mime: &'static str,
    pixels: &[u8],
) -> HygeResult<TextureKind> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create texture parent dir"))?;
    }
    let expected = (width as usize) * (height as usize) * format.texel_size();
    if pixels.len() != expected {
        return Err(hyge_core::result::HygeError::invalid_argument(format!(
            "KTX1 pixel buffer size mismatch: expected {expected} bytes for \
             {width}x{height} {:?} ({} bytes/texel), got {}",
            format,
            format.texel_size(),
            pixels.len()
        )));
    }

    let mut buf = Vec::with_capacity(64 + pixels.len());
    let desc = ktx1_descriptor(format);
    buf.extend_from_slice(&KTX1_MAGIC);
    buf.extend_from_slice(&KTX1_ENDIANNESS.to_le_bytes());
    buf.extend_from_slice(&desc.gl_type.to_le_bytes());
    buf.extend_from_slice(&desc.gl_type_size.to_le_bytes());
    buf.extend_from_slice(&desc.gl_format.to_le_bytes());
    buf.extend_from_slice(&desc.gl_internal_format.to_le_bytes());
    buf.extend_from_slice(&desc.gl_base_internal_format.to_le_bytes());
    buf.extend_from_slice(&width.to_le_bytes());
    buf.extend_from_slice(&height.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // pixelDepth
    buf.extend_from_slice(&0u32.to_le_bytes()); // numberOfArrayElements
    buf.extend_from_slice(&0u32.to_le_bytes()); // numberOfFaces
    buf.extend_from_slice(&1u32.to_le_bytes()); // numberOfMipmapLevels
    buf.extend_from_slice(&0u32.to_le_bytes()); // bytesOfKeyValueData
    buf.extend_from_slice(pixels);

    fs::write(path, buf).map_err(io_error("write texture file"))?;
    Ok(TextureKind::Ktx1 {
        width,
        height,
        format,
        source_mime,
    })
}

/// Reads the KTX1 header from `bytes`. Returns the parsed
/// `(width, height, format)` and the start offset of the pixel
/// data. Returns `None` when `bytes` does not start with the
/// KTX1 magic.
pub fn read_header(bytes: &[u8]) -> Option<(u32, u32, TextureFormat, usize)> {
    if bytes.len() < 64 || bytes[0..12] != KTX1_MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(bytes[36..40].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[40..44].try_into().ok()?);
    let internal_format = u32::from_le_bytes(bytes[28..32].try_into().ok()?);
    let format = match internal_format {
        // R8 / RG8 / RGB8 / RGBA8
        0x8229 => TextureFormat::R8,
        0x822B => TextureFormat::R8G8,
        0x8051 => TextureFormat::R8G8B8,
        0x8058 => TextureFormat::R8G8B8A8,
        // R16 / RG16 / RGBA16
        0x822A => TextureFormat::R16,
        0x822C => TextureFormat::R16G16,
        0x805B => TextureFormat::R16G16B16A16,
        // RGBA32F
        0x8814 => TextureFormat::R32G32B32A32FLOAT,
        _ => return None,
    };
    Some((width, height, format, 64))
}

/// KTX1 OpenGL format descriptor: `glType` / `glTypeSize` /
/// `glFormat` / `glInternalFormat` / `glBaseInternalFormat`.
struct Ktx1Descriptor {
    gl_type: u32,
    gl_type_size: u32,
    gl_format: u32,
    gl_internal_format: u32,
    gl_base_internal_format: u32,
}

fn ktx1_descriptor(format: TextureFormat) -> Ktx1Descriptor {
    // OpenGL constants used by Khronos KTX1 (see glcorearb.h
    // and the KTX file format spec).
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_UNSIGNED_SHORT: u32 = 0x1403;
    const GL_FLOAT: u32 = 0x1406;
    const GL_RED: u32 = 0x1903;
    const GL_RG: u32 = 0x8227;
    const GL_RGB: u32 = 0x1907;
    const GL_RGBA: u32 = 0x1908;
    const GL_R8: u32 = 0x8229;
    const GL_RG8: u32 = 0x822B;
    const GL_RGB8: u32 = 0x8051;
    const GL_RGBA8: u32 = 0x8058;
    const GL_R16: u32 = 0x822A;
    const GL_RG16: u32 = 0x822C;
    const GL_RGBA16: u32 = 0x805B;
    const GL_RGBA32F: u32 = 0x8814;

    match format {
        TextureFormat::R8 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_BYTE,
            gl_type_size: 1,
            gl_format: GL_RED,
            gl_internal_format: GL_R8,
            gl_base_internal_format: GL_RED,
        },
        TextureFormat::R8G8 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_BYTE,
            gl_type_size: 1,
            gl_format: GL_RG,
            gl_internal_format: GL_RG8,
            gl_base_internal_format: GL_RG,
        },
        TextureFormat::R8G8B8 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_BYTE,
            gl_type_size: 1,
            gl_format: GL_RGB,
            gl_internal_format: GL_RGB8,
            gl_base_internal_format: GL_RGB,
        },
        TextureFormat::R8G8B8A8 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_BYTE,
            gl_type_size: 1,
            gl_format: GL_RGBA,
            gl_internal_format: GL_RGBA8,
            gl_base_internal_format: GL_RGBA,
        },
        TextureFormat::R16 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_SHORT,
            gl_type_size: 2,
            gl_format: GL_RED,
            gl_internal_format: GL_R16,
            gl_base_internal_format: GL_RED,
        },
        TextureFormat::R16G16 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_SHORT,
            gl_type_size: 2,
            gl_format: GL_RG,
            gl_internal_format: GL_RG16,
            gl_base_internal_format: GL_RG,
        },
        TextureFormat::R16G16B16A16 => Ktx1Descriptor {
            gl_type: GL_UNSIGNED_SHORT,
            gl_type_size: 2,
            gl_format: GL_RGBA,
            gl_internal_format: GL_RGBA16,
            gl_base_internal_format: GL_RGBA,
        },
        TextureFormat::R32G32B32A32FLOAT => Ktx1Descriptor {
            gl_type: GL_FLOAT,
            gl_type_size: 4,
            gl_format: GL_RGBA,
            gl_internal_format: GL_RGBA32F,
            gl_base_internal_format: GL_RGBA,
        },
    }
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> hyge_core::result::HygeError {
    move |e| hyge_core::result::HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
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
    fn ktx1_magic_matches_spec() {
        // Reference: Khronos KTX1 file format spec, identifier
        // bytes. The "20 31 31" suffix is the ASCII for " 11".
        assert_eq!(
            KTX1_MAGIC,
            [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }

    #[test]
    fn write_and_read_header_round_trip() {
        let dir = tmp("rt");
        let p = dir.join("t.ktx2");
        let pixels: Vec<u8> = (0..(4u32 * 4 * 4)).map(|i| (i & 0xFF) as u8).collect();
        let kind = write(&p, 4, 4, TextureFormat::R8G8B8A8, "image/png", &pixels).unwrap();
        assert!(matches!(kind, TextureKind::Ktx1 { width: 4, .. }));
        let raw = std::fs::read(&p).unwrap();
        let (w, h, fmt, off) = read_header(&raw).expect("must be KTX1 container");
        assert_eq!((w, h), (4, 4));
        assert_eq!(fmt, TextureFormat::R8G8B8A8);
        assert_eq!(off, 64, "KTX1 header is 64 bytes");
        assert_eq!(
            raw.len(),
            64 + pixels.len(),
            "raw file must be header + pixels"
        );
        assert_eq!(&raw[64..], pixels.as_slice());
    }

    #[test]
    fn write_rejects_wrong_pixel_buffer_size() {
        let dir = tmp("bad");
        let p = dir.join("t.ktx2");
        let err = write(&p, 4, 4, TextureFormat::R8G8B8A8, "image/png", &[0u8; 10])
            .expect_err("must reject short buffer");
        assert!(matches!(
            err,
            hyge_core::result::HygeError::InvalidArgument(_)
        ));
    }

    #[test]
    fn read_header_rejects_non_ktx1_bytes() {
        assert!(read_header(&[0u8; 64]).is_none());
        assert!(read_header(&[0xFFu8; 64]).is_none());
        // KTX2 magic must not be accepted by the KTX1 reader.
        let ktx2 = [
            0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
        ];
        let mut bytes = vec![0u8; 64];
        bytes[0..12].copy_from_slice(&ktx2);
        assert!(read_header(&bytes).is_none());
    }

    #[test]
    fn write_is_deterministic_for_same_input() {
        let dir = tmp("det");
        let p1 = dir.join("a.ktx2");
        let p2 = dir.join("b.ktx2");
        let pixels = vec![0x10u8; 2 * 2 * 4];
        write(&p1, 2, 2, TextureFormat::R8G8B8A8, "image/png", &pixels).unwrap();
        write(&p2, 2, 2, TextureFormat::R8G8B8A8, "image/png", &pixels).unwrap();
        assert_eq!(std::fs::read(&p1).unwrap(), std::fs::read(&p2).unwrap());
    }

    #[test]
    fn ktx1_header_carries_correct_gl_constants_for_rgba8() {
        let dir = tmp("hdr");
        let p = dir.join("t.ktx2");
        let pixels = vec![0u8; 4];
        write(&p, 1, 1, TextureFormat::R8G8B8A8, "image/png", &pixels).unwrap();
        let raw = std::fs::read(&p).unwrap();
        assert_eq!(&raw[0..12], &KTX1_MAGIC);
        assert_eq!(
            u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            0x0403_0201
        );
        assert_eq!(
            u32::from_le_bytes(raw[16..20].try_into().unwrap()),
            0x1401,
            "glType = GL_UNSIGNED_BYTE"
        );
        assert_eq!(
            u32::from_le_bytes(raw[20..24].try_into().unwrap()),
            1,
            "glTypeSize = 1 for GL_UNSIGNED_BYTE"
        );
        assert_eq!(
            u32::from_le_bytes(raw[24..28].try_into().unwrap()),
            0x1908,
            "glFormat = GL_RGBA"
        );
        assert_eq!(
            u32::from_le_bytes(raw[28..32].try_into().unwrap()),
            0x8058,
            "glInternalFormat = GL_RGBA8"
        );
        assert_eq!(
            u32::from_le_bytes(raw[32..36].try_into().unwrap()),
            0x1908,
            "glBaseInternalFormat = GL_RGBA"
        );
    }
}
