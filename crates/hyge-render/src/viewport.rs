//! Renderer-owned offscreen targets and deterministic readback.

use std::sync::mpsc;

use wgpu::util::align_to;

use hyge_core::prelude::{HygeError, HygeResult};

/// CPU-visible RGBA8 frame produced by an offscreen render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Monotonically assigned render revision by the caller.
    pub revision: u64,
    /// Unpadded row-major RGBA8 sRGB pixels.
    pub pixels: Vec<u8>,
    /// BLAKE3 hash of `pixels`.
    pub hash: String,
}

/// Renderer-owned persistent target used by editor viewport rendering.
pub(crate) struct OffscreenTarget {
    pub(crate) width: u32,
    pub(crate) height: u32,
    texture: wgpu::Texture,
}

impl OffscreenTarget {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let mut target = Self {
            width: 0,
            height: 0,
            texture: device.create_texture(&texture_descriptor(1, 1)),
        };
        target.resize(device, width, height);
        target
    }

    pub(crate) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.texture = device.create_texture(&texture_descriptor(self.width, self.height));
    }

    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) fn readback(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        revision: u64,
    ) -> HygeResult<ViewportFrame> {
        let bytes_per_pixel = 4;
        let row_bytes = self.width * bytes_per_pixel;
        let padded_row_bytes = align_to(row_bytes, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-render/viewport-readback"),
            size: u64::from(padded_row_bytes * self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hyge-render/viewport-readback-copy"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| HygeError::gpu("viewport readback callback dropped"))?
            .map_err(|error| HygeError::gpu(format!("viewport readback failed: {error}")))?;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((row_bytes * self.height) as usize);
        for row in 0..self.height as usize {
            let start = row * padded_row_bytes as usize;
            pixels.extend_from_slice(&mapped[start..start + row_bytes as usize]);
        }
        drop(mapped);
        staging.unmap();
        let hash = blake3::hash(&pixels).to_hex().to_string();
        Ok(ViewportFrame {
            width: self.width,
            height: self.height,
            revision,
            pixels,
            hash,
        })
    }
}

fn texture_descriptor(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("hyge-render/editor-viewport-color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }
}
