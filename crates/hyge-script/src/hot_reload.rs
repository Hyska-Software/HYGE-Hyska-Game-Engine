//! Notify-backed Lua script loading and hot-reload.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Mutex,
    },
};

use bevy_ecs::{
    entity::Entity,
    event::Events,
    world::{Mut, World},
};
use hyge_core::prelude::{HygeError, HygeResult};
use mlua::{Function, Lua, RegistryKey, Table, Value};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{components::ScriptRef, events::ScriptError};

/// The loaded Lua instance for one entity.
struct ScriptInstance {
    path: PathBuf,
    module: RegistryKey,
    state: RegistryKey,
    generation: u64,
}

/// Current script instances, keyed by their owning entity.
#[derive(Default)]
pub struct ScriptState {
    instances: HashMap<Entity, ScriptInstance>,
    generation: u64,
}

impl bevy_ecs::prelude::Resource for ScriptState {}

impl std::fmt::Debug for ScriptState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScriptState")
            .field("instance_count", &self.instances.len())
            .field("generation", &self.generation)
            .finish()
    }
}

impl ScriptState {
    /// Returns the number of loaded entity scripts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Returns whether no entity script is currently loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Returns whether an entity currently has a loaded script.
    #[must_use]
    pub fn contains(&self, entity: Entity) -> bool {
        self.instances.contains_key(&entity)
    }

    /// Returns the reload generation of an entity, if it has a script.
    #[must_use]
    pub fn generation(&self, entity: Entity) -> Option<u64> {
        self.instances
            .get(&entity)
            .map(|instance| instance.generation)
    }
}

/// Owns the native notify watcher for the project's script directory.
pub struct ScriptWatcher {
    watcher: RecommendedWatcher,
    receiver: Mutex<Receiver<notify::Result<Event>>>,
    project_root: PathBuf,
}

impl bevy_ecs::prelude::Resource for ScriptWatcher {}

impl std::fmt::Debug for ScriptWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScriptWatcher")
            .field("project_root", &self.project_root)
            .finish_non_exhaustive()
    }
}

impl ScriptWatcher {
    /// Starts watching `assets/scripts/` below `project_root` recursively.
    pub fn new(project_root: impl Into<PathBuf>) -> HygeResult<Self> {
        let project_root = project_root.into();
        let scripts_root = project_root.join("assets").join("scripts");
        fs::create_dir_all(&scripts_root)?;
        let (sender, receiver) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .map_err(|error| HygeError::parse(format!("create script watcher: {error}")))?;
        watcher
            .watch(&scripts_root, RecursiveMode::Recursive)
            .map_err(|error| HygeError::parse(format!("watch script directory: {error}")))?;
        Ok(Self {
            watcher,
            receiver: Mutex::new(receiver),
            project_root,
        })
    }

    fn drain_changed_paths(&self) -> (Vec<PathBuf>, Vec<HygeError>) {
        let mut paths = Vec::new();
        let mut errors = Vec::new();
        let Ok(receiver) = self.receiver.lock() else {
            return (
                paths,
                vec![HygeError::parse("script watcher receiver lock poisoned")],
            );
        };
        loop {
            match receiver.try_recv() {
                Ok(Ok(event)) if is_reload_event(&event.kind) => {
                    paths.extend(event.paths.into_iter().filter(|path| {
                        path.extension().is_some_and(|extension| extension == "lua")
                    }));
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => errors.push(HygeError::parse(format!("script watcher: {error}"))),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        paths.sort();
        paths.dedup();
        (paths, errors)
    }

    fn normalize(&self, path: &Path) -> PathBuf {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        };
        absolute.canonicalize().unwrap_or(absolute)
    }
}

impl Drop for ScriptWatcher {
    fn drop(&mut self) {
        let _ = self
            .watcher
            .unwatch(&self.project_root.join("assets").join("scripts"));
    }
}

/// Processes pending script filesystem events and loads newly attached scripts.
pub fn process_script_hot_reload(world: &mut World) {
    let Some(watcher) = world.get_resource::<ScriptWatcher>() else {
        return;
    };
    let (changed_paths, watcher_errors) = watcher.drain_changed_paths();
    let normalized_changes: Vec<PathBuf> = changed_paths
        .iter()
        .map(|path| watcher.normalize(path))
        .collect();
    let project_root = watcher.project_root.clone();
    for error in watcher_errors {
        world
            .resource_mut::<Events<ScriptError>>()
            .send(ScriptError {
                entity: Entity::PLACEHOLDER,
                error,
            });
    }

    let references: Vec<(Entity, ScriptRef)> = world
        .query::<(Entity, &ScriptRef)>()
        .iter(world)
        .filter(|(_, script)| script.enabled)
        .map(|(entity, script)| (entity, script.clone()))
        .collect();
    let mut pending = Vec::new();
    for (entity, script) in references {
        let path = normalize_script_path(&project_root, Path::new(&script.path));
        let should_load = world
            .resource::<ScriptState>()
            .instances
            .get(&entity)
            .map_or(true, |instance| instance.path != path)
            || normalized_changes.iter().any(|changed| changed == &path);
        if should_load {
            pending.push((entity, script, path));
        }
    }
    let mut errors = Vec::new();
    world.resource_scope(|world, runtime: Mut<crate::plugin::ScriptRuntime>| {
        let Some(engine) = runtime.engine.as_ref() else {
            return;
        };
        let lua = engine.lua();
        world.resource_scope(|_world, mut state: Mut<ScriptState>| {
            for (entity, script, path) in pending {
                match fs::read_to_string(&path)
                    .map_err(HygeError::from)
                    .and_then(|source| load_instance(lua, &source, &path, script.table.as_deref()))
                {
                    Ok(new_instance) => {
                        if let Some(old_instance) = state.instances.remove(&entity) {
                            if let Err(error) = call_on_reload(lua, &new_instance, &old_instance) {
                                let _ = lua.remove_registry_value(new_instance.module);
                                let _ = lua.remove_registry_value(new_instance.state);
                                state.instances.insert(entity, old_instance);
                                errors.push(ScriptError { entity, error });
                                continue;
                            }
                            let _ = lua.remove_registry_value(old_instance.module);
                            let _ = lua.remove_registry_value(old_instance.state);
                        }
                        state.generation = state.generation.saturating_add(1);
                        let mut new_instance = new_instance;
                        new_instance.generation = state.generation;
                        state.instances.insert(entity, new_instance);
                    }
                    Err(error) => errors.push(ScriptError { entity, error }),
                }
            }
        });
    });
    for error in errors {
        world.resource_mut::<Events<ScriptError>>().send(error);
    }
}

fn load_instance(
    lua: &Lua,
    source: &str,
    path: &Path,
    table_name: Option<&str>,
) -> HygeResult<ScriptInstance> {
    let value: Value = lua
        .load(source)
        .set_name(path.to_string_lossy())
        .eval()
        .map_err(|error| HygeError::parse(format!("Lua script {}: {error}", path.display())))?;
    let module = match (table_name, value) {
        (Some(name), Value::Table(table)) => table
            .get::<Table>(name)
            .map_err(|error| HygeError::parse(format!("Lua script {}: {error}", path.display())))?,
        (None, Value::Table(table)) => table,
        (_, _) => {
            return Err(HygeError::parse(format!(
                "Lua script {} must return a table",
                path.display()
            )))
        }
    };
    let state = module
        .get::<Option<Table>>("state")
        .map_err(|error| HygeError::parse(format!("Lua script {} state: {error}", path.display())))?
        .map_or_else(|| lua.create_table(), Ok)
        .map_err(|error| {
            HygeError::parse(format!("Lua script {} state: {error}", path.display()))
        })?;
    let module = lua
        .create_registry_value(module)
        .map_err(|error| HygeError::parse(format!("Lua module registry: {error}")))?;
    let state = lua
        .create_registry_value(state)
        .map_err(|error| HygeError::parse(format!("Lua state registry: {error}")))?;
    Ok(ScriptInstance {
        path: path.to_path_buf(),
        module,
        state,
        generation: 0,
    })
}

fn call_on_reload(lua: &Lua, new: &ScriptInstance, old: &ScriptInstance) -> HygeResult<()> {
    let module: Table = lua
        .registry_value(&new.module)
        .map_err(|error| HygeError::parse(format!("read reloaded module: {error}")))?;
    let hook: Option<Function> = module
        .get("on_reload")
        .map_err(|error| HygeError::parse(format!("read on_reload hook: {error}")))?;
    if let Some(hook) = hook {
        let old_state: Value = lua
            .registry_value(&old.state)
            .map_err(|error| HygeError::parse(format!("read old script state: {error}")))?;
        hook.call::<()>(old_state)
            .map_err(|error| HygeError::parse(format!("on_reload hook: {error}")))?;
    }
    Ok(())
}

fn normalize_script_path(project_root: &Path, path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    };
    path.canonicalize().unwrap_or(path)
}

fn is_reload_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use bevy_app::App;
    use hyge_ecs::{prelude::*, schedule::Label};

    use super::*;
    use crate::{
        events::ScriptError,
        plugin::{ScriptConfig, ScriptPlugin},
    };

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("hyge_script_reload_{suffix}"));
            fs::create_dir_all(&path).expect("temporary project should be created");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_script(path: &Path, source: &str) {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .expect("script should open");
        file.write_all(source.as_bytes())
            .expect("script should write");
        file.sync_all().expect("script should flush");
    }

    #[test]
    fn modifying_lua_replaces_state_and_calls_on_reload() {
        let project = TempDir::new();
        let path = project.0.join("assets/scripts/player.lua");
        fs::create_dir_all(path.parent().expect("script parent should exist"))
            .expect("script directory");
        write_script(&path, "return { state = { version = 1 } }");

        let mut app = App::new();
        ScriptPlugin::new(ScriptConfig::default())
            .with_project_root(&project.0)
            .build(&mut app);
        let entity = app
            .world_mut()
            .spawn(ScriptRef::new("assets/scripts/player.lua"))
            .id();
        app.world_mut().run_schedule(Label::Update);
        let first_generation = app
            .world()
            .resource::<ScriptState>()
            .generation(entity)
            .expect("initial state");

        thread::sleep(Duration::from_millis(50));
        write_script(
            &path,
            "local state = { version = 2 }; return { state = state, on_reload = function(old) state.reload_called = old.version end }",
        );
        let deadline = std::time::Instant::now() + Duration::from_millis(800);
        loop {
            app.world_mut().run_schedule(Label::Update);
            let state = app.world().resource::<ScriptState>();
            if state
                .generation(entity)
                .is_some_and(|generation| generation > first_generation)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "script was not reloaded"
            );
            thread::sleep(Duration::from_millis(15));
        }
        let runtime = app.world().resource::<crate::plugin::ScriptRuntime>();
        let lua = runtime
            .engine
            .as_ref()
            .expect("runtime should be enabled")
            .lua();
        let state = app.world().resource::<ScriptState>();
        let instance = state.instances.get(&entity).expect("reloaded instance");
        let table: Table = lua
            .registry_value(&instance.state)
            .expect("state table should exist");
        assert_eq!(table.get::<i64>("version").expect("version"), 2);
        assert_eq!(table.get::<i64>("reload_called").expect("reload hook"), 1);
        assert_eq!(app.world().resource::<Events<ScriptError>>().len(), 0);
    }

    #[test]
    fn invalid_reload_emits_error_and_keeps_old_instance() {
        let project = TempDir::new();
        let path = project.0.join("assets/scripts/player.lua");
        fs::create_dir_all(path.parent().expect("script parent should exist"))
            .expect("script directory");
        write_script(&path, "return { state = { version = 1 } }");
        let mut app = App::new();
        ScriptPlugin::new(ScriptConfig::default())
            .with_project_root(&project.0)
            .build(&mut app);
        let entity = app
            .world_mut()
            .spawn(ScriptRef::new("assets/scripts/player.lua"))
            .id();
        app.world_mut().run_schedule(Label::Update);
        let first_generation = app
            .world()
            .resource::<ScriptState>()
            .generation(entity)
            .expect("initial state");
        thread::sleep(Duration::from_millis(50));
        write_script(&path, "return { this is not valid Lua");
        let deadline = std::time::Instant::now() + Duration::from_millis(800);
        loop {
            app.world_mut().run_schedule(Label::Update);
            let errors = app.world().resource::<Events<ScriptError>>();
            if !errors.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "parse error was not emitted"
            );
            thread::sleep(Duration::from_millis(15));
        }
        assert_eq!(
            app.world().resource::<ScriptState>().generation(entity),
            Some(first_generation)
        );
        assert!(app.world().resource::<ScriptState>().contains(entity));
    }
}
