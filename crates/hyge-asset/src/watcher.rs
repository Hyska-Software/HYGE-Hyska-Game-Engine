//! Notify-based filesystem watcher for hot-reload candidates.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use hyge_core::result::{HygeError, HygeResult};
use hyge_ecs::Resource;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::asset::AssetId;

/// Debounce window used for filesystem hot-reload events.
pub const DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

type AssetResolver = Arc<dyn Fn(&Path) -> Option<AssetId> + Send + Sync + 'static>;

#[derive(Clone, Copy, Debug)]
struct PendingReload {
    id: AssetId,
    last_seen: Instant,
}

/// Queue of debounced file reload candidates.
///
/// Events are stored as `(path, asset_id)` pairs. The inner queue is protected
/// by a [`Mutex`] so the notify callback thread can push events while engine
/// systems drain them from the main thread.
#[derive(Clone, Debug, Default)]
pub struct ReloadQueue {
    inner: Arc<Mutex<Vec<(PathBuf, AssetId)>>>,
}

impl Resource for ReloadQueue {}

impl ReloadQueue {
    /// Creates an empty reload queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a reload candidate into the queue.
    pub fn push(&self, path: PathBuf, id: AssetId) {
        if let Ok(mut events) = self.inner.lock() {
            events.push((path, id));
        }
    }

    /// Drains all queued reload candidates in FIFO order.
    pub fn drain(&self) -> Vec<(PathBuf, AssetId)> {
        match self.inner.lock() {
            Ok(mut events) => events.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Returns a snapshot of the currently queued reload candidates.
    pub fn snapshot(&self) -> Vec<(PathBuf, AssetId)> {
        match self.inner.lock() {
            Ok(events) => events.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// Returns the number of queued reload candidates.
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |events| events.len())
    }

    /// Returns true when the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Notify-backed watcher that pushes debounced reload candidates into a queue.
///
/// The watcher keeps the underlying [`notify::RecommendedWatcher`] alive and
/// recursively watches each registered root. Asset id resolution is injected so
/// later roadmap items can connect this watcher to `AssetDb` path lookups.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    queue: ReloadQueue,
    watched_roots: Vec<PathBuf>,
    debounce_window: Duration,
}

impl FileWatcher {
    /// Watches `assets/source` and `assets/scripts` under `project_root`.
    ///
    /// Missing directories are created so projects can opt into hot-reload
    /// before those folders contain files.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when directories cannot be created or notify
    /// cannot start a recursive watcher for either directory.
    pub fn watch_asset_dirs(
        project_root: &Path,
        queue: ReloadQueue,
        resolver: AssetResolver,
    ) -> HygeResult<Self> {
        let source = project_root.join("assets").join("source");
        let scripts = project_root.join("assets").join("scripts");
        std::fs::create_dir_all(&source)?;
        std::fs::create_dir_all(&scripts)?;
        Self::watch_paths([source, scripts], queue, resolver)
    }

    /// Recursively watches the provided paths.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when notify cannot create the platform watcher or
    /// when one of the paths cannot be watched recursively.
    pub fn watch_paths<I>(paths: I, queue: ReloadQueue, resolver: AssetResolver) -> HygeResult<Self>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        Self::watch_paths_with_debounce(paths, queue, resolver, DEBOUNCE_WINDOW)
    }

    /// Recursively watches paths with a custom debounce window.
    ///
    /// This constructor is public for deterministic tests and tooling that
    /// needs a different latency/throughput trade-off.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when notify cannot create the platform watcher or
    /// when one of the paths cannot be watched recursively.
    pub fn watch_paths_with_debounce<I>(
        paths: I,
        queue: ReloadQueue,
        resolver: AssetResolver,
        debounce_window: Duration,
    ) -> HygeResult<Self>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let pending = Arc::new(Mutex::new(HashMap::<PathBuf, PendingReload>::new()));
        let callback_queue = queue.clone();
        let callback_pending = Arc::clone(&pending);
        let callback_resolver = Arc::clone(&resolver);

        let mut watcher =
            notify::recommended_watcher(move |event: Result<Event, notify::Error>| match event {
                Ok(event) => handle_event(
                    event,
                    callback_queue.clone(),
                    Arc::clone(&callback_pending),
                    Arc::clone(&callback_resolver),
                    debounce_window,
                ),
                Err(_error) => {}
            })
            .map_err(notify_error("create file watcher"))?;

        let mut watched_roots = Vec::new();
        for path in paths {
            watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(notify_error("watch asset directory"))?;
            watched_roots.push(path);
        }

        Ok(Self {
            watcher,
            queue,
            watched_roots,
            debounce_window,
        })
    }

    /// Returns the queue receiving debounced reload events.
    pub fn queue(&self) -> &ReloadQueue {
        &self.queue
    }

    /// Returns the recursively watched roots.
    pub fn watched_roots(&self) -> &[PathBuf] {
        &self.watched_roots
    }

    /// Returns the configured debounce window.
    pub fn debounce_window(&self) -> Duration {
        self.debounce_window
    }

    /// Returns a shared reference to the underlying notify watcher.
    ///
    /// Exposing this reference makes the wrapper explicit while keeping watch
    /// lifecycle ownership inside [`FileWatcher`].
    pub fn raw_watcher(&self) -> &RecommendedWatcher {
        &self.watcher
    }
}

fn handle_event(
    event: Event,
    queue: ReloadQueue,
    pending: Arc<Mutex<HashMap<PathBuf, PendingReload>>>,
    resolver: AssetResolver,
    debounce_window: Duration,
) {
    if !is_reload_event(&event.kind) {
        return;
    }

    for path in event.paths {
        let path = normalize_path(path);
        let Some(id) = resolver(&path) else {
            continue;
        };

        if let Ok(mut pending_events) = pending.lock() {
            pending_events.insert(
                path.clone(),
                PendingReload {
                    id,
                    last_seen: Instant::now(),
                },
            );
        }

        schedule_debounce_flush(path, queue.clone(), Arc::clone(&pending), debounce_window);
    }
}

fn is_reload_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn schedule_debounce_flush(
    path: PathBuf,
    queue: ReloadQueue,
    pending: Arc<Mutex<HashMap<PathBuf, PendingReload>>>,
    debounce_window: Duration,
) {
    thread::spawn(move || {
        let mut remaining = debounce_window;
        loop {
            thread::sleep(remaining);
            let now = Instant::now();
            let mut pending_events = match pending.lock() {
                Ok(pending_events) => pending_events,
                Err(_) => return,
            };

            let Some(reload) = pending_events.get(&path).copied() else {
                return;
            };
            let elapsed = now.saturating_duration_since(reload.last_seen);
            if elapsed >= debounce_window {
                pending_events.remove(&path);
                drop(pending_events);
                queue.push(path, reload.id);
                return;
            }
            remaining = debounce_window - elapsed;
        }
    });
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn notify_error(context: &'static str) -> impl FnOnce(notify::Error) -> HygeError {
    move |error| HygeError::parse(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after UNIX epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "hyge_file_watcher_{name}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("temp watcher directory should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }

    fn write_and_flush(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .expect("watched test file should open");
        file.write_all(bytes)
            .expect("watched test file should be written");
        file.sync_all()
            .expect("watched test file should be flushed to disk");
    }

    #[test]
    fn watcher_detects_file_write_within_500ms() {
        let temp = TempDir::new("detects_file_write");
        let watched = temp.path().join("assets").join("source");
        fs::create_dir_all(&watched).expect("watched source directory should be created");
        let file_path = watched.join("mesh.gltf");
        let canonical_file = normalize_path(file_path.clone());
        let asset_id = AssetId::from(blake3::hash(b"mesh.gltf"));
        let queue = ReloadQueue::new();
        let resolver_id = asset_id;
        let resolver: AssetResolver = Arc::new(move |_path| Some(resolver_id));

        let _watcher = FileWatcher::watch_paths(vec![watched], queue.clone(), resolver)
            .expect("file watcher should start");
        thread::sleep(Duration::from_millis(50));

        write_and_flush(&file_path, b"first write");

        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let events = queue.snapshot();
            if events
                .iter()
                .any(|(path, id)| path == &canonical_file && *id == asset_id)
                || events.iter().any(|(_path, id)| *id == asset_id)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "expected reload event within 500 ms, got {events:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn watcher_debounces_burst_writes() {
        let temp = TempDir::new("debounces_burst_writes");
        let watched = temp.path().join("assets").join("scripts");
        fs::create_dir_all(&watched).expect("watched scripts directory should be created");
        let file_path = watched.join("player.lua");
        let asset_id = AssetId::from(blake3::hash(b"player.lua"));
        let queue = ReloadQueue::new();
        let resolver_id = asset_id;
        let resolver: AssetResolver = Arc::new(move |_path| Some(resolver_id));

        let _watcher = FileWatcher::watch_paths(vec![watched], queue.clone(), resolver)
            .expect("file watcher should start");
        thread::sleep(Duration::from_millis(50));

        write_and_flush(&file_path, b"one");
        write_and_flush(&file_path, b"two");
        write_and_flush(&file_path, b"three");
        thread::sleep(Duration::from_millis(350));

        let events = queue.snapshot();
        assert_eq!(events.len(), 1, "burst writes should coalesce: {events:?}");
        assert_eq!(events[0].1, asset_id);
    }

    #[test]
    fn watch_asset_dirs_creates_and_watches_required_roots() {
        let temp = TempDir::new("asset_dirs");
        let queue = ReloadQueue::new();
        let resolver: AssetResolver = Arc::new(|_path| None);
        let watcher = FileWatcher::watch_asset_dirs(temp.path(), queue, resolver)
            .expect("asset directory watcher should start");

        assert!(temp.path().join("assets").join("source").is_dir());
        assert!(temp.path().join("assets").join("scripts").is_dir());
        assert_eq!(watcher.watched_roots().len(), 2);
        assert_eq!(watcher.debounce_window(), DEBOUNCE_WINDOW);
    }

    #[test]
    fn reload_queue_drains_in_fifo_order() {
        let queue = ReloadQueue::new();
        let first = AssetId::from(blake3::hash(b"first"));
        let second = AssetId::from(blake3::hash(b"second"));

        queue.push(PathBuf::from("a"), first);
        queue.push(PathBuf::from("b"), second);

        let events = queue.drain();
        assert_eq!(
            events,
            vec![(PathBuf::from("a"), first), (PathBuf::from("b"), second)]
        );
        assert!(queue.is_empty());
    }
}
