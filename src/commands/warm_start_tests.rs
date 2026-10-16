//! Warm-start behaviour of `build_index` (ADR-0210, spec 2026-10-15-warm-start).
//!
//! Every test drives the real build against a real git repo in a tempdir and
//! asserts on the returned `BuildReport` and index — never on stderr.

use super::*;
use crate::cache::artifacts::{ArtifactStore, ARTIFACT_STORE_FILE};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const FIXED_TIME: i64 = 1_700_000_000;

fn commit_all(root: &Path, message: &str) {
    commit_all_at(root, message, FIXED_TIME);
}

fn commit_all_at(root: &Path, message: &str, time: i64) {
    let repo = git2::Repository::open(root).expect("open repo");
    let mut index = repo.index().expect("repo index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("git add");
    index.write().expect("index write");
    let tree = repo
        .find_tree(index.write_tree().expect("write tree"))
        .expect("find tree");
    let sig = git2::Signature::new("t", "t@t", &git2::Time::new(time, 0)).expect("sig");
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .expect("commit");
}

/// Two Rust files with a call between them, an HTTP bridge (TS → FastAPI) so
/// a cross-language edge exists, and a markdown file with frontmatter.
fn warm_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    write_warm_files(dir.path());
    git2::Repository::init(dir.path()).expect("git init");
    commit_all(dir.path(), "init");
    dir
}

fn write_warm_files(root: &Path) {
    for (rel, body) in [
        ("src/a.rs", "pub fn helper() -> i32 {\n    1\n}\n"),
        (
            "src/b.rs",
            "use crate::a::helper;\npub fn go() -> i32 {\n    helper()\n}\n",
        ),
        (
            "frontend/api.ts",
            "async function getUsers() { return fetch(\"/api/users\"); }\n",
        ),
        (
            "backend/users.py",
            "from fastapi import FastAPI\n@app.get(\"/api/users\")\ndef get_users():\n    return []\n",
        ),
        ("docs/note.md", "---\ntitle: Note\n---\n# Note\nhelper docs\n"),
    ] {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

fn store_dir(root: &Path) -> PathBuf {
    root.join(".cxpak").join("cache")
}

fn derived_path(root: &Path) -> PathBuf {
    root.join(".cxpak")
        .join("cache")
        .join("root")
        .join("derived.json")
}

fn load_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn save_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_string(value).unwrap()).unwrap();
}

/// Rewrite `rel` with different bytes of the same length and put its mtime
/// back, so neither size nor mtime betrays the edit.
fn same_length_edit_restoring_mtime(root: &Path, rel: &str, from: &str, to: &str) {
    assert_eq!(from.len(), to.len());
    let path = root.join(rel);
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let body = std::fs::read_to_string(&path).unwrap().replace(from, to);
    std::fs::write(&path, body).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

/// Sort every JSON array so container order (HashMap/HashSet iteration) does
/// not decide equality; file order is asserted separately.
fn norm(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            let mut items: Vec<_> = items.into_iter().map(norm).collect();
            items.sort_by_key(|v| v.to_string());
            serde_json::Value::Array(items)
        }
        serde_json::Value::Object(map) => {
            serde_json::Value::Object(map.into_iter().map(|(k, v)| (k, norm(v))).collect())
        }
        other => other,
    }
}

/// Every field spec C1 requires to be identical between cold and warm builds.
fn snapshot(idx: &CodebaseIndex) -> serde_json::Value {
    let files: Vec<_> = idx
        .files
        .iter()
        .map(|f| {
            json!({
                "path": f.relative_path,
                "language": f.language,
                "size": f.size_bytes,
                "tokens": f.token_count,
                "parse": f.parse_result,
                "content": f.content,
                "mtime_ns": f.mtime_ns,
            })
        })
        .collect();
    let pagerank: BTreeMap<_, _> = idx
        .pagerank
        .iter()
        .map(|(k, v)| (k.clone(), v.to_bits()))
        .collect();
    let mut domains: Vec<String> = idx.domains.iter().map(|d| format!("{d:?}")).collect();
    domains.sort();
    let language_stats: BTreeMap<_, _> = idx
        .language_stats
        .iter()
        .map(|(k, s)| (k.clone(), (s.file_count, s.total_bytes, s.total_tokens)))
        .collect();
    norm(json!({
        "files": files,
        "term_frequencies": idx.term_frequencies,
        "graph": idx.graph,
        "pagerank": pagerank,
        "call_graph": idx.call_graph,
        "cross_lang_edges": idx.cross_lang_edges,
        "test_map": idx.test_map,
        "schema": idx.schema,
        "domains": domains,
        "conventions": idx.conventions,
        "co_changes": idx.co_changes,
        "totals": [idx.total_files, idx.total_tokens, idx.total_bytes],
        "language_stats": language_stats,
    }))
}

fn symbol_names(idx: &CodebaseIndex, rel: &str) -> Vec<String> {
    idx.files
        .iter()
        .find(|f| f.relative_path == rel)
        .and_then(|f| f.parse_result.as_ref())
        .map(|p| p.symbols.iter().map(|s| s.name.clone()).collect())
        .unwrap_or_default()
}

fn plant_store_sentinel(root: &Path, rel: &str) {
    let path = store_dir(root).join(ARTIFACT_STORE_FILE);
    let mut store = load_json(&path);
    store["entries"][rel]["parse_result"]["symbols"][0]["name"] = json!("SENTINEL_FROM_STORE");
    save_json(&path, &store);
}

#[test]
fn warm_equals_cold_on_derived_hit() {
    let dir = warm_repo();
    let (cold, r1) = build_index_reported(dir.path(), None).unwrap();
    assert!(!r1.derived_hit);
    assert_eq!(r1.artifact_misses, cold.total_files);
    assert!(
        !cold.cross_lang_edges.is_empty(),
        "fixture must exercise cross-language edges"
    );

    let (warm, r2) = build_index_reported(dir.path(), None).unwrap();
    assert!(r2.derived_hit, "unchanged repo must hit the derived cache");
    assert_eq!(r2.artifact_hits, warm.total_files);
    assert_eq!(r2.artifact_misses, 0);
    assert_eq!(snapshot(&cold), snapshot(&warm));
}

#[test]
fn warm_equals_cold_on_derived_miss() {
    let dir = warm_repo();
    let (cold, _) = build_index_reported(dir.path(), None).unwrap();
    std::fs::remove_file(derived_path(dir.path())).unwrap();

    let (warm, r2) = build_index_reported(dir.path(), None).unwrap();
    assert!(!r2.derived_hit);
    assert_eq!(r2.artifact_hits, warm.total_files);
    assert_eq!(snapshot(&cold), snapshot(&warm));
}

#[test]
fn artifact_reused_by_content_hash() {
    let dir = warm_repo();
    build_index_reported(dir.path(), None).unwrap();
    plant_store_sentinel(dir.path(), "src/a.rs");

    let (warm, _) = build_index_reported(dir.path(), None).unwrap();
    assert!(
        symbol_names(&warm, "src/a.rs").contains(&"SENTINEL_FROM_STORE".to_string()),
        "a hash-matching store entry must be reused, not recomputed"
    );
}

#[test]
fn stale_hash_not_reused() {
    let dir = warm_repo();
    build_index_reported(dir.path(), None).unwrap();
    plant_store_sentinel(dir.path(), "src/a.rs");
    same_length_edit_restoring_mtime(dir.path(), "src/a.rs", "helper", "helpez");

    let (warm, r2) = build_index_reported(dir.path(), None).unwrap();
    let names = symbol_names(&warm, "src/a.rs");
    assert!(!names.contains(&"SENTINEL_FROM_STORE".to_string()));
    assert!(names.contains(&"helpez".to_string()), "got {names:?}");
    assert_eq!(r2.artifact_misses, 1);
}

#[test]
fn same_length_edit_with_restored_mtime_misses_derived() {
    let dir = warm_repo();
    build_index_reported(dir.path(), None).unwrap();
    same_length_edit_restoring_mtime(dir.path(), "src/a.rs", "helper", "helpez");

    let (_, r2) = build_index_reported(dir.path(), None).unwrap();
    assert!(
        !r2.derived_hit,
        "the derived fingerprint must come from content, not (mtime, size)"
    );
}

#[test]
fn derived_hit_does_not_rebuild_call_graph() {
    use crate::intelligence::call_graph::BUILD_CALL_GRAPH_CALLS;
    let dir = warm_repo();
    BUILD_CALL_GRAPH_CALLS.with(|c| c.set(0));
    let (cold, _) = build_index_reported(dir.path(), None).unwrap();
    assert!(BUILD_CALL_GRAPH_CALLS.with(|c| c.get()) >= 1);

    let path = derived_path(dir.path());
    let mut derived = load_json(&path);
    derived["call_graph"]["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "caller_file": "src/b.rs",
            "caller_symbol": "SENTINEL_CALLER",
            "callee_file": "src/a.rs",
            "callee_symbol": "helper",
            "confidence": "Exact",
        }));
    save_json(&path, &derived);

    BUILD_CALL_GRAPH_CALLS.with(|c| c.set(0));
    let (warm, r2) = build_index_reported(dir.path(), None).unwrap();
    assert!(r2.derived_hit);
    assert_eq!(
        BUILD_CALL_GRAPH_CALLS.with(|c| c.get()),
        0,
        "a derived hit must not compute the call graph it restores"
    );
    assert!(warm
        .call_graph
        .edges
        .iter()
        .any(|e| e.caller_symbol == "SENTINEL_CALLER"));
    assert_eq!(
        serde_json::to_value(&warm.cross_lang_edges).unwrap(),
        serde_json::to_value(&cold.cross_lang_edges).unwrap()
    );
}

#[test]
fn unreadable_and_over_cap_leave_no_trace() {
    let dir = warm_repo();
    let root = dir.path();
    std::fs::write(root.join("src/bad.rs"), [0xff_u8, 0xfe, 0x00, 0x41]).unwrap();
    let cap = crate::index::max_file_bytes() as usize;
    let mut big = String::from("// ");
    big.push_str(&"a".repeat(cap));
    std::fs::write(root.join("src/big.rs"), big).unwrap();

    let (idx, _) = build_index_reported(root, None).unwrap();
    let paths: Vec<_> = idx.files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(!paths.contains(&"src/bad.rs") && !paths.contains(&"src/big.rs"));
    assert_eq!(idx.total_files, paths.len());
    let store = ArtifactStore::load(&store_dir(root));
    assert!(!store.entries.contains_key("src/bad.rs"));
    assert!(!store.entries.contains_key("src/big.rs"));
    assert_eq!(store.entries.len(), idx.total_files);
}

#[test]
fn store_failure_is_not_an_indexing_error() {
    // Truncated store.
    let dir = warm_repo();
    let (cold, _) = build_index_reported(dir.path(), None).unwrap();
    let store_file = store_dir(dir.path()).join(ARTIFACT_STORE_FILE);
    let bytes = std::fs::read(&store_file).unwrap();
    std::fs::write(&store_file, &bytes[..bytes.len() / 3]).unwrap();
    let (idx, r) = build_index_reported(dir.path(), None).unwrap();
    assert_eq!(r.artifact_misses, idx.total_files);
    assert_eq!(idx.total_files, cold.total_files);

    // Version mismatch.
    let mut store = load_json(&store_file);
    store["version"] = json!(9999);
    save_json(&store_file, &store);
    let (idx, r) = build_index_reported(dir.path(), None).unwrap();
    assert_eq!(r.artifact_misses, idx.total_files);
}

#[cfg(unix)]
#[test]
fn unwritable_cache_dir_is_not_an_indexing_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = warm_repo();
    let cache = store_dir(dir.path());
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = build_index_reported(dir.path(), None);
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (idx, r) = result.expect("an unwritable cache must not fail indexing");
    assert_eq!(r.artifact_misses, idx.total_files);
    assert!(!r.store_saved);
}

#[test]
fn workspace_build_keeps_foreign_entries_then_full_build_prunes() {
    let dir = warm_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    build_index_reported(root, Some("src")).unwrap();
    let store = ArtifactStore::load(&store_dir(root));
    assert!(store.entries.contains_key("frontend/api.ts"));
    assert!(store.entries.contains_key("backend/users.py"));

    std::fs::remove_file(root.join("backend/users.py")).unwrap();
    build_index_reported(root, None).unwrap();
    let store = ArtifactStore::load(&store_dir(root));
    assert!(!store.entries.contains_key("backend/users.py"));
    assert!(store.entries.contains_key("frontend/api.ts"));
}

#[test]
fn concurrent_builds_leave_valid_store() {
    let dir = warm_repo();
    let root = dir.path().to_path_buf();
    build_index_reported(&root, None).unwrap();
    std::fs::write(
        root.join("src/a.rs"),
        "pub fn helper() -> i32 {\n    42\n}\n",
    )
    .unwrap();

    let handles: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || build_index(&root).map(|i| i.total_files).ok())
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap().is_some());
    }

    let store = ArtifactStore::load(&store_dir(&root));
    assert!(!store.entries.is_empty());
    for (rel, entry) in &store.entries {
        let bytes = std::fs::read(root.join(rel)).unwrap();
        assert_eq!(
            entry.content_sha256,
            format!("{:x}", Sha256::digest(&bytes)),
            "entry for {rel} must describe the file's current bytes"
        );
    }
}

#[test]
fn cli_entry_is_partial_hit() {
    let dir = warm_repo();
    let root = dir.path();
    let files = Scanner::new(root).unwrap().scan_workspace(None).unwrap();
    let counter = TokenCounter::new();
    crate::cache::parse::parse_with_cache(&files, root, &counter, false);
    let cli_store = ArtifactStore::load(&store_dir(root));
    let cli_entries = cli_store.entries.len();
    assert!(cli_entries > 0);
    assert!(cli_store
        .entries
        .values()
        .all(|e| e.token_count.is_none() && e.term_frequencies.is_none()));
    plant_store_sentinel(root, "src/a.rs");

    let (idx, r) = build_index_reported(root, None).unwrap();
    assert_eq!(r.artifact_partial, cli_entries);
    assert!(symbol_names(&idx, "src/a.rs").contains(&"SENTINEL_FROM_STORE".to_string()));
    let a = idx
        .files
        .iter()
        .find(|f| f.relative_path == "src/a.rs")
        .unwrap();
    assert_eq!(a.token_count, counter.count_or_zero(&a.content));

    let filled = ArtifactStore::load(&store_dir(root));
    assert!(filled
        .entries
        .values()
        .all(|e| e.token_count.is_some() && e.term_frequencies.is_some()));
}

#[test]
fn warm_unchanged_writes_nothing() {
    let dir = warm_repo();
    let (_, r1) = build_index_reported(dir.path(), None).unwrap();
    assert!(r1.store_saved);
    let (_, r2) = build_index_reported(dir.path(), None).unwrap();
    assert!(
        !r2.store_saved,
        "an unchanged repo must not rewrite the store"
    );
    assert!(
        !store_dir(dir.path())
            .join("root")
            .join("stat_index.json")
            .exists(),
        "the index path no longer keeps an mtime-keyed stat-index"
    );
}

#[test]
fn files_keep_scanner_order() {
    let dir = warm_repo();
    let expected: Vec<String> = Scanner::new(dir.path())
        .unwrap()
        .scan_workspace(None)
        .unwrap()
        .into_iter()
        .map(|f| f.relative_path)
        .collect();
    assert!(expected.len() >= 5);
    let (idx, _) = build_index_reported(dir.path(), None).unwrap();
    let got: Vec<String> = idx.files.iter().map(|f| f.relative_path.clone()).collect();
    assert_eq!(got, expected);
    let unique: BTreeSet<_> = got.iter().collect();
    assert_eq!(unique.len(), got.len());
}

#[test]
fn cli_touch_preserves_complete_entry() {
    let dir = warm_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();

    let path = root.join("src/a.rs");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(10))
        .unwrap();
    let files = Scanner::new(root).unwrap().scan_workspace(None).unwrap();
    let counter = TokenCounter::new();
    crate::cache::parse::parse_with_cache(&files, root, &counter, false);

    let store = ArtifactStore::load(&store_dir(root));
    let entry = &store.entries["src/a.rs"];
    assert!(
        entry.token_count.is_some() && entry.term_frequencies.is_some(),
        "same bytes, new mtime: the CLI must carry the computed fields over"
    );

    let (idx, r) = build_index_reported(root, None).unwrap();
    assert_eq!(r.artifact_partial, 0);
    assert_eq!(r.artifact_hits, idx.total_files);
    assert_eq!(r.artifact_misses, 0);
}

// ─── Phase 2 (ADR-0212): conventions off the Ready path, cached call sites ───

use crate::cache::DerivedCache;
use crate::core_graph::conventions::ConventionsStatus;
use crate::intelligence::call_graph::PARSED_SOURCES;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// The warm fixture with commits dated now, so git-history mining (180-day
/// window) and co-change mining see real history. `.cxpak/` is ignored so the
/// cache never makes the working tree dirty (the derived cache's `base_commit`
/// is recorded only for a clean tree).
fn history_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    write_warm_files(root);
    std::fs::write(root.join(".gitignore"), ".cxpak/\n").unwrap();
    git2::Repository::init(root).expect("git init");
    commit_all_at(root, "init", now_secs());
    dir
}

/// A second commit touching two files together, so HEAD moves and the
/// derived fingerprint misses while every other artifact still hits.
fn second_commit(root: &Path) {
    let a = root.join("src/a.rs");
    let b = root.join("src/b.rs");
    std::fs::write(&a, "pub fn helper() -> i32 {\n    2\n}\n").unwrap();
    std::fs::write(
        &b,
        "use crate::a::helper;\npub fn go() -> i32 {\n    helper() + 1\n}\n",
    )
    .unwrap();
    commit_all_at(root, "fix helper", now_secs());
}

fn json_of<T: serde::Serialize>(value: &T) -> serde_json::Value {
    norm(serde_json::to_value(value).unwrap())
}

fn ns_dir(root: &Path) -> PathBuf {
    derived_path(root).parent().unwrap().to_path_buf()
}

fn rewrite_store(root: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = store_dir(root).join(ARTIFACT_STORE_FILE);
    let mut store = load_json(&path);
    edit(&mut store);
    save_json(&path, &store);
}

fn parsed_sources() -> Vec<String> {
    PARSED_SOURCES.with(|p| p.borrow().clone())
}

fn clear_parsed_sources() {
    PARSED_SOURCES.with(|p| p.borrow_mut().clear());
}

fn source_of(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn conventions_status_fresh_on_first_build() {
    let dir = history_repo();
    let (idx, _, mut pending) = build_index_pending_deferred(dir.path(), None).unwrap();
    assert_eq!(idx.conventions_status, ConventionsStatus::Fresh);
    assert!(
        pending.take_conventions().is_none(),
        "no prior derived cache: nothing to defer"
    );
    assert!(
        DerivedCache::load_for_delta(&ns_dir(dir.path())).is_some(),
        "the inline path saves the derived cache"
    );
}

#[test]
fn conventions_status_stale_pending_after_commit() {
    let dir = history_repo();
    let root = dir.path();
    let (first, _) = build_index_reported(root, None).unwrap();
    let head1 = git_head_oid(root);
    assert!(!head1.is_empty());
    second_commit(root);

    let (stale, report, mut pending) = build_index_pending_deferred(root, None).unwrap();
    assert!(!report.derived_hit);
    assert_eq!(
        stale.conventions_status,
        ConventionsStatus::StalePending { base_commit: head1 }
    );
    assert_eq!(json_of(&stale.conventions), json_of(&first.conventions));
    assert!(pending.take_conventions().is_some());
}

#[test]
fn stale_window_carries_prior_co_changes() {
    let dir = history_repo();
    let root = dir.path();
    let (first, _) = build_index_reported(root, None).unwrap();
    assert!(
        !first.co_changes.is_empty(),
        "fixture must have co-change history"
    );
    second_commit(root);

    let (stale, _, _pending) = build_index_pending_deferred(root, None).unwrap();
    assert!(matches!(
        stale.conventions_status,
        ConventionsStatus::StalePending { .. }
    ));
    assert!(
        !stale.co_changes.is_empty(),
        "conventions and co_changes are one derived unit"
    );
    assert_eq!(json_of(&stale.co_changes), json_of(&first.co_changes));
}

#[test]
fn conventions_republish_matches_cold() {
    let dir = history_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    second_commit(root);

    let (stale, _, mut pending) = build_index_pending_deferred(root, None).unwrap();
    let pending = pending.take_conventions().expect("stale build defers");
    let fingerprint = pending.fingerprint().to_string();
    let mut republished = stale.clone();
    pending.mine(&stale).apply_to(&mut republished);
    assert_eq!(republished.conventions_status, ConventionsStatus::Fresh);
    assert!(
        DerivedCache::load(&ns_dir(root), &fingerprint).is_some(),
        "the background phase saves the complete derived cache under the current fingerprint"
    );

    std::fs::remove_dir_all(root.join(".cxpak")).unwrap();
    let (cold, _) = build_index_reported(root, None).unwrap();
    assert_eq!(cold.conventions_status, ConventionsStatus::Fresh);
    assert_eq!(snapshot(&republished), snapshot(&cold));
}

#[test]
fn stale_build_never_saves_derived_under_current_fingerprint() {
    let dir = history_repo();
    let root = dir.path();
    let (_, r1) = build_index_reported(root, None).unwrap();
    assert!(!r1.derived_hit);
    let before = DerivedCache::load_for_delta(&ns_dir(root)).unwrap();
    second_commit(root);

    let (_stale, _, mut pending) = build_index_pending_deferred(root, None).unwrap();
    let pending = pending.take_conventions().expect("stale build defers");
    assert_ne!(pending.fingerprint(), before.fingerprint);
    assert!(
        DerivedCache::load(&ns_dir(root), pending.fingerprint()).is_none(),
        "an interrupted start must not leave stale conventions under the current fingerprint"
    );
    let after = DerivedCache::load_for_delta(&ns_dir(root)).unwrap();
    assert_eq!(after.fingerprint, before.fingerprint);
}

#[test]
fn conventions_inline_when_no_prior_cache() {
    let dir = history_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    std::fs::remove_file(derived_path(root)).unwrap();
    second_commit(root);

    let (idx, _, mut pending) = build_index_pending_deferred(root, None).unwrap();
    assert_eq!(idx.conventions_status, ConventionsStatus::Fresh);
    assert!(pending.take_conventions().is_none());
}

#[test]
fn conventions_load_for_delta_grammar_mismatch_falls_back_inline() {
    let dir = history_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    let mut derived = load_json(&derived_path(root));
    derived["grammar_hash"] = json!("not-this-build");
    save_json(&derived_path(root), &derived);
    assert!(DerivedCache::load_for_delta(&ns_dir(root)).is_none());
    second_commit(root);

    let (idx, _, mut pending) = build_index_pending_deferred(root, None).unwrap();
    assert_eq!(idx.conventions_status, ConventionsStatus::Fresh);
    assert!(pending.take_conventions().is_none());
}

#[test]
fn non_deferring_build_always_mines_inline() {
    let dir = history_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    second_commit(root);

    let (idx, _) = build_index_reported(root, None).unwrap();
    assert_eq!(idx.conventions_status, ConventionsStatus::Fresh);
    let saved = DerivedCache::load_for_delta(&ns_dir(root)).unwrap();
    assert_eq!(
        saved.base_commit.as_deref(),
        Some(git_head_oid(root).as_str())
    );
}

#[test]
fn cold_builds_are_deterministic() {
    let dir = history_repo();
    let root = dir.path();
    second_commit(root);
    let (one, _) = build_index_reported(root, None).unwrap();
    assert!(!one.co_changes.is_empty());
    std::fs::remove_dir_all(root.join(".cxpak")).unwrap();
    let (two, _) = build_index_reported(root, None).unwrap();

    assert_eq!(json_of(&one.conventions), json_of(&two.conventions));
    assert_eq!(json_of(&one.co_changes), json_of(&two.co_changes));
}

#[test]
fn call_sites_cache_hit_skips_reparse() {
    let dir = warm_repo();
    let root = dir.path();
    let (cold, _) = build_index_reported(root, None).unwrap();
    let store = ArtifactStore::load(&store_dir(root));
    assert!(
        store.entries["src/b.rs"].call_sites_by_symbol.is_some(),
        "the first build writes call sites back"
    );

    std::fs::remove_file(derived_path(root)).unwrap();
    clear_parsed_sources();
    let (warm, r) = build_index_reported(root, None).unwrap();
    assert!(!r.derived_hit);
    assert!(
        parsed_sources().is_empty(),
        "every call site came from the store, yet tree-sitter parsed {:?}",
        parsed_sources().len()
    );
    assert_eq!(json_of(&warm.call_graph), json_of(&cold.call_graph));

    rewrite_store(root, |store| {
        store["entries"]["src/b.rs"]["call_sites_by_symbol"] =
            json!({"go": ["helper", "SENTINEL_CALLEE"]});
    });
    std::fs::remove_file(derived_path(root)).unwrap();
    let (sentinel, _) = build_index_reported(root, None).unwrap();
    assert!(sentinel
        .call_graph
        .unresolved
        .iter()
        .any(|u| u.callee_name == "SENTINEL_CALLEE"));
}

#[test]
fn call_sites_cache_miss_on_content_change() {
    let dir = warm_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    let a_before = ArtifactStore::load(&store_dir(root)).entries["src/a.rs"].clone();
    std::fs::write(
        root.join("src/b.rs"),
        "use crate::a::helper;\npub fn go() -> i32 {\n    helper() + other()\n}\n",
    )
    .unwrap();
    std::fs::remove_file(derived_path(root)).unwrap();

    clear_parsed_sources();
    let (idx, _) = build_index_reported(root, None).unwrap();
    let parsed = parsed_sources();
    assert!(parsed.contains(&source_of(root, "src/b.rs")));
    assert!(!parsed.contains(&source_of(root, "src/a.rs")));
    assert!(idx
        .call_graph
        .unresolved
        .iter()
        .any(|u| u.callee_name == "other"));

    let store = ArtifactStore::load(&store_dir(root));
    let sites = store.entries["src/b.rs"]
        .call_sites_by_symbol
        .as_ref()
        .expect("recomputed call sites are written back");
    assert!(sites["go"].contains(&"other".to_string()), "{sites:?}");
    assert_eq!(store.entries["src/a.rs"], a_before);
}

#[test]
fn call_sites_cache_miss_on_language_change() {
    let dir = warm_repo();
    let root = dir.path();
    build_index_reported(root, None).unwrap();
    rewrite_store(root, |store| {
        store["entries"]["src/b.rs"]["language"] = json!("python");
        store["entries"]["src/b.rs"]["call_sites_by_symbol"] =
            json!({"go": ["helper", "SENTINEL_CALLEE"]});
    });
    std::fs::remove_file(derived_path(root)).unwrap();

    clear_parsed_sources();
    let (idx, _) = build_index_reported(root, None).unwrap();
    assert!(!idx
        .call_graph
        .unresolved
        .iter()
        .any(|u| u.callee_name == "SENTINEL_CALLEE"));
    assert!(parsed_sources().contains(&source_of(root, "src/b.rs")));
    let store = ArtifactStore::load(&store_dir(root));
    assert_eq!(store.entries["src/b.rs"].language.as_deref(), Some("rust"));
    assert!(!store.entries["src/b.rs"]
        .call_sites_by_symbol
        .as_ref()
        .unwrap()
        .values()
        .flatten()
        .any(|c| c == "SENTINEL_CALLEE"));
}

#[test]
fn call_sites_none_and_v1_store_are_per_file_misses() {
    let dir = warm_repo();
    let root = dir.path();
    let (cold, _) = build_index_reported(root, None).unwrap();

    rewrite_store(root, |store| {
        for entry in store["entries"].as_object_mut().unwrap().values_mut() {
            entry["call_sites_by_symbol"] = serde_json::Value::Null;
        }
    });
    std::fs::remove_file(derived_path(root)).unwrap();
    clear_parsed_sources();
    let (idx, r) = build_index_reported(root, None).unwrap();
    assert_eq!(
        r.artifact_hits, idx.total_files,
        "per-file artifacts still hit"
    );
    assert!(parsed_sources().contains(&source_of(root, "src/a.rs")));
    assert_eq!(json_of(&idx.call_graph), json_of(&cold.call_graph));
    assert!(ArtifactStore::load(&store_dir(root)).entries["src/a.rs"]
        .call_sites_by_symbol
        .is_some());

    rewrite_store(root, |store| {
        store["version"] = json!(1);
        for entry in store["entries"].as_object_mut().unwrap().values_mut() {
            entry
                .as_object_mut()
                .unwrap()
                .remove("call_sites_by_symbol");
        }
    });
    std::fs::remove_file(derived_path(root)).unwrap();
    let (idx, r) = build_index_reported(root, None).unwrap();
    assert_eq!(r.artifact_misses, idx.total_files);
    assert_eq!(json_of(&idx.call_graph), json_of(&cold.call_graph));
}
