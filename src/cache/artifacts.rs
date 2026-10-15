//! Per-file artifact store (ADR-0210).
//!
//! One JSON file in the flat cache dir (`.cxpak/cache/artifacts.json`), shared
//! by every workspace namespace and by the CLI. Each entry records the
//! SHA-256 of the bytes it was computed from; an entry is reused only when that
//! hash matches the bytes the current build just read (index path), or when the
//! CLI's (mtime_ns, size) fast path matches.

use crate::cache::{lock_cache, CURRENT_GRAMMAR_HASH};
use crate::parser::language::ParseResult;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// File name of the store inside the flat cache dir.
pub(crate) const ARTIFACT_STORE_FILE: &str = "artifacts.json";
/// Bumped when the on-disk shape of [`ArtifactStore`] changes.
pub(crate) const ARTIFACT_STORE_VERSION: u32 = 1;

/// Everything computed from one file's bytes. A `Some` field was computed from
/// the bytes `content_sha256` names; a writer that did not compute a field
/// stores `None` (C6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ArtifactEntry {
    pub(crate) content_sha256: String,
    /// CLI fast-path key only; the index path never trusts it.
    pub(crate) mtime_ns: u64,
    pub(crate) size_bytes: u64,
    pub(crate) language: Option<String>,
    pub(crate) parse_result: Option<ParseResult>,
    pub(crate) token_count: Option<usize>,
    pub(crate) term_frequencies: Option<BTreeMap<String, u32>>,
}

/// Content-validated per-file artifacts, keyed by repo-relative path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ArtifactStore {
    pub(crate) version: u32,
    pub(crate) grammar_hash: String,
    pub(crate) entries: BTreeMap<String, ArtifactEntry>,
}

impl ArtifactStore {
    pub(crate) fn new() -> Self {
        Self {
            version: ARTIFACT_STORE_VERSION,
            grammar_hash: CURRENT_GRAMMAR_HASH.to_string(),
            entries: BTreeMap::new(),
        }
    }

    /// Read the store without taking the lock. Fail-closed: a missing,
    /// truncated, corrupt, version- or grammar-mismatched file is an empty store.
    fn read(cache_root: &Path) -> Self {
        std::fs::read_to_string(cache_root.join(ARTIFACT_STORE_FILE))
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .filter(|s| {
                s.version == ARTIFACT_STORE_VERSION && s.grammar_hash == CURRENT_GRAMMAR_HASH
            })
            .unwrap_or_else(Self::new)
    }

    /// Write the store atomically. The caller holds the cache lock.
    fn write(&self, cache_root: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string(self)?;
        let tmp = cache_root.join(format!("{ARTIFACT_STORE_FILE}.tmp"));
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, cache_root.join(ARTIFACT_STORE_FILE))
    }

    pub(crate) fn load(cache_root: &Path) -> Self {
        let _lock = lock_cache(cache_root).ok();
        Self::read(cache_root)
    }

    #[cfg(test)]
    pub(crate) fn save(&self, cache_root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(cache_root)?;
        let _lock = lock_cache(cache_root)?;
        self.write(cache_root)
    }

    /// Load, mutate and write back under one lock acquisition, so concurrent
    /// writers cannot drop each other's entries between the read and the write.
    pub(crate) fn update(cache_root: &Path, mutate: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        std::fs::create_dir_all(cache_root)?;
        let _lock = lock_cache(cache_root)?;
        let mut store = Self::read(cache_root);
        mutate(&mut store);
        store.write(cache_root)
    }
}

/// Merge one build's fresh entries into the persisted ones. Returns whether
/// anything changed (a prune, an insert, or a replaced entry).
///
/// `scanned` is every path the build saw and read successfully. In-scope
/// persisted paths (all of them for a full scan, those under the prefix for a
/// workspace scan) that are not in `scanned` are pruned; out-of-scope entries
/// are kept. `changed` entries replace stale ones for the same path.
pub(crate) fn merge_store_entries(
    existing: &mut BTreeMap<String, ArtifactEntry>,
    changed: BTreeMap<String, ArtifactEntry>,
    scanned: &BTreeSet<String>,
    workspace: Option<&str>,
) -> bool {
    let before = existing.len();
    existing.retain(|path, _| {
        let in_scope =
            workspace.is_none_or(|ws| crate::scanner::path_starts_with_workspace(path, ws));
        !in_scope || scanned.contains(path)
    });
    let mut dirty = existing.len() != before;
    for (path, entry) in changed {
        if existing.get(&path) != Some(&entry) {
            existing.insert(path, entry);
            dirty = true;
        }
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::ParseResult;
    use std::collections::{BTreeMap, BTreeSet};

    fn entry(sha: &str) -> ArtifactEntry {
        ArtifactEntry {
            content_sha256: sha.to_string(),
            mtime_ns: 1,
            size_bytes: 2,
            language: Some("rust".to_string()),
            parse_result: Some(ParseResult {
                symbols: vec![],
                imports: vec![],
                exports: vec![],
            }),
            token_count: Some(3),
            term_frequencies: Some(BTreeMap::from([("helper".to_string(), 2u32)])),
        }
    }

    #[test]
    fn artifact_store_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = ArtifactStore::new();
        store.entries.insert("src/a.rs".to_string(), entry("aa"));
        store.save(dir.path()).unwrap();
        let loaded = ArtifactStore::load(dir.path());
        assert_eq!(loaded.entries, store.entries);
        assert_eq!(loaded.version, ARTIFACT_STORE_VERSION);
    }

    #[test]
    fn artifact_store_rejects_version_and_grammar_mismatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = ArtifactStore::new();
        store.entries.insert("src/a.rs".to_string(), entry("aa"));
        store.version = ARTIFACT_STORE_VERSION + 1;
        store.save(dir.path()).unwrap();
        assert!(ArtifactStore::load(dir.path()).entries.is_empty());

        let mut store = ArtifactStore::new();
        store.entries.insert("src/a.rs".to_string(), entry("aa"));
        store.grammar_hash = "not-this-build".to_string();
        store.save(dir.path()).unwrap();
        assert!(ArtifactStore::load(dir.path()).entries.is_empty());
    }

    #[test]
    fn artifact_store_truncated_file_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = ArtifactStore::new();
        store.entries.insert("src/a.rs".to_string(), entry("aa"));
        store.save(dir.path()).unwrap();
        let path = dir.path().join(ARTIFACT_STORE_FILE);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(ArtifactStore::load(dir.path()).entries.is_empty());
    }

    #[test]
    fn artifact_store_missing_dir_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(ArtifactStore::load(&dir.path().join("absent"))
            .entries
            .is_empty());
    }

    #[test]
    fn artifact_store_update_merges_under_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut first = ArtifactStore::new();
        first.entries.insert("keep.rs".to_string(), entry("k"));
        first.save(dir.path()).unwrap();
        ArtifactStore::update(dir.path(), |store| {
            store.entries.insert("new.rs".to_string(), entry("n"));
        })
        .unwrap();
        let loaded = ArtifactStore::load(dir.path());
        assert_eq!(
            loaded.entries.keys().cloned().collect::<Vec<_>>(),
            vec!["keep.rs".to_string(), "new.rs".to_string()]
        );
    }

    fn scanned(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn merge_full_scan_prunes() {
        let mut existing = BTreeMap::from([
            ("src/a.rs".to_string(), entry("a")),
            ("src/gone.rs".to_string(), entry("g")),
        ]);
        let changed = merge_store_entries(
            &mut existing,
            BTreeMap::new(),
            &scanned(&["src/a.rs"]),
            None,
        );
        assert!(changed, "a prune is a change");
        assert_eq!(existing.keys().collect::<Vec<_>>(), vec!["src/a.rs"]);
    }

    #[test]
    fn merge_workspace_keeps_foreign() {
        let mut existing = BTreeMap::from([
            ("src/a.rs".to_string(), entry("a")),
            ("src/gone.rs".to_string(), entry("g")),
            ("web/app.ts".to_string(), entry("w")),
        ]);
        let changed = merge_store_entries(
            &mut existing,
            BTreeMap::new(),
            &scanned(&["src/a.rs"]),
            Some("src"),
        );
        assert!(changed);
        assert_eq!(
            existing.keys().collect::<Vec<_>>(),
            vec!["src/a.rs", "web/app.ts"],
            "entries outside the workspace prefix survive a workspace-scoped build"
        );
    }

    #[test]
    fn merge_fresh_replaces_stale() {
        let mut existing = BTreeMap::from([("src/a.rs".to_string(), entry("old"))]);
        let changed = merge_store_entries(
            &mut existing,
            BTreeMap::from([("src/a.rs".to_string(), entry("new"))]),
            &scanned(&["src/a.rs"]),
            None,
        );
        assert!(changed);
        assert_eq!(existing["src/a.rs"].content_sha256, "new");
    }

    #[test]
    fn merge_without_changes_reports_unchanged() {
        let mut existing = BTreeMap::from([("src/a.rs".to_string(), entry("a"))]);
        let changed = merge_store_entries(
            &mut existing,
            BTreeMap::new(),
            &scanned(&["src/a.rs"]),
            None,
        );
        assert!(!changed);
        assert_eq!(existing.len(), 1);
    }

    #[test]
    fn merge_output_sorted() {
        let mut existing = BTreeMap::new();
        merge_store_entries(
            &mut existing,
            BTreeMap::from([
                ("z.rs".to_string(), entry("z")),
                ("a.rs".to_string(), entry("a")),
            ]),
            &scanned(&["z.rs", "a.rs"]),
            None,
        );
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = ArtifactStore::new();
        store.entries = existing;
        store.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(ARTIFACT_STORE_FILE)).unwrap();
        assert!(text.find("\"a.rs\"").unwrap() < text.find("\"z.rs\"").unwrap());
    }
}
