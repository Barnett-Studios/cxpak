pub mod parse;

use crate::parser::language::ParseResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const CACHE_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct FileCache {
    pub version: u32,
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

impl FileCache {
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: Vec::new(),
        }
    }

    pub fn load(cache_dir: &Path) -> Self {
        let cache_file = cache_dir.join("cache.json");
        let content = match std::fs::read_to_string(&cache_file) {
            Ok(c) => c,
            Err(_) => return Self::new(),
        };
        match serde_json::from_str::<FileCache>(&content) {
            Ok(cache) if cache.version == CACHE_VERSION => cache,
            _ => Self::new(),
        }
    }

    pub fn save(&self, cache_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(cache_dir)?;
        let json = serde_json::to_string(self)?;
        std::fs::write(cache_dir.join("cache.json"), json)
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
