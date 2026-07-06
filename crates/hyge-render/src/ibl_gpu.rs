//! Runtime upload of an [`EnvironmentBake`] to wgpu textures.
//!
//! The PBR shader's `irradiance_map` / `prefiltered_env_map` /
//! `brdf_lut` bindings (`@group(1)` slots 2/3/4) live outside
//! the bindless table (they are per-scene, not per-material),
//! so [`IblResources`] holds the three `Arc<wgpu::TextureView>`s
//! the PBR pass binds directly.
//!
//! The textures are Rgba16Float in all three cases; the cubemaps
//! use `texture_dimension: Cube` and the per-mip prefilter is
//! uploaded as a single `Cube` texture with `array_layers = 6 *
//! mip_count` so the shader can `textureSampleLevel` it directly
//! with a roughness-driven LOD.
//!
//! This is the "online" path: a runtime-loaded environment
//! (R-041 acceptance #4) walks the same code path as the
//! offline bake, just in a different host function.

use std::sync::Arc;

use hyge_core::prelude::HygeResult;

use crate::ibl::{BrdfLut, EnvironmentBake, IrradianceCubemap, PrefilterCubemap};

/// The three wgpu texture views the PBR pass binds at
/// `@group(1)` slots 2/3/4. The views are `Arc` so the PBR
/// pass can hold them in its per-frame bind group without
/// duplicating the underlying wgpu object.
#[derive(Debug, Clone)]
pub struct IblResources {
    /// The diffuse irradiance cubemap view (Rgba16Float,
    /// `Cube` dimension, 6 array layers, 1 mip).
    pub irradiance_view: Arc<wgpu::TextureView>,
    /// The prefiltered environment cubemap view (Rgba16Float,
    /// `Cube` dimension, `6 * mip_count` array layers,
    /// `mip_count` mips). The shader samples the appropriate
    /// layer via the roughness-LOD formula.
    pub prefiltered_view: Arc<wgpu::TextureView>,
    /// The 2D BRDF integration LUT view (Rgba16Float,
    /// square).
    pub brdf_lut_view: Arc<wgpu::TextureView>,
}

impl IblResources {
    /// Returns the irradiance view (alias for
    /// `self.irradiance_view.clone()`).
    #[must_use]
    pub fn irradiance(&self) -> Arc<wgpu::TextureView> {
        Arc::clone(&self.irradiance_view)
    }

    /// Returns the prefiltered environment view (alias for
    /// `self.prefiltered_view.clone()`).
    #[must_use]
    pub fn prefilter(&self) -> Arc<wgpu::TextureView> {
        Arc::clone(&self.prefiltered_view)
    }

    /// Returns the BRDF LUT view (alias for
    /// `self.brdf_lut_view.clone()`).
    #[must_use]
    pub fn brdf_lut(&self) -> Arc<wgpu::TextureView> {
        Arc::clone(&self.brdf_lut_view)
    }
}

/// Uploads `bake` to three wgpu textures and returns the
/// views. The caller is responsible for binding the views
/// into the PBR pass's frame bind group.
///
/// # Errors
///
/// Returns [`hyge_core::prelude::HygeError::Gpu`] when the
/// device is lost.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bake: &EnvironmentBake,
) -> HygeResult<IblResources> {
    let _ = device; // device is only used for the texture constructors below.
    let prefiltered = upload_prefilter(device, queue, &bake.prefilter);
    let irradiance = upload_irradiance(device, queue, &bake.irradiance);
    let brdf_lut = upload_brdf_lut(device, queue, &bake.brdf_lut);
    Ok(IblResources {
        irradiance_view: Arc::new(irradiance),
        prefiltered_view: Arc::new(prefiltered),
        brdf_lut_view: Arc::new(brdf_lut),
    })
}

fn upload_prefilter(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    prefilter: &PrefilterCubemap,
) -> wgpu::TextureView {
    let base = prefilter.base_size;
    let mips = prefilter.mip_count;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hyge-render/ibl-prefilter"),
        size: wgpu::Extent3d {
            width: base,
            height: base,
            depth_or_array_layers: 6 * mips,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for mip in 0..mips {
        let s = (base >> mip).max(1);
        let bytes_per_face = (s as usize) * (s as usize) * 4 * 2; // RGBA16F = 8 bytes/texel
        let mut face_bytes: Vec<u8> = Vec::with_capacity(bytes_per_face * 6);
        for face in 0..6 {
            for t in &prefilter.mip_chain[mip as usize][face] {
                face_bytes.extend_from_slice(t);
            }
        }
        for face in 0..6u32 {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: face + mip * 6,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &face_bytes[(face as usize) * bytes_per_face..(face as usize + 1) * bytes_per_face],
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(s * 8),
                    rows_per_image: Some(s),
                },
                wgpu::Extent3d {
                    width: s,
                    height: s,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("hyge-render/ibl-prefilter-view"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

fn upload_irradiance(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    irradiance: &IrradianceCubemap,
) -> wgpu::TextureView {
    let s = irradiance.size;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hyge-render/ibl-irradiance"),
        size: wgpu::Extent3d {
            width: s,
            height: s,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for face in 0..6u32 {
        let texels = &irradiance.faces_rgba16f[face as usize];
        let bytes: Vec<u8> = texels.iter().flat_map(|t| t.iter().copied()).collect();
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: face,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(s * 8),
                rows_per_image: Some(s),
            },
            wgpu::Extent3d {
                width: s,
                height: s,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("hyge-render/ibl-irradiance-view"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

fn upload_brdf_lut(device: &wgpu::Device, queue: &wgpu::Queue, lut: &BrdfLut) -> wgpu::TextureView {
    let s = lut.size;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hyge-render/ibl-brdf-lut"),
        size: wgpu::Extent3d {
            width: s,
            height: s,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = lut
        .pixels_rgba16f
        .iter()
        .flat_map(|t| t.iter().copied())
        .collect();
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(s * 8),
            rows_per_image: Some(s),
        },
        wgpu::Extent3d {
            width: s,
            height: s,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("hyge-render/ibl-brdf-lut-view"),
        dimension: Some(wgpu::TextureViewDimension::D2),
        ..Default::default()
    })
}
