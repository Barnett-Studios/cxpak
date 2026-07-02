pub mod parse;

use crate::parser::language::ParseResult;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;

/// Cache schema version.  Bumped 4→5 in Task 0.4 (cxpak 3.0.0 Phase 0)
/// because `TypedEdge` gained a `confidence: EdgeConfidence` field that is
/// serialized into `DerivedCache.graph`.  A stale v4 derived.json would
/// deserialize successfully (the field has `serde(default)`) but all edges
/// would carry `Extracted` regardless of their true confidence.  Bumping the
/// version forces a clean rebuild so every edge gets the correct value from
/// `EdgeType::default_confidence()`.
///
/// Bumped 5→6 in Task B3d (ADR-0179) because `DerivedCache` gained a
/// `base_commit: Option<String>` — the git HEAD SHA the cached derived
/// analysis was built at.  A stale v5 derived.json would deserialize
/// successfully (the field has `serde(default)` → `None`) but the post-commit
/// edge-delta path treats a `None` base as "unverified" and falls back to a
/// full rebuild anyway; bumping forces a clean rebuild once so every cache is
/// re-stamped with a real base_commit (fail-closed).
pub const CACHE_VERSION: u32 = 6;

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
    /// Mtime in nanoseconds since UNIX epoch.  Added in CACHE_VERSION 4.
    /// `None` for entries written by older code (treated as a stat-index miss).
    #[serde(default)]
    pub mtime_ns: Option<u64>,
    /// SHA-256 of the file's content (after markdown frontmatter stripping).
    /// Added in CACHE_VERSION 4; `None` means the hash has not been computed yet.
    #[serde(default)]
    pub content_sha256: Option<String>,
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

// ---------------------------------------------------------------------------
// Stat-index: per-file (mtime_ns, size_bytes) → content_sha256 map that lets
// `content_fingerprint` skip re-hashing files whose stat is unchanged.
//
// Design (ADR-0167 constraint): the fingerprint VALUE is content-based and
// byte-identical across machines.  The stat-index is a *local computation
// accelerator only*: a fresh clone with no stat-index recomputes all SHA-256 →
// identical fingerprint.  A file whose (mtime_ns, size_bytes) changed is
// always re-read + re-hashed + stat-index updated.  Fail-closed: any load
// error → empty index → full re-hash.
// ---------------------------------------------------------------------------

/// A single entry in the stat-index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatIndexEntry {
    /// Mtime of the file in nanoseconds since UNIX epoch.
    pub mtime_ns: u64,
    /// File size in bytes.
    pub size_bytes: u64,
    /// SHA-256 of the file content (markdown frontmatter stripped before hashing).
    pub content_sha256: String,
}

/// Persisted map of `relative_path → StatIndexEntry`.
///
/// Stored as `.cxpak/cache/<namespace>/stat_index.json`.  Invalidated on
/// `CACHE_VERSION` mismatch (same lock/atomic-save infra as `FileCache`).
#[derive(Debug, Serialize, Deserialize)]
pub struct StatIndex {
    pub version: u32,
    pub entries: HashMap<String, StatIndexEntry>,
}

impl Default for StatIndex {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }
}

/// Return the mtime of `path` as nanoseconds since UNIX epoch, or 0 on failure.
/// Used to populate the stat-index key.
pub fn file_mtime_ns(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

impl StatIndex {
    /// Load the stat-index from `<cache_dir>/stat_index.json`.
    /// Returns an empty index on any error or version mismatch (fail-closed).
    pub fn load(cache_dir: &Path) -> Self {
        let path = cache_dir.join("stat_index.json");
        let _lock = lock_cache(cache_dir).ok();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };
        match serde_json::from_str::<StatIndex>(&content) {
            Ok(idx) if idx.version == CACHE_VERSION => idx,
            _ => Self::default(),
        }
    }

    /// Atomically persist the stat-index (lock + temp-write + rename).
    pub fn save(&self, cache_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(cache_dir)?;
        let _lock = lock_cache(cache_dir)?;
        let json = serde_json::to_string(self)?;
        let tmp = cache_dir.join("stat_index.json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(tmp, cache_dir.join("stat_index.json"))
    }
}

// ---------------------------------------------------------------------------
// Markdown frontmatter stripping
// ---------------------------------------------------------------------------

/// Strip a leading YAML frontmatter block (`---\n...\n---\n`) from `content`
/// before hashing, so frontmatter-only edits (e.g. updated dates) do not churn
/// the derived cache.  Non-markdown content without a frontmatter delimiter is
/// returned as-is.
///
/// Only a `---` on the very first line triggers stripping.  If there is no
/// closing `---\n`, the full content is returned unchanged (conservative: a
/// malformed frontmatter is treated as body content).
pub fn strip_md_frontmatter(content: &str) -> &str {
    // Must start with exactly `---\n`.
    let after_open = match content.strip_prefix("---\n") {
        Some(rest) => rest,
        None => return content,
    };
    // Scan line by line to find the closing delimiter.  We advance one line at
    // a time so body content — including horizontal rules (`---`) — is never
    // mistaken for the frontmatter close.
    let mut rest = after_open;
    let mut consumed = 4_usize; // length of opening "---\n"
    loop {
        if rest.starts_with("---\n") {
            // Closing `---` followed by a newline: body starts after it.
            let body_start = consumed + 4;
            return &content[body_start..];
        }
        if rest == "---" {
            // Closing `---` at EOF with no trailing newline: no body.
            return "";
        }
        match rest.find('\n') {
            Some(nl) => {
                consumed += nl + 1;
                rest = &rest[nl + 1..];
            }
            None => {
                // No closing `---` found; return original content unchanged.
                return content;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Derived-index cache (ADR-0167): persist the expensive derived analysis
// (dependency graph, call graph, PageRank, conventions, co-changes) keyed by a
// **content** fingerprint so the cache survives `git checkout` / clone / CI and
// is portable across machines (mtimes are meaningless across clones; content
// hashes are not). Fail-closed: any mismatch / corruption → full rebuild.
// ---------------------------------------------------------------------------

/// SHA-256 content fingerprint over `(relative_path, sha256(content))` pairs
/// (sorted for determinism) plus the git HEAD oid. Two checkouts with identical
/// file contents and HEAD produce the same fingerprint on any machine; a
/// same-size content edit changes it (unlike `(mtime, size)`), and a HEAD move
/// changes it (so history-derived data — conventions, co-changes — is rebuilt).
///
/// Markdown frontmatter is stripped before hashing so frontmatter-only edits
/// (e.g. updated `date:` fields) do not churn the derived cache.
///
/// For builds where mtime_ns and size_bytes are available, prefer
/// [`content_fingerprint_with_stat_index`] to skip re-hashing unchanged files.
pub fn content_fingerprint(files: &[(String, String)], head_oid: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut pairs: Vec<(&str, String)> = files
        .iter()
        .map(|(path, content)| {
            let body = strip_md_frontmatter(content);
            (
                path.as_str(),
                format!("{:x}", Sha256::digest(body.as_bytes())),
            )
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    for (path, content_hash) in &pairs {
        hasher.update(path.as_bytes());
        hasher.update(b":");
        hasher.update(content_hash.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(b"HEAD:");
    hasher.update(head_oid.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Stat-index-accelerated variant of [`content_fingerprint`].
///
/// For each file `(path, content, mtime_ns, size_bytes)`:
/// - If `stat_index` has a matching `(mtime_ns, size_bytes)` entry, the stored
///   `content_sha256` is reused — `hash_fn` is **not** called.
/// - Otherwise `hash_fn(content)` is called (must return a hex SHA-256 of the
///   content after frontmatter stripping, matching what `content_fingerprint`
///   produces), and the stat-index entry is updated in place.
///
/// The fingerprint value is byte-identical to `content_fingerprint` for the
/// same file contents: the stat-index is a local accelerator only, with no
/// effect on portability or determinism (ADR-0167).
///
/// `hash_fn` receives the **full raw content** of the file; it is responsible
/// for calling `strip_md_frontmatter` internally (the production hasher does
/// this; the test counting hasher also delegates to the real SHA-256).
///
/// The `stat_index` is mutated in place to record updated entries so the
/// caller can persist it after the call.
pub fn content_fingerprint_with_stat_index<F>(
    files: &[(String, String, u64, u64)],
    head_oid: &str,
    stat_index: &mut StatIndex,
    hash_fn: F,
) -> String
where
    F: Fn(&str) -> String,
{
    use sha2::{Digest, Sha256};
    let mut pairs: Vec<(&str, String)> = files
        .iter()
        .map(|(path, content, mtime_ns, size_bytes)| {
            let content_hash = if let Some(entry) = stat_index.entries.get(path.as_str()) {
                if entry.mtime_ns == *mtime_ns && entry.size_bytes == *size_bytes {
                    // Stat-index hit: reuse stored hash, skip re-hashing.
                    entry.content_sha256.clone()
                } else {
                    // Stat changed: re-hash and update the index.
                    let h = hash_fn(content);
                    stat_index.entries.insert(
                        path.clone(),
                        StatIndexEntry {
                            mtime_ns: *mtime_ns,
                            size_bytes: *size_bytes,
                            content_sha256: h.clone(),
                        },
                    );
                    h
                }
            } else {
                // New file: hash and record.
                let h = hash_fn(content);
                stat_index.entries.insert(
                    path.clone(),
                    StatIndexEntry {
                        mtime_ns: *mtime_ns,
                        size_bytes: *size_bytes,
                        content_sha256: h.clone(),
                    },
                );
                h
            };
            (path.as_str(), content_hash)
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    for (path, content_hash) in &pairs {
        hasher.update(path.as_bytes());
        hasher.update(b":");
        hasher.update(content_hash.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(b"HEAD:");
    hasher.update(head_oid.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Persisted derived analysis. Validated on load by `version` + `grammar_hash`
/// + `fingerprint`; any mismatch is treated as a miss (fail-closed).
#[derive(Debug, Serialize, Deserialize)]
pub struct DerivedCache {
    pub version: u32,
    pub grammar_hash: String,
    pub fingerprint: String,
    pub graph: crate::index::graph::DependencyGraph,
    pub call_graph: crate::intelligence::call_graph::CallGraph,
    pub pagerank: HashMap<String, f64>,
    pub conventions: crate::conventions::ConventionProfile,
    pub co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,
    /// Git HEAD SHA (full oid, hex) the cached derived analysis was built at,
    /// or `None` when the build happened outside a git repo / on an unborn HEAD.
    ///
    /// This is the anchor for the SAFE post-commit edge-delta (ADR-0179): the
    /// delta may be applied onto this cache's `graph` **only** if
    /// `base_commit == parent(HEAD)` at post-commit time — i.e. the cache
    /// reflects exactly the tree state before the new commit. Every other case
    /// (absent/`None`, mismatched, >1 commit behind, or any load failure) falls
    /// back to a full rebuild. `#[serde(default)]` so a pre-v6 cache
    /// deserializes to `None` (which the delta path treats as unverified →
    /// full rebuild); the `CACHE_VERSION` bump invalidates such caches anyway.
    #[serde(default)]
    pub base_commit: Option<String>,
}

impl DerivedCache {
    /// Construct a derived cache stamped with the current `CACHE_VERSION` and
    /// grammar hash, ready to [`save`](Self::save).
    ///
    /// `base_commit` is the git HEAD SHA the derived analysis was built at
    /// (`None` outside a git repo / unborn HEAD). Stamping it at every write
    /// site — including an interleaved `overview`/`serve` build between commits —
    /// keeps the recorded base current, so the next post-commit's edge-delta can
    /// still validate against it (see [`DerivedCache::base_commit`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fingerprint: String,
        graph: crate::index::graph::DependencyGraph,
        call_graph: crate::intelligence::call_graph::CallGraph,
        pagerank: HashMap<String, f64>,
        conventions: crate::conventions::ConventionProfile,
        co_changes: Vec<crate::intelligence::co_change::CoChangeEdge>,
        base_commit: Option<String>,
    ) -> Self {
        Self {
            version: CACHE_VERSION,
            grammar_hash: CURRENT_GRAMMAR_HASH.to_string(),
            fingerprint,
            graph,
            call_graph,
            pagerank,
            conventions,
            co_changes,
            base_commit,
        }
    }

    /// Load the derived cache and return it only if it is structurally valid AND
    /// matches the current grammar hash and the supplied content `fingerprint`.
    /// Returns `None` on any error, version/grammar/fingerprint mismatch, or
    /// corruption — the caller must then rebuild from scratch (fail-closed).
    pub fn load(cache_dir: &Path, fingerprint: &str) -> Option<Self> {
        let path = cache_dir.join("derived.json");
        let _lock = lock_cache(cache_dir).ok();
        let content = std::fs::read_to_string(&path).ok()?;
        let cache: DerivedCache = serde_json::from_str(&content).ok()?;
        if cache.version == CACHE_VERSION
            && cache.grammar_hash == CURRENT_GRAMMAR_HASH
            && cache.fingerprint == fingerprint
        {
            Some(cache)
        } else {
            None
        }
    }

    /// Load the derived cache for use as an edge-delta BASE, validating only the
    /// structural compatibility gates (`version` + `grammar_hash`) and **not**
    /// the content `fingerprint`.
    ///
    /// The post-commit rebuild (ADR-0179) needs the graph as it stood *before*
    /// the new commit, whose content fingerprint necessarily differs from the
    /// post-commit tree — so the fingerprint gate must be skipped here. Safety
    /// is instead enforced by the caller: apply the delta only if
    /// `base_commit == parent(HEAD)`. Grammar/version are still validated so a
    /// stale-schema graph is never deltaed onto (fail-closed → full rebuild).
    /// Returns `None` on any error or version/grammar mismatch.
    pub fn load_for_delta(cache_dir: &Path) -> Option<Self> {
        let path = cache_dir.join("derived.json");
        let _lock = lock_cache(cache_dir).ok();
        let content = std::fs::read_to_string(&path).ok()?;
        let cache: DerivedCache = serde_json::from_str(&content).ok()?;
        if cache.version == CACHE_VERSION && cache.grammar_hash == CURRENT_GRAMMAR_HASH {
            Some(cache)
        } else {
            None
        }
    }

    /// Atomically persist the derived cache (lock + temp-write + rename), mirroring
    /// [`FileCache::save`]. Errors are returned but are non-fatal to the caller.
    pub fn save(&self, cache_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(cache_dir)?;
        let _lock = lock_cache(cache_dir)?;
        let json = serde_json::to_string(self)?;
        let tmp = cache_dir.join("derived.json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(tmp, cache_dir.join("derived.json"))
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
            mtime_ns: None,
            content_sha256: None,
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

    // ── DerivedCache (ADR-0167) ────────────────────────────────────────────

    fn sample_derived(fingerprint: &str) -> DerivedCache {
        let mut graph = crate::index::graph::DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", crate::index::graph::EdgeType::Import);
        let mut pagerank = HashMap::new();
        pagerank.insert("a.rs".to_string(), 0.5);
        DerivedCache::new(
            fingerprint.to_string(),
            graph,
            crate::intelligence::call_graph::CallGraph::default(),
            pagerank,
            crate::conventions::ConventionProfile::default(),
            Vec::new(),
            Some("base-commit-sha".to_string()),
        )
    }

    #[test]
    fn derived_cache_roundtrip_hit_on_matching_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        sample_derived("fp-abc").save(dir.path()).unwrap();
        let loaded = DerivedCache::load(dir.path(), "fp-abc").expect("hit on matching fingerprint");
        assert_eq!(loaded.fingerprint, "fp-abc");
        assert!(loaded
            .graph
            .dependencies("a.rs")
            .map(|s| s.iter().any(|e| e.target == "b.rs"))
            .unwrap_or(false));
        assert_eq!(loaded.pagerank.get("a.rs"), Some(&0.5));
    }

    #[test]
    fn derived_cache_fingerprint_mismatch_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        sample_derived("fp-abc").save(dir.path()).unwrap();
        // A different content fingerprint must be a miss, never a stale hit.
        assert!(DerivedCache::load(dir.path(), "fp-different").is_none());
    }

    #[test]
    fn derived_cache_corrupt_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("derived.json"), "{ not valid json").unwrap();
        assert!(DerivedCache::load(dir.path(), "fp-abc").is_none());
    }

    #[test]
    fn derived_cache_missing_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DerivedCache::load(dir.path(), "fp-abc").is_none());
    }

    #[test]
    fn content_fingerprint_is_content_based_and_order_independent() {
        let a = vec![
            ("src/a.rs".to_string(), "fn a() {}".to_string()),
            ("src/b.rs".to_string(), "fn b() {}".to_string()),
        ];
        // Same contents, different input ordering → identical fingerprint (sorted).
        let b = vec![
            ("src/b.rs".to_string(), "fn b() {}".to_string()),
            ("src/a.rs".to_string(), "fn a() {}".to_string()),
        ];
        assert_eq!(
            content_fingerprint(&a, "head1"),
            content_fingerprint(&b, "head1")
        );
        // A content change flips the fingerprint (unlike a (mtime,size) key).
        let changed = vec![
            ("src/a.rs".to_string(), "fn a() { 1 }".to_string()),
            ("src/b.rs".to_string(), "fn b() {}".to_string()),
        ];
        assert_ne!(
            content_fingerprint(&a, "head1"),
            content_fingerprint(&changed, "head1")
        );
        // A HEAD move flips it (history-derived data must be rebuilt).
        assert_ne!(
            content_fingerprint(&a, "head1"),
            content_fingerprint(&a, "head2")
        );
    }

    #[test]
    fn derived_cache_grammar_or_version_mismatch_fails_closed() {
        // A derived.json with a stale grammar_hash must be rejected.
        let dir = tempfile::tempdir().unwrap();
        let stale = serde_json::json!({
            "version": CACHE_VERSION,
            "grammar_hash": "stale-grammar",
            "fingerprint": "fp-abc",
            "graph": { "edges": {}, "reverse_edges": {} },
            "call_graph": crate::intelligence::call_graph::CallGraph::default(),
            "pagerank": {},
            "conventions": crate::conventions::ConventionProfile::default(),
            "co_changes": [],
        });
        std::fs::write(dir.path().join("derived.json"), stale.to_string()).unwrap();
        assert!(DerivedCache::load(dir.path(), "fp-abc").is_none());
    }

    #[test]
    fn derived_cache_roundtrips_base_commit() {
        let dir = tempfile::tempdir().unwrap();
        sample_derived("fp-abc").save(dir.path()).unwrap();
        let loaded = DerivedCache::load(dir.path(), "fp-abc").expect("hit");
        assert_eq!(loaded.base_commit.as_deref(), Some("base-commit-sha"));
    }

    #[test]
    fn load_for_delta_ignores_fingerprint_but_keeps_grammar_version() {
        let dir = tempfile::tempdir().unwrap();
        sample_derived("fp-abc").save(dir.path()).unwrap();
        // Fingerprint-gated load with a DIFFERENT fingerprint misses...
        assert!(DerivedCache::load(dir.path(), "fp-does-not-match").is_none());
        // ...but load_for_delta returns it regardless of fingerprint, carrying
        // the base_commit the delta path validates against.
        let delta =
            DerivedCache::load_for_delta(dir.path()).expect("delta load ignores fingerprint");
        assert_eq!(delta.base_commit.as_deref(), Some("base-commit-sha"));
    }

    #[test]
    fn load_for_delta_fails_closed_on_grammar_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let stale = serde_json::json!({
            "version": CACHE_VERSION,
            "grammar_hash": "stale-grammar",
            "fingerprint": "fp-abc",
            "graph": { "edges": {}, "reverse_edges": {} },
            "call_graph": crate::intelligence::call_graph::CallGraph::default(),
            "pagerank": {},
            "conventions": crate::conventions::ConventionProfile::default(),
            "co_changes": [],
            "base_commit": "sha",
        });
        std::fs::write(dir.path().join("derived.json"), stale.to_string()).unwrap();
        assert!(DerivedCache::load_for_delta(dir.path()).is_none());
    }

    #[test]
    fn load_for_delta_missing_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DerivedCache::load_for_delta(dir.path()).is_none());
    }

    #[test]
    fn derived_cache_pre_v6_base_commit_defaults_to_none() {
        // A cache JSON written before base_commit existed must deserialize
        // (serde default → None) rather than error.
        let json = serde_json::json!({
            "version": CACHE_VERSION,
            "grammar_hash": CURRENT_GRAMMAR_HASH,
            "fingerprint": "fp-abc",
            "graph": { "edges": {}, "reverse_edges": {} },
            "call_graph": crate::intelligence::call_graph::CallGraph::default(),
            "pagerank": {},
            "conventions": crate::conventions::ConventionProfile::default(),
            "co_changes": [],
        });
        let cache: DerivedCache = serde_json::from_str(&json.to_string()).unwrap();
        assert!(cache.base_commit.is_none());
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
