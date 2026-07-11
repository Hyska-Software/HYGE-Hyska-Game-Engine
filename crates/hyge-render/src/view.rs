//! Shared camera/view contract for runtime and editor rendering.

use hyge_core::prelude::{Mat4, Quat, Vec3};

use crate::clustered_forward::FrameData;

/// Identifies the owner of a render view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderViewMode {
    /// The gameplay camera.
    Game,
    /// The editor-only camera.
    Editor,
    /// An asset preview camera.
    Preview,
}

/// Immutable camera data shared by scene extraction and rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderView {
    /// Camera world-space position.
    pub position: Vec3,
    /// Camera world-space orientation.
    pub rotation: Quat,
    /// View matrix.
    pub view: Mat4,
    /// Projection matrix.
    pub projection: Mat4,
    /// Combined view-projection matrix.
    pub view_proj: Mat4,
    /// Near clipping plane.
    pub near: f32,
    /// Far clipping plane.
    pub far: f32,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// View owner/mode.
    pub mode: RenderViewMode,
}

impl RenderView {
    /// Creates a deterministic editor camera looking at the origin.
    #[must_use]
    pub fn editor_default(width: u32, height: u32) -> Self {
        Self::look_at(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::ZERO,
            width,
            height,
            RenderViewMode::Editor,
        )
    }

    /// Creates a perspective camera looking at a target.
    #[must_use]
    pub fn look_at(
        position: Vec3,
        target: Vec3,
        width: u32,
        height: u32,
        mode: RenderViewMode,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let near = 0.1;
        let far = 1_000.0;
        let view = Mat4::look_at_rh(position, target, Vec3::Y);
        let projection = Mat4::perspective_rh_gl(
            60.0_f32.to_radians(),
            width as f32 / height as f32,
            near,
            far,
        );
        Self {
            position,
            rotation: Quat::from_mat4(&view.inverse()),
            view,
            projection,
            view_proj: projection * view,
            near,
            far,
            width,
            height,
            mode,
        }
    }

    /// Converts the view to the renderer's GPU uniform contract.
    #[must_use]
    pub fn frame_data(self) -> FrameData {
        let mut data = FrameData::default_looking_at_origin();
        data.view_proj = self.view_proj.to_cols_array_2d();
        data.view = self.view.to_cols_array_2d();
        data.camera_pos_alpha_cutoff = [self.position.x, self.position.y, self.position.z, 0.5];
        data.viewport = [self.width as f32, self.height as f32, self.near, self.far];
        data
    }
}
