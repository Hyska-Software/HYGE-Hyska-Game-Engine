//! Engine-owned project and scene lifecycle.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy_app::App;
use bevy_ecs::world::World;
use hyge_core::result::{HygeError, HygeResult};
use hyge_ecs::plugin::HygePlugin;
use hyge_scene::{
    load_world_document_from_path, sync_editor_layer_from_world, LoadedSceneState, ScenePlugin,
};

use crate::commands::{CommandEffect, CommandFailure, EditorCommand};
use crate::data::{
    AssetSnapshot, ConsoleFilter, ConsoleSnapshot, EditorDataServices, PreviewResult,
    ProfilerSnapshot,
};
use crate::history::CommandHistory;
use crate::project::Project;
use crate::snapshots::{build_snapshot, EditorSnapshot, EntityId};
use crate::transport::{InputBridge, ViewportInputBatch};
use crate::viewport::{EditorCameraState, ViewportState};

/// Lifecycle state visible to the frontend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    /// An operation is in progress.
    Loading,
    /// Project or scene is usable.
    Ready,
    /// Usable with non-fatal diagnostics.
    Degraded,
    /// The requested operation failed.
    Failed,
}

impl LifecycleState {
    /// Returns the protocol spelling.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

/// Public lifecycle snapshot.
#[derive(Clone, Debug)]
pub struct LifecycleSnapshot {
    /// Current state.
    pub state: LifecycleState,
    /// Canonical project path.
    pub project: Option<PathBuf>,
    /// Canonical scene path.
    pub scene: Option<PathBuf>,
    /// Persisted scene revision.
    pub revision: u64,
    /// Diagnostics collected during the last operation.
    pub diagnostics: Vec<String>,
}

/// One engine-owned editor runtime.
pub struct EditorSessionRuntime {
    world: World,
    project: Option<Project>,
    scene: Option<PathBuf>,
    revision: u64,
    snapshot_revision: u64,
    selection: Vec<EntityId>,
    history: CommandHistory,
    snapshot: LifecycleSnapshot,
    data: EditorDataServices,
    editor_camera: EditorCameraState,
    viewport: ViewportState,
    input: InputBridge,
}

impl EditorSessionRuntime {
    /// Creates an empty headless-capable runtime without entering `winit`.
    #[must_use]
    pub fn new() -> Self {
        let world = build_world();
        Self {
            world,
            project: None,
            scene: None,
            revision: 0,
            snapshot_revision: 0,
            selection: Vec::new(),
            history: CommandHistory::default(),
            snapshot: LifecycleSnapshot {
                state: LifecycleState::Failed,
                project: None,
                scene: None,
                revision: 0,
                diagnostics: Vec::new(),
            },
            data: EditorDataServices::default(),
            editor_camera: EditorCameraState::default(),
            viewport: ViewportState::default(),
            input: InputBridge::default(),
        }
    }

    /// Opens and locks a project, replacing the previous project only after
    /// discovery succeeds.
    pub fn open_project(&mut self, path: &Path) -> HygeResult<LifecycleSnapshot> {
        let project = Project::open(path)?;
        let mut candidate = build_world();
        let mut library = candidate
            .get_resource_mut::<hyge_scene::PrefabLibrary>()
            .ok_or_else(|| {
                HygeError::invalid_argument("scene plugin did not install prefab library")
            })?;
        let count = project.load_prefabs(&mut library)?;
        let mut diagnostics = project.diagnostics.clone();
        if count == 0 {
            diagnostics.push("project contains no .hyge-prefab files".to_owned());
        }
        self.world = candidate;
        self.project = Some(project);
        self.data
            .previews
            .set_project(self.project.as_ref().map(|project| project.root.as_path()));
        self.scene = None;
        self.revision = 0;
        self.snapshot_revision = 1;
        self.viewport.scene_revision = self.snapshot_revision;
        self.viewport.last_frame_revision = None;
        self.viewport.state = crate::viewport::ViewportRenderState::Ready;
        self.selection.clear();
        self.history.clear();
        self.snapshot = self.make_snapshot(
            if diagnostics.is_empty() {
                LifecycleState::Ready
            } else {
                LifecycleState::Degraded
            },
            diagnostics,
        );
        Ok(self.snapshot.clone())
    }

    /// Loads a scene through the production scene runtime.
    pub fn open_scene(&mut self, path: &Path) -> HygeResult<LifecycleSnapshot> {
        let project = self
            .project
            .as_ref()
            .ok_or_else(|| HygeError::invalid_argument("project is not open"))?;
        let scene = project.scene_path(path)?;
        let mut candidate = build_world();
        let library = self
            .world
            .get_resource::<hyge_scene::PrefabLibrary>()
            .cloned()
            .ok_or_else(|| {
                HygeError::invalid_argument("scene plugin did not install prefab library")
            })?;
        candidate.insert_resource(library);
        load_world_document_from_path(&mut candidate, &scene)?;
        let state = candidate
            .get_resource::<LoadedSceneState>()
            .ok_or_else(|| HygeError::invalid_argument("scene loader did not publish state"))?;
        if state.document.is_none() {
            return Err(HygeError::invalid_argument(
                "scene loader returned no document",
            ));
        }
        self.world = candidate;
        self.scene = Some(scene);
        self.revision = read_revision(self.project_root()?)?;
        self.snapshot_revision = self.snapshot_revision.saturating_add(1).max(1);
        self.viewport.scene_revision = self.snapshot_revision;
        self.viewport.last_frame_revision = None;
        self.viewport.state = crate::viewport::ViewportRenderState::Ready;
        self.selection.clear();
        self.history.clear();
        self.snapshot = self.make_snapshot(LifecycleState::Ready, Vec::new());
        Ok(self.snapshot.clone())
    }

    /// Atomically persists the currently loaded world document.
    pub fn save_scene(&mut self) -> HygeResult<LifecycleSnapshot> {
        let project_root = self.project_root()?.to_path_buf();
        let scene = self
            .scene
            .clone()
            .ok_or_else(|| HygeError::invalid_argument("scene is not open"))?;
        let mut document = self
            .world
            .get_resource::<LoadedSceneState>()
            .and_then(|state| state.document.clone())
            .ok_or_else(|| HygeError::invalid_argument("scene has no loaded document"))?;
        sync_editor_layer_from_world(&mut self.world, &mut document)?;
        let bytes = document.to_bytes()?;
        AtomicFile::new(&scene, AllowOverwrite)
            .write(|file| {
                use std::io::Write;
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .map_err(|error| HygeError::invalid_argument(format!("save scene: {error}")))?;
        let next_revision = self.revision.saturating_add(1);
        let revision_path = project_root.join(".hyge").join("editor.revision");
        AtomicFile::new(&revision_path, AllowOverwrite)
            .write(|file| {
                use std::io::Write;
                file.write_all(next_revision.to_string().as_bytes())?;
                file.sync_all()
            })
            .map_err(|error| HygeError::invalid_argument(format!("save revision: {error}")))?;
        self.revision = next_revision;
        if let Some(mut state) = self.world.get_resource_mut::<LoadedSceneState>() {
            state.document = Some(document);
        }
        self.snapshot_revision = self.snapshot_revision.saturating_add(1).max(1);
        self.viewport.scene_revision = self.snapshot_revision;
        self.viewport.last_frame_revision = None;
        self.snapshot = self.make_snapshot(LifecycleState::Ready, Vec::new());
        Ok(self.snapshot.clone())
    }

    /// Returns the current lifecycle snapshot.
    #[must_use]
    pub fn snapshot(&self) -> LifecycleSnapshot {
        self.snapshot.clone()
    }

    /// Returns the session-owned data services.
    #[must_use]
    pub fn data_services(&self) -> EditorDataServices {
        self.data.clone()
    }

    /// Returns the project asset snapshot.
    pub fn asset_snapshot(&self) -> Result<AssetSnapshot, String> {
        self.project
            .as_ref()
            .ok_or_else(|| "project is not open".to_owned())
            .and_then(|project| self.data.asset_snapshot(&project.root))
    }

    /// Returns filtered retained console lines.
    #[must_use]
    pub fn console_snapshot(&self, filter: ConsoleFilter) -> ConsoleSnapshot {
        self.data.console.snapshot(filter)
    }

    /// Returns retained profiler samples.
    #[must_use]
    pub fn profiler_snapshot(&self) -> ProfilerSnapshot {
        self.data.profiler.snapshot()
    }

    /// Requests a deterministic preview for an asset.
    pub fn request_asset_preview(
        &self,
        asset_id: &str,
        job_id: &str,
    ) -> Result<PreviewResult, String> {
        self.data.previews.request(asset_id, job_id)
    }

    /// Cancels a preview job.
    pub fn cancel_asset_preview(&self, job_id: &str) -> bool {
        self.data.previews.cancel(job_id)
    }

    /// Returns the immutable ECS/editor snapshot for the current session.
    pub fn editor_snapshot(&self) -> HygeResult<EditorSnapshot> {
        build_snapshot(
            &self.world,
            self.snapshot_revision,
            self.revision,
            &self.selection,
        )
    }

    /// Returns the session-owned editor camera.
    #[must_use]
    pub fn editor_camera(&self) -> EditorCameraState {
        self.editor_camera
    }

    /// Returns the session-owned viewport state.
    #[must_use]
    pub fn viewport_state(&self) -> ViewportState {
        self.viewport.clone()
    }

    /// Updates the editor camera without touching any game-camera component.
    pub fn set_editor_camera(
        &mut self,
        camera: EditorCameraState,
    ) -> Result<EditorCameraState, String> {
        let camera = camera.validate()?;
        self.editor_camera = camera;
        self.viewport.camera_revision = self.viewport.camera_revision.saturating_add(1).max(1);
        self.viewport.last_frame_revision = None;
        Ok(camera)
    }

    /// Resizes the editor viewport and invalidates its current frame.
    pub fn set_viewport_size(&mut self, width: u32, height: u32) -> ViewportState {
        self.viewport.resize(width, height);
        self.viewport.clone()
    }

    /// Applies one ordered, rate-limited input batch for a transport generation.
    pub fn apply_viewport_input(
        &mut self,
        batch: &ViewportInputBatch,
        generation: u64,
    ) -> Result<u64, &'static str> {
        self.input.accept(batch, generation)?;
        self.viewport.camera_revision = self.viewport.camera_revision.saturating_add(1);
        self.viewport.last_frame_revision = None;
        Ok(self.input.revision())
    }

    /// Replaces the engine-owned selection and returns its new snapshot.
    pub fn select_entities(&mut self, entities: Vec<EntityId>) -> HygeResult<EditorSnapshot> {
        self.select_entities_with_shift(entities, false)
    }

    /// Replaces or extends the engine-owned selection.
    pub fn select_entities_with_shift(
        &mut self,
        entities: Vec<EntityId>,
        shift: bool,
    ) -> HygeResult<EditorSnapshot> {
        let mut selection = if shift {
            self.selection.clone()
        } else {
            Vec::new()
        };
        selection.extend(entities);
        selection.sort_unstable();
        selection.dedup();
        selection.retain(|entity| {
            let Ok(entity) = bevy_ecs::entity::Entity::try_from_bits(*entity) else {
                return false;
            };
            self.world.get_entity(entity).is_some()
        });
        self.selection = selection;
        self.snapshot_revision = self.snapshot_revision.saturating_add(1).max(1);
        self.editor_snapshot()
    }

    /// Selects persistent scene identities, resolving them to live ECS entities.
    pub fn select_scene_ids(
        &mut self,
        scene_ids: Vec<String>,
        shift: bool,
    ) -> HygeResult<EditorSnapshot> {
        let mut query = self
            .world
            .query::<(bevy_ecs::entity::Entity, &hyge_scene::SceneNodeId)>();
        let ids: std::collections::HashMap<String, EntityId> = query
            .iter(&self.world)
            .map(|(entity, scene_id)| (scene_id.0.clone(), entity.to_bits()))
            .collect();
        let entities = scene_ids
            .into_iter()
            .filter_map(|scene_id| ids.get(&scene_id).copied())
            .collect();
        self.select_entities_with_shift(entities, shift)
    }

    /// Applies a command after validating its optimistic snapshot revision.
    pub fn apply_command(
        &mut self,
        expected_revision: u64,
        command: EditorCommand,
    ) -> Result<(CommandEffect, EditorSnapshot), CommandFailure> {
        self.check_revision(expected_revision)?;
        let command_kind = command.clone();
        let effect = self.history.apply(command, &mut self.world)?;
        self.update_selection_after_command(&command_kind, &effect);
        self.bump_snapshot_revision();
        let snapshot = self
            .editor_snapshot()
            .map_err(|error| CommandFailure::new("command_failed", error.to_string()))?;
        Ok((effect, snapshot))
    }

    /// Undoes the latest command after validating the snapshot revision.
    pub fn undo_command(
        &mut self,
        expected_revision: u64,
    ) -> Result<(CommandEffect, EditorSnapshot), CommandFailure> {
        self.check_revision(expected_revision)?;
        let effect = self.history.undo(&mut self.world)?;
        self.filter_selection_to_live_entities();
        self.bump_snapshot_revision();
        let snapshot = self
            .editor_snapshot()
            .map_err(|error| CommandFailure::new("command_failed", error.to_string()))?;
        Ok((effect, snapshot))
    }

    /// Redoes the latest reverted command after validating the snapshot revision.
    pub fn redo_command(
        &mut self,
        expected_revision: u64,
    ) -> Result<(CommandEffect, EditorSnapshot), CommandFailure> {
        self.check_revision(expected_revision)?;
        let effect = self.history.redo(&mut self.world)?;
        self.filter_selection_to_live_entities();
        self.bump_snapshot_revision();
        let snapshot = self
            .editor_snapshot()
            .map_err(|error| CommandFailure::new("command_failed", error.to_string()))?;
        Ok((effect, snapshot))
    }

    /// Returns whether undo is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Returns whether redo is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Records a failed operation without replacing the active runtime.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.snapshot = self.make_snapshot(LifecycleState::Failed, vec![message.into()]);
    }

    fn project_root(&self) -> HygeResult<&Path> {
        self.project
            .as_ref()
            .map(|project| project.root.as_path())
            .ok_or_else(|| HygeError::invalid_argument("project is not open"))
    }

    fn check_revision(&self, expected_revision: u64) -> Result<(), CommandFailure> {
        if expected_revision != self.snapshot_revision {
            return Err(CommandFailure::new(
                "stale_revision",
                format!(
                    "expected snapshot revision {}, current revision {}",
                    expected_revision, self.snapshot_revision
                ),
            ));
        }
        Ok(())
    }

    fn bump_snapshot_revision(&mut self) {
        self.snapshot_revision = self.snapshot_revision.saturating_add(1).max(1);
        self.viewport.scene_revision = self.snapshot_revision;
        self.viewport.last_frame_revision = None;
    }

    fn update_selection_after_command(&mut self, command: &EditorCommand, effect: &CommandEffect) {
        match command {
            EditorCommand::Duplicate(_) | EditorCommand::Instantiate(_) => {
                if let Some(root) = effect.affected_entities.first().copied() {
                    self.selection = vec![root];
                }
            }
            EditorCommand::Destroy(_) => self.filter_selection_to_live_entities(),
            EditorCommand::Reparent(_)
            | EditorCommand::EditComponent(_)
            | EditorCommand::EditComponents(_)
            | EditorCommand::AddComponent(_)
            | EditorCommand::RemoveComponent(_) => {}
        }
    }

    fn filter_selection_to_live_entities(&mut self) {
        self.selection.retain(|bits| {
            bevy_ecs::entity::Entity::try_from_bits(*bits)
                .ok()
                .is_some_and(|entity| self.world.get_entity(entity).is_some())
        });
    }

    fn make_snapshot(&self, state: LifecycleState, diagnostics: Vec<String>) -> LifecycleSnapshot {
        LifecycleSnapshot {
            state,
            project: self.project.as_ref().map(|p| p.root.clone()),
            scene: self.scene.clone(),
            revision: self.revision,
            diagnostics,
        }
    }
}

impl Default for EditorSessionRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe runtime handle used by the server.
pub type RuntimeHandle = std::sync::Arc<Mutex<EditorSessionRuntime>>;

fn read_revision(root: &Path) -> HygeResult<u64> {
    let path = root.join(".hyge").join("editor.revision");
    match std::fs::read_to_string(path) {
        Ok(value) => value
            .trim()
            .parse()
            .map_err(|_| HygeError::invalid_argument("editor revision is corrupt")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

fn build_world() -> World {
    let mut app = App::new();
    ScenePlugin.build(&mut app);
    std::mem::take(app.world_mut())
}
