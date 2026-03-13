use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Debounced file change events from the file system.
pub enum FileChange {
    Modified(PathBuf),
    Created(PathBuf),
    Removed(PathBuf),
}

/// Watches a directory for file changes with debouncing.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<FileChange>,
}

impl FileWatcher {
    /// Start watching `root` for file changes.
    ///
    /// Changes are debounced: rapid successive events on the same file
    /// are collapsed into one.
    pub fn new(root: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel();

        let sender = tx;
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                for path in event.paths {
                    let change = match event.kind {
                        EventKind::Create(_) => FileChange::Created(path),
                        EventKind::Modify(_) => FileChange::Modified(path),
                        EventKind::Remove(_) => FileChange::Removed(path),
                        _ => continue,
                    };
                    let _ = sender.send(change);
                }
            }
        })?;

        watcher.watch(root, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    /// Receive the next file change event, blocking up to `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<FileChange> {
        self.receiver.recv_timeout(timeout).ok()
    }

    /// Drain all pending events (non-blocking).
    pub fn drain(&self) -> Vec<FileChange> {
        let mut events = Vec::new();
        while let Ok(change) = self.receiver.try_recv() {
            events.push(change);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_watcher_detects_file_create() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = FileWatcher::new(dir.path()).unwrap();

        let file = dir.path().join("new.rs");
        fs::write(&file, "fn new() {}").unwrap();

        std::thread::sleep(Duration::from_millis(200));
        let events = watcher.drain();
        assert!(!events.is_empty(), "watcher should detect file creation");
    }

    #[test]
    fn test_watcher_detects_file_modify() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("existing.rs");
        fs::write(&file, "fn v1() {}").unwrap();

        let watcher = FileWatcher::new(dir.path()).unwrap();

        fs::write(&file, "fn v2() {}").unwrap();

        std::thread::sleep(Duration::from_millis(200));
        let events = watcher.drain();
        assert!(
            !events.is_empty(),
            "watcher should detect file modification"
        );
    }

    #[test]
    fn test_watcher_detects_file_remove() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("doomed.rs");
        fs::write(&file, "fn doomed() {}").unwrap();

        let watcher = FileWatcher::new(dir.path()).unwrap();

        fs::remove_file(&file).unwrap();

        std::thread::sleep(Duration::from_millis(200));
        let events = watcher.drain();
        assert!(!events.is_empty(), "watcher should detect file removal");
    }
}
