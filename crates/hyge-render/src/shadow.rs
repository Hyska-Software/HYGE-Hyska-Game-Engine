//! Shadow resources and CPU planning helpers for M4.
//!
//! The GPU renderer uses these types to build cascade shadow-map
//! layouts and point/spot-light atlas allocations before recording
//! the depth-only passes.

use hyge_core::prelude::{Mat4, Vec3};

/// Number of cascades used by the sun shadow map.
pub const CSM_CASCADE_COUNT: usize = 4;

/// A monotonically increasing list of cascade end distances.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeSplits {
    /// Cascade far distances in view-space units.
    pub distances: Vec<f32>,
}

impl CascadeSplits {
    /// Builds `count` cascades by blending uniform and logarithmic splits.
    #[must_use]
    pub fn lambda_blend(near: f32, far: f32, count: usize, lambda: f32) -> Self {
        let count = count.max(1);
        let near = near.max(0.0001);
        let far = far.max(near + 0.0001);
        let lambda = lambda.clamp(0.0, 1.0);
        let mut distances = Vec::with_capacity(count);
        for i in 1..=count {
            let p = i as f32 / count as f32;
            let log = near * (far / near).powf(p);
            let uniform = near + (far - near) * p;
            distances.push(lambda.mul_add(log, (1.0 - lambda) * uniform));
        }
        Self { distances }
    }
}

/// Per-cascade CPU parameters uploaded to the shadow-aware PBR shader.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct CascadeData {
    /// Light view-projection matrix for this cascade.
    pub light_view_proj: Mat4,
    /// View-space far distance used to select this cascade.
    pub split_depth: f32,
}

/// Builds stable placeholder cascade matrices from a sun direction.
///
/// This helper is intentionally CPU-side and deterministic; the render
/// path can later replace the bounding-sphere fitting step without
/// changing the shader ABI.
#[must_use]
pub fn build_cascade_data(splits: &CascadeSplits, sun_direction: Vec3) -> Vec<CascadeData> {
    let dir = sun_direction
        .try_normalize()
        .unwrap_or(Vec3::new(0.0, -1.0, 0.0));
    let eye = -dir * 100.0;
    let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
    splits
        .distances
        .iter()
        .map(|split| CascadeData {
            light_view_proj: Mat4::orthographic_rh(-split, *split, -split, *split, -200.0, 200.0)
                * view,
            split_depth: *split,
        })
        .collect()
}

/// Rectangle inside a shadow atlas.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    /// Left coordinate in pixels.
    pub x: u32,
    /// Top coordinate in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// One atlas allocation, containing one spot rect or six point-light faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowAtlasAllocation {
    /// Rectangles assigned to the light.
    pub rects: Vec<AtlasRect>,
}

/// First-fit row allocator for a persistent 2D shadow atlas.
#[derive(Debug, Clone)]
pub struct ShadowAtlasAllocator {
    width: u32,
    height: u32,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl ShadowAtlasAllocator {
    /// Creates an empty atlas allocator.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
        }
    }

    /// Allocates one square shadow tile.
    pub fn allocate_spot(&mut self, size: u32) -> Option<AtlasRect> {
        self.allocate_rect(size, size)
    }

    /// Allocates the six faces required for a point-light cubemap layout.
    pub fn allocate_point_light(&mut self, face_size: u32) -> Option<ShadowAtlasAllocation> {
        let checkpoint = (self.cursor_x, self.cursor_y, self.row_height);
        let mut rects = Vec::with_capacity(6);
        for _ in 0..6 {
            if let Some(rect) = self.allocate_rect(face_size, face_size) {
                rects.push(rect);
            } else {
                (self.cursor_x, self.cursor_y, self.row_height) = checkpoint;
                return None;
            }
        }
        Some(ShadowAtlasAllocation { rects })
    }

    fn allocate_rect(&mut self, width: u32, height: u32) -> Option<AtlasRect> {
        if width == 0 || height == 0 || width > self.width || height > self.height {
            return None;
        }
        if self.cursor_x + width > self.width {
            self.cursor_x = 0;
            self.cursor_y = self.cursor_y.saturating_add(self.row_height);
            self.row_height = 0;
        }
        if self.cursor_y + height > self.height {
            return None;
        }
        let rect = AtlasRect {
            x: self.cursor_x,
            y: self.cursor_y,
            width,
            height,
        };
        self.cursor_x += width;
        self.row_height = self.row_height.max(height);
        Some(rect)
    }
}
