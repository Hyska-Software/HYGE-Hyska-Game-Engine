//! Binding-file hot reload.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

use crate::{action::ActionMap, binding::load_file};

/// Watches a binding file and exposes validated replacements.
pub struct BindingWatcher {
    path: PathBuf,
    receiver: Receiver<notify::Result<Event>>,
    _watcher: RecommendedWatcher,
}

impl BindingWatcher {
    /// Starts watching the file's parent directory.
    pub fn new(path: impl Into<PathBuf>) -> notify::Result<Self> {
        let path = path.into();
        let (sender, receiver) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(sender, Config::default())?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        Ok(Self {
            path,
            receiver,
            _watcher: watcher,
        })
    }

    /// Returns a validated map when the watched file changed.
    pub fn poll(&self) -> Option<Result<ActionMap, String>> {
        while let Ok(event) = self.receiver.try_recv() {
            let Ok(event) = event else { continue };
            let relevant = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) && event.paths.iter().any(|p| p == &self.path);
            if relevant {
                return Some(load_file(&self.path).map_err(|e| e.to_string()));
            }
        }
        None
    }
}
