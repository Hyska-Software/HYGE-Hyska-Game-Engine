//! Engine-owned project and scene lifecycle.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy_app::App;
use bevy_ecs::world::World;
use hyge_core::result::{HygeError, HygeResult};
use hyge_ecs::plugin::HygePlugin;
use hyge_scene::{load_world_document_from_path, LoadedSceneState, ScenePlugin};

use crate::project::Project;

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
    snapshot: LifecycleSnapshot,
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
            snapshot: LifecycleSnapshot {
                state: LifecycleState::Failed,
                project: None,
                scene: None,
                revision: 0,
                diagnostics: Vec::new(),
            },
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
        self.scene = None;
        self.revision = 0;
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
        let document = self
            .world
            .get_resource::<LoadedSceneState>()
            .and_then(|state| state.document.clone())
            .ok_or_else(|| HygeError::invalid_argument("scene has no loaded document"))?;
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
        self.snapshot = self.make_snapshot(LifecycleState::Ready, Vec::new());
        Ok(self.snapshot.clone())
    }

    /// Returns the current lifecycle snapshot.
    #[must_use]
    pub fn snapshot(&self) -> LifecycleSnapshot {
        self.snapshot.clone()
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
