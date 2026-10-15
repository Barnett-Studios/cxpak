//! Per-file artifact store (ADR-0210).
//!
//! One JSON file in the flat cache dir (`.cxpak/cache/artifacts.json`), shared
//! by every workspace namespace and by the CLI. Each entry records the
//! SHA-256 of the bytes it was computed from; an entry is reused only when that
//! hash matches the bytes the current build just read (index path), or when the
//! CLI's (mtime_ns, size) fast path matches.

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
