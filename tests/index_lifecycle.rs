//! Adversarial tests for the v2.1.2 architectural changes:
//!
//! - `Arc<RwLock<Arc<CodebaseIndex>>>` snapshot-then-swap pattern: readers
//!   take an O(1) snapshot, drop the lock, and run handlers against the
//!   snapshot.  Watcher writes build a new index off a clone, then atomically
//!   swap the inner Arc.
//! - `health_cached()`: lazy-fill OnceLock so polling /v1/health doesn't
//!   re-run the 5 scoring passes per request.  Invalidated by watcher tick.
#![cfg(feature = "lsp")]

use std::sync::{Arc, OnceLock, RwLock};

#[test]
fn shared_index_inner_arc_clones_in_constant_time() {
    // The double-Arc pattern's value: a snapshot is a cheap atomic
    // refcount bump, NOT a deep clone.  Compare pointer identity to
    // confirm clones share the underlying CodebaseIndex.
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let inner: Arc<CodebaseIndex> = Arc::new(idx);
    let shared: Arc<RwLock<Arc<CodebaseIndex>>> = Arc::new(RwLock::new(Arc::clone(&inner)));

    let snap1 = {
        let g = shared.read().unwrap();
        Arc::clone(&*g)
    };
    let snap2 = {
        let g = shared.read().unwrap();
        Arc::clone(&*g)
    };
    assert!(
        Arc::ptr_eq(&snap1, &snap2),
        "two snapshots taken without a write must share the same allocation"
    );
    assert!(
        Arc::ptr_eq(&snap1, &inner),
        "snapshot must be the same Arc as the original (no deep clone)"
    );
}

#[test]
fn shared_index_swap_does_not_disturb_in_flight_snapshot() {
    // Reader takes a snapshot.  Writer swaps in a new Arc.  The reader's
    // snapshot must remain valid and unchanged — this is the property
    // that lets long-running LSP handlers run lock-free without being
    // disturbed by a concurrent watcher write.
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::scanner::ScannedFile;
    let counter = TokenCounter::new();
    let original = CodebaseIndex::build_with_content(
        vec![ScannedFile {
            relative_path: "src/orig.rs".into(),
            absolute_path: "/tmp/src/orig.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        }],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let original_total = original.total_files;
    let shared: Arc<RwLock<Arc<CodebaseIndex>>> = Arc::new(RwLock::new(Arc::new(original)));

    // Reader takes snapshot.
    let snapshot = {
        let g = shared.read().unwrap();
        Arc::clone(&*g)
    };
    assert_eq!(snapshot.total_files, original_total);

    // Writer swaps in a brand-new index with different shape.
    let new_idx = CodebaseIndex::build_with_content(
        (0..5)
            .map(|i| ScannedFile {
                relative_path: format!("src/new_{i}.rs"),
                absolute_path: format!("/tmp/src/new_{i}.rs").into(),
                language: Some("rust".into()),
                size_bytes: 0,
            })
            .collect(),
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let new_total = new_idx.total_files;
    {
        let mut g = shared.write().unwrap();
        *g = Arc::new(new_idx);
    }

    // Reader's snapshot still sees the OLD state — no torn read.
    assert_eq!(
        snapshot.total_files, original_total,
        "in-flight snapshot must NOT see the post-swap state — that's the whole point of the pattern"
    );
    // A fresh snapshot sees the new state.
    let after = {
        let g = shared.read().unwrap();
        Arc::clone(&*g)
    };
    assert_eq!(after.total_files, new_total);
}

#[test]
fn process_watcher_changes_invalidates_health_cache() {
    // Mirror of process_watcher_changes_invalidates_dead_code_cache from
    // round3_hardening.rs — the watcher tick MUST reset both per-index
    // caches (dead_code_cache AND health_cache) when it builds a new
    // CodebaseIndex.  Otherwise the SPA dashboard / v1/health would
    // serve pre-edit metrics for the lifetime of the process.
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("foo.rs"), "fn one() {}\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "t@t"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init", "--quiet"])
        .current_dir(dir.path())
        .output();

    let idx = cxpak::commands::serve::build_index(dir.path()).expect("build_index");
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
    // Prime the health cache.
    {
        let snap = {
            let g = shared.read().unwrap();
            std::sync::Arc::clone(&*g)
        };
        let _ = snap.health_cached();
        assert!(
            snap.health_cache.get().is_some(),
            "health cache must populate on first read"
        );
    }
    // Add a new file via the watcher path.
    let new_path = dir.path().join("bar.rs");
    std::fs::write(&new_path, "fn two() {}\n").unwrap();
    let change = cxpak::daemon::watcher::FileChange::Created(new_path);
    cxpak::commands::serve::process_watcher_changes(&[change], dir.path(), &shared);
    // The watcher swap-installed a NEW CodebaseIndex Arc.  Its
    // health_cache must be a fresh OnceLock (i.e., empty until the
    // next caller asks).
    {
        let g = shared.read().unwrap();
        assert!(
            g.health_cache.get().is_none(),
            "process_watcher_changes must reset health_cache on the new Arc"
        );
        assert!(
            g.dead_code_cache.get().is_none(),
            "process_watcher_changes must also reset dead_code_cache"
        );
    }
}

#[test]
fn health_cached_returns_stable_pointer_across_calls() {
    // Same contract as dead_code_cached — repeat reads return the same
    // backing allocation, NOT a recomputed value.
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let p1: *const _ = idx.health_cached();
    let p2: *const _ = idx.health_cached();
    let p3: *const _ = idx.health_cached();
    assert_eq!(p1, p2, "second call must return the cached pointer");
    assert_eq!(p2, p3, "third call must return the same cached pointer");
}

#[test]
fn health_cached_matches_compute_health() {
    // The cache MUST return the same value compute_health would have
    // returned — no silent divergence allowed.
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let cached = idx.health_cached();
    let direct = cxpak::intelligence::health::compute_health(&idx);
    assert_eq!(
        cached.composite.to_bits(),
        direct.composite.to_bits(),
        "cached composite must equal compute_health composite (bit-identical)"
    );
}

#[test]
fn health_cache_field_is_arc_oncelock_holder() {
    // Type-level pin: a future refactor that removes the cache or
    // changes its type must update this test, ensuring the contract
    // change is visible.  OnceLock allows lock-free first-write on the
    // hot path; Arc allows clones to share the cache.
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::intelligence::health::HealthScore;
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    fn _assert_type<T: 'static>(_: &T) {}
    let h: &Arc<OnceLock<HealthScore>> = &idx.health_cache;
    _assert_type(h);
}
