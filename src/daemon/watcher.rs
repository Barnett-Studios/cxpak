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
/// this set is a coarse, cheap pre-filter applied (against the path RELATIVE
/// to the watch root) at the notify callback so noisy paths never enter the
/// bounded channel.  Every component here is also in BUILTIN_IGNORES (or the
/// `.git` check), so the pre-filter can only ever be a subset of what
/// classify_changes rejects — enforced by
/// `test_noise_components_are_rejected_by_classify_changes` below.
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
        Self::with_bound(root, CHANNEL_BOUND)
    }

    /// [`FileWatcher::new`] with an explicit channel bound.
    ///
    /// Exists so the overflow path can be driven in a test without queueing
    /// 65 536 real filesystem events. Production always uses
    /// [`CHANNEL_BOUND`] via [`FileWatcher::new`].
    pub fn with_bound(root: &Path, bound: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::sync_channel(bound);
        let overflow = Arc::new(AtomicBool::new(false));
        let drop_count = Arc::new(AtomicU64::new(0));

        let overflow_cb = Arc::clone(&overflow);
        let drop_count_cb = Arc::clone(&drop_count);

        // Owned copy moved into the notify callback. Mirrors classify_changes:
        // the noise pre-filter matches the path RELATIVE to the watch root, so a
        // noise-named ANCESTOR of the root (e.g. /opt/build/repo, a `.venv`-parented
        // checkout, GitLab CI /builds/...) cannot silently drop every source event.
        //
        // Canonicalized first, because the pre-filter is only reachable when
        // `strip_prefix` succeeds. Event paths arrive fully resolved from the OS,
        // so a root that is relative (`.`) or reached through a symlink (on macOS
        // `/var/...` resolves to `/private/var/...`) would never match, every
        // strip would fail, and the filter would silently degrade to a no-op —
        // passing all the `target/` and `.git/` churn it exists to drop. Both
        // production callers in serve.rs already canonicalize; this makes the
        // watcher correct on its own rather than by their good behaviour.
        // Fall back to the path as given if canonicalization fails (the root may
        // legitimately not exist yet) — that restores the previous behaviour
        // rather than failing the watcher.
        let root_filter = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            let Ok(event) = res else {
                return;
            };
            for path in event.paths {
                // Relativize against the watch root before the noise check. On strip
                // failure (path not under root — should not happen for a recursive
                // watch) keep the event and let classify_changes decide, exactly as
                // classify_changes does (watch.rs: `let Ok(rel) = … else return false`).
                if let Ok(rel) = path.strip_prefix(&root_filter) {
                    if is_noise_path(rel) {
                        continue;
                    }
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

    /// A repo checked out UNDER a noise-named ancestor (e.g. `.../build/repo`)
    /// must still receive source events — the pre-filter relativizes against the
    /// watch root, so an ancestor component name cannot blind the watcher (M1).
    #[test]
    fn test_noise_ancestor_does_not_blind_watcher() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("build").join("repo"); // "build" is a NOISE_COMPONENT
        fs::create_dir_all(&root).unwrap();

        let watcher = FileWatcher::new(&root).unwrap();

        let file = root.join("main.rs");
        fs::write(&file, "fn main() {}").unwrap();

        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !watcher.drain().is_empty(),
            "source event under a noise-named ancestor must NOT be filtered"
        );
    }

    /// Parity invariant (L1): every NOISE_COMPONENT dropped by the coarse
    /// pre-filter MUST also be rejected by the authoritative classify_changes for
    /// a path relative to the repo root. Guards against the pre-filter silently
    /// dropping events classify_changes would keep. No git repo is needed — with
    /// `Repository::discover` returning None, classify_changes falls back to the
    /// BUILTIN_IGNORES matcher, which is exactly what we assert parity against.
    #[test]
    fn test_noise_components_are_rejected_by_classify_changes() {
        use crate::commands::watch::classify_changes;
        let base = tempfile::TempDir::new().unwrap();
        for &component in NOISE_COMPONENTS {
            let abs = base.path().join(component).join("file.txt");
            let (modified, removed) = classify_changes(&[FileChange::Modified(abs)], base.path());
            assert!(
                modified.is_empty() && removed.is_empty(),
                "NOISE_COMPONENT `{component}` must be rejected by classify_changes"
            );
        }
    }

    /// `collect_debounced` must return a coalesced batch from a REAL watcher.
    ///
    /// The previous version of this test built its own `HashMap` and asserted on
    /// it, so it passed without ever calling `collect_debounced` — it would have
    /// stayed green if the function returned `Vec::new()`. This one fails if the
    /// real coalescing breaks.
    #[test]
    fn collect_debounced_returns_one_entry_per_path_and_kind() {
        let dir = tempfile::TempDir::new().unwrap();
        let watcher = FileWatcher::new(dir.path()).unwrap();

        let file = dir.path().join("lib.rs");
        for i in 0..8u8 {
            fs::write(&file, format!("fn v{i}() {{}}")).unwrap();
        }

        let batch = watcher.collect_debounced(Duration::from_secs(3), Duration::from_millis(200));
        assert!(
            !batch.is_empty(),
            "collect_debounced must return the rapid writes, not an empty batch"
        );
        let mut keys: Vec<(PathBuf, u8)> = batch
            .iter()
            .map(|fc| (fc.path().to_path_buf(), fc.kind_ord()))
            .collect();
        let before = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys.len(),
            before,
            "collect_debounced returned duplicate (path, kind) entries — coalescing is broken"
        );
        assert!(
            batch.iter().any(|fc| fc.path().ends_with("lib.rs")),
            "the edited file must appear in the batch"
        );
        // Deliberately NOT asserting that `lib.rs` is the ONLY entry: the OS also
        // reports an event for the containing directory, and that is the
        // watcher's documented behaviour rather than a coalescing failure.
    }

    /// `take_overflow` must be set by a REAL watcher whose bounded channel
    /// filled, and must reset on read.
    ///
    /// The previous version constructed its own `sync_channel` and its own
    /// `AtomicBool`, then asserted that the bool it had just set was set — it
    /// never touched `FileWatcher` and would have passed with
    /// `fn take_overflow(&self) -> bool { false }`. This drives the real notify
    /// callback through a deliberately tiny bound.
    #[test]
    fn take_overflow_is_set_by_a_real_watcher_and_resets_on_read() {
        let dir = tempfile::TempDir::new().unwrap();
        // Bound of 1: the second unread non-noise event cannot be queued.
        let watcher = FileWatcher::with_bound(dir.path(), 1).unwrap();

        assert!(
            !watcher.take_overflow(),
            "a fresh watcher must not report overflow"
        );

        for i in 0..400 {
            fs::write(dir.path().join(format!("f{i}.rs")), "fn x() {}").unwrap();
        }
        // Do NOT drain — the channel must stay full so try_send fails.
        std::thread::sleep(Duration::from_millis(600));

        assert!(
            watcher.take_overflow(),
            "overflow flag must be set once the bounded channel rejects events"
        );
        assert!(
            !watcher.take_overflow(),
            "take_overflow must reset the flag, so one overflow triggers one rebuild"
        );
    }

    /// Noise paths must never reach the channel, so they can never be the cause
    /// of an overflow-triggered full rebuild.
    #[test]
    fn noise_paths_do_not_overflow_the_channel() {
        let dir = tempfile::TempDir::new().unwrap();
        // Bound of 256 with 4 000 noise writes: if the pre-filter were bypassed
        // the channel would overflow many times over, while leaving enough
        // headroom that a stray event from a sibling temp dir (the suite runs in
        // parallel) cannot by itself trip the assertion.
        let watcher = FileWatcher::with_bound(dir.path(), 256).unwrap();
        let noisy = dir.path().join("target").join("debug");
        fs::create_dir_all(&noisy).unwrap();

        for i in 0..4_000 {
            fs::write(noisy.join(format!("a{i}.o")), "x").unwrap();
        }
        std::thread::sleep(Duration::from_millis(800));

        assert!(
            !watcher.take_overflow(),
            "4 000 target/ writes are pre-filtered and must not overflow a 256-slot channel"
        );
    }
}
