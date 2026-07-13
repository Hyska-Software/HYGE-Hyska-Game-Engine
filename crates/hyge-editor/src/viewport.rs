//! Editor-owned camera state and asynchronous renderer bridge.

use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use hyge_asset::{MaterialAsset, MeshAsset};
use hyge_core::prelude::{Mat4, Quat, Vec3};
use hyge_render::prelude::{
    pod_collect_to_vec, RenderView, RenderViewMode, Renderer, RendererConfig, ViewportFrame,
    ViewportGeometry,
};
use hyge_scene::extract::FrameSnapshot;

/// Persistent camera state for one editor session.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EditorCameraState {
    /// Camera position.
    pub position: [f32; 3],
    /// Camera orientation quaternion `(x, y, z, w)`.
    pub rotation: [f32; 4],
    /// Vertical field of view in degrees.
    pub fov_degrees: f32,
    /// Near clipping plane.
    pub near: f32,
    /// Far clipping plane.
    pub far: f32,
}

impl Default for EditorCameraState {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 5.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            fov_degrees: 60.0,
            near: 0.1,
            far: 1_000.0,
        }
    }
}

impl EditorCameraState {
    /// Validates camera values supplied by the external editor.
    pub fn validate(self) -> Result<Self, String> {
        let values = [
            self.position[0],
            self.position[1],
            self.position[2],
            self.rotation[0],
            self.rotation[1],
            self.rotation[2],
            self.rotation[3],
            self.fov_degrees,
            self.near,
            self.far,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err("editor camera values must be finite".into());
        }
        if !(0.1..179.0).contains(&self.fov_degrees) {
            return Err("editor camera field of view must be between 0.1 and 179 degrees".into());
        }
        if self.near <= 0.0 || self.far <= self.near {
            return Err("editor camera clipping planes are invalid".into());
        }
        let rotation = Quat::from_xyzw(
            self.rotation[0],
            self.rotation[1],
            self.rotation[2],
            self.rotation[3],
        );
        if rotation.length_squared() <= f32::EPSILON {
            return Err("editor camera rotation must not be zero".into());
        }
        Ok(Self {
            rotation: rotation.normalize().to_array(),
            ..self
        })
    }

    /// Builds a render view for the requested target dimensions.
    #[must_use]
    pub fn render_view(self, width: u32, height: u32) -> RenderView {
        let validated = self.validate().unwrap_or_default();
        let rotation = Quat::from_xyzw(
            validated.rotation[0],
            validated.rotation[1],
            validated.rotation[2],
            validated.rotation[3],
        );
        let position = Vec3::from_array(validated.position);
        let view = Mat4::from_rotation_translation(rotation, position).inverse();
        let projection = Mat4::perspective_rh_gl(
            validated.fov_degrees.to_radians(),
            width.max(1) as f32 / height.max(1) as f32,
            validated.near,
            validated.far,
        );
        RenderView {
            position,
            rotation,
            view,
            projection,
            view_proj: projection * view,
            near: validated.near,
            far: validated.far,
            width: width.max(1),
            height: height.max(1),
            mode: RenderViewMode::Editor,
        }
    }
}

/// Mutable viewport state retained by an editor session.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ViewportState {
    /// Current width in pixels.
    pub width: u32,
    /// Current height in pixels.
    pub height: u32,
    /// Camera revision.
    pub camera_revision: u64,
    /// Scene snapshot revision last submitted.
    pub scene_revision: u64,
    /// Last frame revision successfully produced.
    pub last_frame_revision: Option<u64>,
    /// Current render state.
    pub state: ViewportRenderState,
}

/// Viewport render lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ViewportRenderState {
    /// No frame has been requested yet.
    Ready,
    /// A target resize is pending.
    Resizing,
    /// A non-fatal render/readback problem occurred.
    Degraded,
    /// A fatal render problem occurred.
    Failed,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            width: 640,
            height: 360,
            camera_revision: 1,
            scene_revision: 0,
            last_frame_revision: None,
            state: ViewportRenderState::Ready,
        }
    }
}

impl ViewportState {
    /// Updates dimensions and marks the target as needing recreation.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.state = ViewportRenderState::Resizing;
        self.last_frame_revision = None;
    }
}

struct RenderRequest {
    revision: u64,
    view: RenderView,
    instances: Vec<hyge_render::bindless::Instance>,
    draws: Vec<hyge_render::bindless::DrawCommand>,
    lights: Vec<hyge_render::bindless::Light>,
}

enum RenderCommand {
    Render(Box<RenderRequest>),
    Stop,
}

/// Opaque asynchronous bridge between immutable editor snapshots and the
/// renderer-owned headless GPU thread.
pub struct EditorRenderBridge {
    commands: Sender<RenderCommand>,
    frames: Receiver<Result<ViewportFrame, String>>,
    worker: Option<JoinHandle<()>>,
}

impl EditorRenderBridge {
    /// Starts a headless renderer worker.
    pub fn new(config: RendererConfig, geometry: ViewportGeometry) -> Result<Self, String> {
        let (commands, command_rx) = mpsc::channel();
        let (frame_tx, frames) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("hyge-editor-render".into())
            .spawn(move || {
                let mut renderer = match Renderer::new_headless(&config) {
                    Ok(mut renderer) => {
                        renderer.set_viewport_geometry(geometry);
                        let _ = ready_tx.send(Ok(()));
                        renderer
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                while let Ok(command) = command_rx.recv() {
                    match command {
                        RenderCommand::Render(request) => {
                            let result = renderer
                                .render_viewport_frame(
                                    request.view.width,
                                    request.view.height,
                                    request.revision,
                                    &request.view.frame_data(),
                                    &request.instances,
                                    &request.draws,
                                    &request.lights,
                                )
                                .map_err(|error| error.to_string());
                            let _ = frame_tx.send(result);
                        }
                        RenderCommand::Stop => break,
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        ready_rx.recv().map_err(|error| error.to_string())??;
        Ok(Self {
            commands,
            frames,
            worker: Some(worker),
        })
    }

    /// Enqueues an immutable scene snapshot for rendering.
    pub fn submit(
        &self,
        revision: u64,
        view: RenderView,
        snapshot: &FrameSnapshot,
    ) -> Result<(), String> {
        self.commands
            .send(RenderCommand::Render(Box::new(RenderRequest {
                revision,
                view,
                instances: pod_collect_to_vec(&snapshot.instances),
                draws: pod_collect_to_vec(&snapshot.draw_commands),
                lights: pod_collect_to_vec(&snapshot.lights),
            })))
            .map_err(|error| error.to_string())
    }

    /// Returns the newest completed frame without waiting.
    pub fn try_receive(&self) -> Option<Result<ViewportFrame, String>> {
        let mut newest = None;
        while let Ok(frame) = self.frames.try_recv() {
            newest = Some(frame);
        }
        newest
    }
}

/// Loads the first deterministic cooked mesh and material from a project.
///
/// # Errors
///
/// Returns an error when the cook directory is absent, contains no matching
/// assets, or either cooked representation is invalid.
pub fn load_viewport_geometry(
    project_root: &Path,
    mesh_slot: u32,
    material_slot: u32,
) -> Result<ViewportGeometry, String> {
    let cook = project_root.join("assets").join("cook");
    let mut paths = fs::read_dir(&cook)
        .map_err(|error| format!("read cooked assets {}: {error}", cook.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    let mesh_path = paths
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "hyge-mesh"))
        .nth(mesh_slot as usize)
        .ok_or_else(|| {
            format!(
                "mesh handle {mesh_slot} is not present in {}",
                cook.display()
            )
        })?;
    let material_path = paths
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "hyge-mat"))
        .nth(material_slot as usize)
        .ok_or_else(|| {
            format!(
                "material handle {material_slot} is not present in {}",
                cook.display()
            )
        })?;
    let mesh = hyge_asset::importer::mesh::from_bytes(
        &fs::read(mesh_path).map_err(|error| format!("read {}: {error}", mesh_path.display()))?,
    )
    .map_err(|error| format!("decode {}: {error}", mesh_path.display()))?;
    let material = serde_json::from_slice(
        &fs::read(material_path)
            .map_err(|error| format!("read {}: {error}", material_path.display()))?,
    )
    .map_err(|error| format!("decode {}: {error}", material_path.display()))?;
    let base_indices = mesh.meshlets.first().map_or(&[][..], |meshlet| {
        let start = meshlet.index_offset as usize;
        let end = start
            .saturating_add(meshlet.index_count as usize)
            .min(mesh.indices.len());
        &mesh.indices[start..end]
    });
    if mesh.vertices.is_empty() || base_indices.is_empty() {
        return Err("cooked viewport mesh contains no renderable geometry".into());
    }
    let vertices = mesh
        .vertices
        .iter()
        .map(|vertex| {
            [
                vertex.position[0],
                vertex.position[1],
                vertex.position[2],
                vertex.normal[0],
                vertex.normal[1],
                vertex.normal[2],
                1.0,
                0.0,
                0.0,
                1.0,
                vertex.uv[0],
                vertex.uv[1],
            ]
        })
        .collect();
    Ok(ViewportGeometry {
        vertices,
        indices: base_indices.to_vec(),
        mesh: MeshAsset::to_gpu(&mesh).0,
        material: MaterialAsset::to_gpu(&material).0,
    })
}

impl Drop for EditorRenderBridge {
    fn drop(&mut self) {
        let _ = self.commands.send(RenderCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
