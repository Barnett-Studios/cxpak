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
    let repo = git2::Repository::open(root).expect("open repo");
    let mut index = repo.index().expect("repo index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("git add");
    index.write().expect("index write");
    let tree = repo
        .find_tree(index.write_tree().expect("write tree"))
        .expect("find tree");
    let sig = git2::Signature::new("t", "t@t", &git2::Time::new(FIXED_TIME, 0)).expect("sig");
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .expect("commit");
}

/// Two Rust files with a call between them, an HTTP bridge (TS → FastAPI) so
/// a cross-language edge exists, and a markdown file with frontmatter.
fn warm_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
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
    git2::Repository::init(root).expect("git init");
    commit_all(root, "init");
    dir
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
