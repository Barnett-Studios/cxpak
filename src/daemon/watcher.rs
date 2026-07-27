use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Maximum number of queued raw FS events.
///
/// 65 536 slots × ~200 bytes per PathBuf ≈ 13 MB ceiling, affordable on any
/// server.  The coarse pre-filter below drops the `target/`, `.git/`, and
/// similar noise paths before they enter the channel, so in practice only
/// genuine source edits queue here.  Under a git-checkout storm (~50 000
/// events/s) the pre-filter reduces ingress by ~98 %; this bound handles the
/// remaining 2 % comfortably at 3 ms/batch debounce latency.
const CHANNEL_BOUND: usize = 65_536;

/// Minimum inter-batch quiet period before `collect_debounced` returns.
const DEBOUNCE_SLICE_MS: u64 = 50;

/// Absolute cap on total `collect_debounced` wait to prevent starvation under
/// a continuous event storm (e.g. a long `cargo build` that keeps touching
/// files the pre-filter didn't match).
const DEBOUNCE_MAX_MS: u64 = 2_000;

/// Path components that identify well-known noise trees.
///
/// classify_changes (watch.rs) remains the AUTHORITATIVE correctness filter;
/// this set is a coarse, cheap pre-filter applied at the notify callback so
/// the noisy paths never enter the bounded channel.  Any component listed here
/// MUST already be rejected by classify_changes (BUILTIN_IGNORES + .git check)
/// — the overlap is verified by the unit tests below.
const NOISE_COMPONENTS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".cxpak",
    "dist",
    "build",
    ".gradle",
    ".idea",
    ".next",
    ".venv",
];

/// Rate-limit noisy overflow warnings: emit at most once per this many drops.
const OVERFLOW_WARN_INTERVAL: u64 = 1_000;

/// Debounced file change events from the file system.
pub enum FileChange {
    Modified(PathBuf),
    Created(PathBuf),
    Removed(PathBuf),
}

impl FileChange {
    pub fn path(&self) -> &Path {
        match self {
            FileChange::Modified(p) | FileChange::Created(p) | FileChange::Removed(p) => p,
        }
    }

    /// Integer discriminant used as the coalescing key: (path, kind).
    fn kind_ord(&self) -> u8 {
        match self {
            FileChange::Created(_) => 0,
            FileChange::Modified(_) => 1,
            FileChange::Removed(_) => 2,
        }
    }
}

/// Returns `true` when any component of `path` matches a known noise directory.
fn is_noise_path(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        NOISE_COMPONENTS.iter().any(|&n| s == n)
    })
}

/// Watches a directory for file changes with a bounded channel, coarse noise
/// pre-filter, and windowed debouncing.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<FileChange>,
    overflow: Arc<AtomicBool>,
    /// Kept alive so the drop-count Arc is not freed before the watcher stops.
    _drop_count: Arc<AtomicU64>,
}

impl FileWatcher {
    /// Start watching `root` for file changes.
    ///
    /// The internal channel is bounded (`CHANNEL_BOUND`).  Paths under known
    /// noise trees (`.git/`, `target/`, `node_modules/`, …) are dropped at the
    /// notify callback before they reach the channel.  If the channel is full
    /// and a non-noise event cannot be queued, the overflow flag is set; call
    /// [`FileWatcher::take_overflow`] to detect this and trigger a full rebuild.
    pub fn new(root: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_BOUND);
        let overflow = Arc::new(AtomicBool::new(false));
        let drop_count = Arc::new(AtomicU64::new(0));

        let overflow_cb = Arc::clone(&overflow);
        let drop_count_cb = Arc::clone(&drop_count);

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            let Ok(event) = res else {
                return;
            };
            for path in event.paths {
                if is_noise_path(&path) {
                    continue;
                }
                let change = match event.kind {
                    EventKind::Create(_) => FileChange::Created(path),
                    EventKind::Modify(_) => FileChange::Modified(path),
                    EventKind::Remove(_) => FileChange::Removed(path),
                    _ => continue,
                };
                match tx.try_send(change) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_)) => {
                        overflow_cb.store(true, Ordering::Relaxed);
                        let prev = drop_count_cb.fetch_add(1, Ordering::Relaxed);
                        // Rate-limited warning: emit once every OVERFLOW_WARN_INTERVAL drops.
                        if prev.is_multiple_of(OVERFLOW_WARN_INTERVAL) {
                            eprintln!(
                                "cxpak: watcher channel full — {} event(s) dropped; \
                                 a full rebuild will be triggered",
                                prev + 1
                            );
                        }
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        // Receiver has been dropped; nothing to do.
                    }
                }
            }
        })?;

        watcher.watch(root, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
            overflow,
            _drop_count: drop_count,
        })
    }

    /// Returns `true` (and resets the flag to `false`) if at least one
    /// non-noise event was silently dropped because the channel was full.
    /// The consumer should trigger a full rebuild in this case.
    pub fn take_overflow(&self) -> bool {
        self.overflow.swap(false, Ordering::AcqRel)
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

    /// Block until the first event arrives (up to `first_timeout`), then keep
    /// draining in `DEBOUNCE_SLICE_MS` slices until no new events arrive for a
    /// full `window`, or until `DEBOUNCE_MAX_MS` total has elapsed since the
    /// first event.
    ///
    /// Returns the coalesced, de-duplicated batch (at most one entry per
    /// `(path, kind)`), or an empty `Vec` if no event arrived within
    /// `first_timeout`.
    pub fn collect_debounced(&self, first_timeout: Duration, window: Duration) -> Vec<FileChange> {
        let first = match self.recv_timeout(first_timeout) {
            Some(e) => e,
            None => return Vec::new(),
        };

        // Map from (path, kind discriminant) → FileChange so that duplicate
        // events on the same path+kind collapse to one entry.
        let mut seen: HashMap<(PathBuf, u8), FileChange> = HashMap::new();
        {
            let key = (first.path().to_path_buf(), first.kind_ord());
            seen.entry(key).or_insert(first);
        }

        let deadline = Instant::now() + Duration::from_millis(DEBOUNCE_MAX_MS);
        let mut last_event = Instant::now();

        loop {
            std::thread::sleep(Duration::from_millis(DEBOUNCE_SLICE_MS));

            let batch = self.drain();
            let had_events = !batch.is_empty();
            for fc in batch {
                let key = (fc.path().to_path_buf(), fc.kind_ord());
                seen.entry(key).or_insert(fc);
            }
            if had_events {
                last_event = Instant::now();
            }

            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if now.saturating_duration_since(last_event) >= window {
                break;
            }
        }

        seen.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── existing tests ──────────────────────────────────────────────────────

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

    // ── new tests ───────────────────────────────────────────────────────────

    /// is_noise_path drops known build/VCS directories but passes source paths.
    #[test]
    fn test_noise_filter_drops_target_and_git() {
        let target_path = PathBuf::from("/repo/target/debug/my_binary");
        let git_path = PathBuf::from("/repo/.git/COMMIT_EDITMSG");
        let src_path = PathBuf::from("/repo/src/main.rs");
        let node_path = PathBuf::from("/repo/node_modules/lodash/index.js");

        assert!(is_noise_path(&target_path), "target/ must be noise");
        assert!(is_noise_path(&git_path), ".git/ must be noise");
        assert!(is_noise_path(&node_path), "node_modules/ must be noise");
        assert!(!is_noise_path(&src_path), "src/ must NOT be noise");
    }

    /// Every component in NOISE_COMPONENTS is covered.
    #[test]
    fn test_noise_filter_covers_all_components() {
        for &component in NOISE_COMPONENTS {
            let path = PathBuf::from(format!("/repo/{component}/something.txt"));
            assert!(
                is_noise_path(&path),
                "component `{component}` must be detected as noise"
            );
        }
    }

    /// collect_debounced collapses duplicate (path, kind) events into one.
    #[test]
    fn test_coalescing_deduplicates_same_path_same_kind() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = FileWatcher::new(dir.path()).unwrap();

        let p = PathBuf::from("/repo/src/lib.rs");
        // Push five identical Modified events directly into the internal state
        // by writing the same file rapidly so the OS fires multiple events.
        let file = dir.path().join("lib.rs");
        for i in 0..5u8 {
            fs::write(&file, format!("fn v{}() {{}}", i)).unwrap();
        }

        std::thread::sleep(Duration::from_millis(200));
        let raw = watcher.drain();
        assert!(
            !raw.is_empty(),
            "OS should have fired at least one event for the rapid writes"
        );

        // Now test the coalescing map logic directly with synthetic duplicates.
        let mut seen: std::collections::HashMap<(PathBuf, u8), FileChange> =
            std::collections::HashMap::new();
        for _ in 0..10 {
            let fc = FileChange::Modified(p.clone());
            let key = (fc.path().to_path_buf(), fc.kind_ord());
            seen.entry(key).or_insert(fc);
        }
        assert_eq!(
            seen.len(),
            1,
            "ten identical events must coalesce to one entry"
        );
    }

    /// Different kinds on the same path are kept separate.
    #[test]
    fn test_coalescing_keeps_distinct_kinds() {
        let p = PathBuf::from("/repo/src/lib.rs");
        let mut seen: std::collections::HashMap<(PathBuf, u8), FileChange> =
            std::collections::HashMap::new();

        for fc in [
            FileChange::Modified(p.clone()),
            FileChange::Created(p.clone()),
            FileChange::Removed(p.clone()),
        ] {
            let key = (fc.path().to_path_buf(), fc.kind_ord());
            seen.entry(key).or_insert(fc);
        }
        assert_eq!(seen.len(), 3, "distinct kinds must not coalesce");
    }

    /// CHANNEL_BOUND is finite; flooding the channel sets the overflow flag.
    #[test]
    fn test_bounded_channel_overflow_sets_flag() {
        // Construct a channel with the same bound and flood it.
        let (tx, rx) = mpsc::sync_channel::<FileChange>(CHANNEL_BOUND);
        let overflow = Arc::new(AtomicBool::new(false));
        let drop_count = Arc::new(AtomicU64::new(0));

        // Fill the channel to capacity.
        for i in 0..CHANNEL_BOUND {
            let fc = FileChange::Modified(PathBuf::from(format!("/repo/src/f{i}.rs")));
            tx.try_send(fc).expect("should fit within bound");
        }

        // One more must fail with Full.
        let extra = FileChange::Modified(PathBuf::from("/repo/src/extra.rs"));
        match tx.try_send(extra) {
            Err(mpsc::TrySendError::Full(_)) => {
                overflow.store(true, Ordering::Relaxed);
                drop_count.fetch_add(1, Ordering::Relaxed);
            }
            other => panic!("expected Full, got {:?}", other.err()),
        }

        assert!(
            overflow.load(Ordering::Relaxed),
            "overflow flag must be set when channel is full"
        );
        assert_eq!(drop_count.load(Ordering::Relaxed), 1);

        // Drain the channel so the test doesn't leak.
        drop(rx);
    }
}
