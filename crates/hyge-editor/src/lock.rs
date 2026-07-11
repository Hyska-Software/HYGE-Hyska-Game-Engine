//! Project lock ownership for an editor session.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

/// An exclusive lock held for the lifetime of an open editor project.
#[derive(Debug)]
pub struct ProjectLock {
    path: PathBuf,
    file: Option<File>,
}

impl ProjectLock {
    /// Acquires the project lock at `<project>/.hyge/editor.lock`.
    pub fn acquire(project: &Path) -> std::io::Result<Self> {
        let directory = project.join(".hyge");
        fs::create_dir_all(&directory)?;
        let path = directory.join("editor.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        file.try_lock_exclusive()?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_released_on_drop() {
        let root = std::env::temp_dir().join(format!("hyge-lock-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create temp project");
        let first = ProjectLock::acquire(&root).expect("first lock");
        assert!(ProjectLock::acquire(&root).is_err());
        drop(first);
        let second = ProjectLock::acquire(&root).expect("lock after drop");
        drop(second);
        fs::remove_dir_all(root).expect("remove temp project");
    }
}
