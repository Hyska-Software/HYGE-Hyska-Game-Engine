//! Capture a rendered [`wgpu::Texture`] to CPU memory.
//!
//! The capture is row-major, 4 bytes per pixel (RGBA8). The
//! bytes are un-padded (wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`
//! can leave gaps between rows in the GPU-side staging buffer;
//! we strip those here so the returned `Vec<u8>` is exactly
//! `width * height * 4` bytes).

use wgpu::util::align_to;

/// Reads the contents of a rendered [`wgpu::Texture`] back to CPU
/// memory and returns the pixels as a row-major RGBA8 buffer.
///
/// `target` must have the `COPY_SRC` usage bit set and its
/// format must be a copyable format (the standard RGBA8
/// formats are). The output is un-padded: the returned
/// `Vec<u8>` is exactly `width * height * 4` bytes long.
///
/// Polls the device with `Maintain::Wait` before returning, so
/// the bytes are guaranteed to be valid by the time the caller
/// reads them.
#[must_use]
pub fn capture_frame(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
    let width = target.width();
    let height = target.height();
    let bytes_per_pixel = 4u32;
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let padded_bytes_per_row = align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer_size = u64::from(padded_bytes_per_row * height);

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hyge-runtime-test/capture-staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hyge-runtime-test/capture-encoder"),
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::ImageCopyBuffer {
            buffer: &staging,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    // Block until the GPU finishes so the map_async callback
    // has fired by the time we touch the mapped range.
    device.poll(wgpu::Maintain::Wait);

    let slice = staging.slice(..);
    let mapped = slice.get_mapped_range();
    let padded_bytes = mapped.to_vec();
    drop(mapped);
    staging.unmap();

    // Strip the row padding so the output is exactly
    // `width * height * 4` bytes.
    let mut unpadded =
        Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
    for row in 0..height as usize {
        let start = row * padded_bytes_per_row as usize;
        let end = start + unpadded_bytes_per_row as usize;
        unpadded.extend_from_slice(&padded_bytes[start..end]);
    }
    unpadded
}
