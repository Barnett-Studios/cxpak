pub mod parse;

use crate::parser::language::ParseResult;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;

pub const CACHE_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct FileCache {
    pub version: u32,
    /// SHA-like hash of all tree-sitter grammar versions at compile time.
    /// When grammars are updated, this hash changes and the cache is invalidated.
    #[serde(default)]
    pub grammar_hash: String,
    pub entries: Vec<CacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub relative_path: String,
    pub mtime: i64,
    pub size_bytes: u64,
    pub language: Option<String>,
    pub token_count: usize,
    pub parse_result: Option<ParseResult>,
}

/// Grammar hash computed at compile time by build.rs.
const CURRENT_GRAMMAR_HASH: &str = env!("CXPAK_GRAMMAR_HASH");

fn lock_cache(cache_dir: &Path) -> std::io::Result<std::fs::File> {
    let lock_path = cache_dir.join("cache.lock");
    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    lock_file.lock_exclusive()?;
    Ok(lock_file)
}

impl FileCache {
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            grammar_hash: CURRENT_GRAMMAR_HASH.to_string(),
            entries: Vec::new(),
        }
    }

    pub fn load(cache_dir: &Path) -> Self {
        let cache_file = cache_dir.join("cache.json");
        // Acquire lock before reading so concurrent processes don't read a
        // partially-written file. Failures to lock are non-fatal: fall back to
        // a fresh cache rather than blocking or crashing.
        let _lock = lock_cache(cache_dir).ok();
        let content = match std::fs::read_to_string(&cache_file) {
            Ok(c) => c,
            Err(_) => return Self::new(),
        };
        match serde_json::from_str::<FileCache>(&content) {
            Ok(cache)
                if cache.version == CACHE_VERSION && cache.grammar_hash == CURRENT_GRAMMAR_HASH =>
            {
                cache
            }
            _ => Self::new(),
        }
    }

    pub fn save(&self, cache_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(cache_dir)?;
        // Hold the lock for the entire write + rename sequence so concurrent
        // processes cannot observe an incomplete cache.json.
        let _lock = lock_cache(cache_dir)?;
        let json = serde_json::to_string(self)?;
        let tmp = cache_dir.join("cache.json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(tmp, cache_dir.join("cache.json"))
    }

    pub fn as_map(&self) -> HashMap<&str, &CacheEntry> {
        self.entries
            .iter()
            .map(|e| (e.relative_path.as_str(), e))
            .collect()
    }
}

impl Default for FileCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::{Export, Import, Symbol, SymbolKind, Visibility};

    fn make_entry(path: &str) -> CacheEntry {
        CacheEntry {
            relative_path: path.to_string(),
            mtime: 1_700_000_000,
            size_bytes: 1024,
            language: Some("rust".to_string()),
            token_count: 42,
            parse_result: None,
        }
    }

    fn make_parse_result() -> ParseResult {
        ParseResult {
            symbols: vec![Symbol {
                name: "my_fn".to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "fn my_fn()".to_string(),
                body: "fn my_fn() {}".to_string(),
                start_line: 1,
                end_line: 3,
            }],
            imports: vec![Import {
                source: "std::io".to_string(),
                names: vec!["Read".to_string()],
            }],
            exports: vec![Export {
                name: "my_fn".to_string(),
                kind: SymbolKind::Function,
            }],
        }
    }

    #[test]
    fn test_cache_roundtrip() {
        let mut cache = FileCache::new();
        cache.entries.push(make_entry("src/main.rs"));

        let json = serde_json::to_string(&cache).expect("serialize");
        let restored: FileCache = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.version, CACHE_VERSION);
        assert_eq!(restored.entries.len(), 1);
        let entry = &restored.entries[0];
        assert_eq!(entry.relative_path, "src/main.rs");
        assert_eq!(entry.mtime, 1_700_000_000);
        assert_eq!(entry.size_bytes, 1024);
        assert_eq!(entry.language.as_deref(), Some("rust"));
        assert_eq!(entry.token_count, 42);
        assert!(entry.parse_result.is_none());
    }

    #[test]
    fn test_cache_with_parse_result() {
        let mut cache = FileCache::new();
        let mut entry = make_entry("src/lib.rs");
        entry.parse_result = Some(make_parse_result());
        cache.entries.push(entry);

        let json = serde_json::to_string(&cache).expect("serialize");
        let restored: FileCache = serde_json::from_str(&json).expect("deserialize");

        let pr = restored.entries[0]
            .parse_result
            .as_ref()
            .expect("parse_result present");
        assert_eq!(pr.symbols.len(), 1);
        assert_eq!(pr.symbols[0].name, "my_fn");
        assert_eq!(pr.imports.len(), 1);
        assert_eq!(pr.imports[0].source, "std::io");
        assert_eq!(pr.exports.len(), 1);
        assert_eq!(pr.exports[0].name, "my_fn");
    }

    #[test]
    fn test_grammar_hash_mismatch_returns_empty() {
        // A cache with a different grammar_hash should be discarded.
        let stale = serde_json::json!({
            "version": CACHE_VERSION,
            "grammar_hash": "stale_grammar_hash_that_does_not_match",
            "entries": [
                {
                    "relative_path": "src/main.rs",
                    "mtime": 1_700_000_000_i64,
                    "size_bytes": 512,
                    "language": null,
                    "token_count": 10,
                    "parse_result": null
                }
            ]
        });
        let json = stale.to_string();

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("cache.json"), &json).expect("write");

        let cache = FileCache::load(dir.path());
        assert_eq!(cache.version, CACHE_VERSION);
        assert!(
            cache.entries.is_empty(),
            "stale grammar hash should invalidate cache"
        );
        assert_eq!(cache.grammar_hash, CURRENT_GRAMMAR_HASH);
    }

    #[test]
    fn test_cache_version_mismatch_returns_empty() {
        let stale = serde_json::json!({
            "version": 0,
            "entries": [
                {
                    "relative_path": "src/main.rs",
                    "mtime": 1_700_000_000_i64,
                    "size_bytes": 512,
                    "language": null,
                    "token_count": 10,
                    "parse_result": null
                }
            ]
        });
        let json = stale.to_string();

        // Write to a temp dir and load via FileCache::load so the version check runs.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("cache.json"), &json).expect("write");

        let cache = FileCache::load(dir.path());
        assert_eq!(cache.version, CACHE_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn test_save_and_load_cache() {
        let dir = tempfile::tempdir().expect("tempdir");

        let mut cache = FileCache::new();
        cache.entries.push(make_entry("src/lib.rs"));
        cache.save(dir.path()).expect("save");

        let loaded = FileCache::load(dir.path());
        assert_eq!(loaded.version, CACHE_VERSION);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].relative_path, "src/lib.rs");
        assert_eq!(loaded.entries[0].token_count, 42);
    }

    #[test]
    fn test_atomic_save_no_tmp_file_after_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cache = FileCache::new();
        cache.entries.push(make_entry("src/lib.rs"));
        cache.save(dir.path()).expect("save");

        // After a successful save, the .tmp file must not remain.
        let tmp_path = dir.path().join("cache.json.tmp");
        assert!(
            !tmp_path.exists(),
            "cache.json.tmp must not exist after successful save"
        );
        // The real file must exist and be loadable.
        let loaded = FileCache::load(dir.path());
        assert_eq!(loaded.entries.len(), 1);
    }

    #[test]
    fn test_load_missing_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nonexistent = dir.path().join("does_not_exist");

        let cache = FileCache::load(&nonexistent);
        assert_eq!(cache.version, CACHE_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn test_load_corrupt_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("cache.json"), "not json").expect("write");

        let cache = FileCache::load(dir.path());
        assert_eq!(cache.version, CACHE_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn test_default_impl() {
        let cache = FileCache::default();
        assert_eq!(cache.version, CACHE_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn test_concurrent_save_no_corruption() {
        // Two threads both call save() with distinct data. After both finish,
        // the resulting cache.json must deserialize cleanly (not be corrupted)
        // and must contain the data from exactly one of the two writers.
        let dir = tempfile::tempdir().expect("tempdir");
        let dir_path = dir.path().to_path_buf();

        let dir_a = dir_path.clone();
        let dir_b = dir_path.clone();

        let handle_a = std::thread::spawn(move || {
            let mut cache = FileCache::new();
            for i in 0..5 {
                let mut e = make_entry(&format!("src/file_a_{i}.rs"));
                e.token_count = 100 + i;
                cache.entries.push(e);
            }
            cache.save(&dir_a).expect("thread A save");
        });

        let handle_b = std::thread::spawn(move || {
            let mut cache = FileCache::new();
            for i in 0..5 {
                let mut e = make_entry(&format!("src/file_b_{i}.rs"));
                e.token_count = 200 + i;
                cache.entries.push(e);
            }
            cache.save(&dir_b).expect("thread B save");
        });

        handle_a.join().expect("thread A panicked");
        handle_b.join().expect("thread B panicked");

        // The file must load without error (no partial JSON).
        let loaded = FileCache::load(&dir_path);
        assert_eq!(loaded.version, CACHE_VERSION);
        // Must contain exactly 5 entries — data from one writer, not a mix.
        assert_eq!(
            loaded.entries.len(),
            5,
            "expected 5 entries from one writer, got: {:?}",
            loaded
                .entries
                .iter()
                .map(|e| &e.relative_path)
                .collect::<Vec<_>>()
        );
        // All entries must share the same writer prefix.
        let all_a = loaded
            .entries
            .iter()
            .all(|e| e.relative_path.contains("file_a"));
        let all_b = loaded
            .entries
            .iter()
            .all(|e| e.relative_path.contains("file_b"));
        assert!(
            all_a || all_b,
            "entries must come from exactly one writer, not a mix: {:?}",
            loaded
                .entries
                .iter()
                .map(|e| &e.relative_path)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_load_acquires_lock_before_read() {
        // Verify that load() does not panic or return corrupt data when the
        // lock file already exists (created by a previous save or lock_cache).
        let dir = tempfile::tempdir().expect("tempdir");
        // Pre-create the lock file to simulate a scenario where it was left
        // behind by a previous process.
        std::fs::write(dir.path().join("cache.lock"), b"").expect("create lock file");

        let mut cache = FileCache::new();
        cache.entries.push(make_entry("src/main.rs"));
        cache
            .save(dir.path())
            .expect("save with existing lock file");

        let loaded = FileCache::load(dir.path());
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].relative_path, "src/main.rs");
    }

    #[test]
    fn test_as_map() {
        let mut cache = FileCache::new();
        cache.entries.push(make_entry("src/main.rs"));
        cache.entries.push(make_entry("src/lib.rs"));

        let map = cache.as_map();
        assert_eq!(map.len(), 2);

        let main_entry = map.get("src/main.rs").expect("main.rs in map");
        assert_eq!(main_entry.token_count, 42);

        let lib_entry = map.get("src/lib.rs").expect("lib.rs in map");
        assert_eq!(lib_entry.size_bytes, 1024);

        assert!(!map.contains_key("src/unknown.rs"));
    }
}
