use crate::budget::counter::TokenCounter;
use crate::cache::artifacts::{merge_store_entries, ArtifactEntry, ArtifactStore};
use crate::index::parse_source;
use crate::parser::language::ParseResult;
use crate::parser::LanguageRegistry;
use crate::scanner::ScannedFile;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// Files larger than this are skipped by the parser to avoid runaway memory
/// usage (e.g. large generated files that slipped through the blocklist).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB

/// Parse all `files` using tree-sitter, with a persistent disk cache stored
/// under `<repo_root>/.cxpak/cache/` (the artifact store, ADR-0210).
///
/// For each file the function:
/// 1. Checks whether the store holds an entry whose (mtime_ns, size) match.
/// 2. On a hit, reuses the stored `ParseResult` without reading the file.
/// 3. On a miss, reads the file, hashes its bytes, parses it with tree-sitter
///    and records an entry that leaves `token_count` and `term_frequencies`
///    unset (this path computes neither).
/// 4. Merges the fresh entries into the store and saves it.
///
/// Returns a tuple of:
/// - A `HashMap` mapping `relative_path → ParseResult` for every file that could be parsed.
/// - A `HashMap` mapping `relative_path → file contents` for files read from disk on a
///   cache miss (so callers can avoid re-reading files that were already read here).
pub fn parse_with_cache(
    files: &[ScannedFile],
    repo_root: &Path,
    _counter: &TokenCounter,
    verbose: bool,
) -> (HashMap<String, ParseResult>, HashMap<String, String>) {
    if verbose {
        eprintln!("cxpak: parsing with tree-sitter");
    }

    let cache_dir = repo_root.join(".cxpak").join("cache");
    let store = ArtifactStore::load(&cache_dir);
    let registry = LanguageRegistry::new();

    // Each iteration is independent: it reads the store (shared, read-only),
    // creates its own tree-sitter Parser and reads source from disk.
    let outcomes: Vec<Option<Outcome>> = files
        .par_iter()
        .map(|file| {
            let mtime_ns = crate::cache::file_mtime_ns(&file.absolute_path);
            let size_bytes = file.size_bytes;

            if let Some(entry) = store.entries.get(&file.relative_path) {
                if mtime_ns != 0
                    && entry.mtime_ns == mtime_ns
                    && entry.size_bytes == size_bytes
                    && entry.language == file.language
                {
                    return Some(Outcome {
                        parse_result: entry.parse_result.clone(),
                        fresh: None,
                    });
                }
            }

            // Skip files that exceed the size cap to avoid runaway memory usage.
            if size_bytes > MAX_FILE_BYTES {
                eprintln!(
                    "cxpak: skipping parse of {} ({} bytes > {} byte limit)",
                    file.absolute_path.display(),
                    size_bytes,
                    MAX_FILE_BYTES
                );
                return None;
            }

            let source = match std::fs::read_to_string(&file.absolute_path) {
                Ok(source) => source,
                Err(e) => {
                    eprintln!(
                        "cxpak: failed to read {}: {}",
                        file.absolute_path.display(),
                        e
                    );
                    return None;
                }
            };
            let parse_result = parse_source(&source, file.language.as_deref(), &registry);
            let entry = ArtifactEntry {
                content_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
                mtime_ns,
                size_bytes,
                language: file.language.clone(),
                parse_result: parse_result.clone(),
                token_count: None,
                term_frequencies: None,
            };
            Some(Outcome {
                parse_result,
                fresh: Some((source, entry)),
            })
        })
        .collect();

    let mut parse_results: HashMap<String, ParseResult> = HashMap::new();
    let mut content_map: HashMap<String, String> = HashMap::new();
    let mut changed: BTreeMap<String, ArtifactEntry> = BTreeMap::new();
    let mut scanned: BTreeSet<String> = BTreeSet::new();
    for (file, outcome) in files.iter().zip(outcomes) {
        let Some(outcome) = outcome else { continue };
        scanned.insert(file.relative_path.clone());
        if let Some(pr) = outcome.parse_result {
            parse_results.insert(file.relative_path.clone(), pr);
        }
        if let Some((source, entry)) = outcome.fresh {
            content_map.insert(file.relative_path.clone(), source);
            changed.insert(file.relative_path.clone(), entry);
        }
    }

    if verbose {
        eprintln!("cxpak: parsed {} files", parse_results.len());
    }

    let prune = store.entries.keys().any(|path| !scanned.contains(path));
    if !changed.is_empty() || prune {
        let saved = ArtifactStore::update(&cache_dir, |persisted| {
            merge_store_entries(&mut persisted.entries, changed, &scanned, None);
        });
        if let (Err(e), true) = (saved, verbose) {
            eprintln!("cxpak: warning: failed to save cache: {e}");
        }
    }

    (parse_results, content_map)
}

/// One file's result: its parse result, plus the content and store entry when
/// it was read (a miss).
struct Outcome {
    parse_result: Option<ParseResult>,
    fresh: Option<(String, ArtifactEntry)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use std::fs;

    /// Create a minimal git repo with one Rust source file and return its root.
    ///
    /// Uses `git2` to avoid spawning a `git` subprocess; subprocess-based init
    /// is prone to resource contention under high test parallelism on macOS.
    fn make_test_repo(tmp: &tempfile::TempDir, source: &str) -> std::path::PathBuf {
        let root = tmp.path().to_path_buf();
        // Initialise a git repo so Scanner accepts the directory.
        let repo = git2::Repository::init(&root).expect("git2 init");
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        let file = src_dir.join("lib.rs");
        fs::write(&file, source).unwrap();
        // Stage the file so it is git-tracked.
        let mut index = repo.index().expect("repo index");
        index
            .add_path(std::path::Path::new("src/lib.rs"))
            .expect("git add src/lib.rs");
        index.write().expect("index write");
        root
    }

    fn scan_files(root: &Path) -> Vec<ScannedFile> {
        crate::scanner::Scanner::new(root)
            .expect("scanner")
            .scan()
            .expect("scan")
    }

    // ------------------------------------------------------------------
    // test_parse_with_cache_creates_cache
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_with_cache_creates_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_test_repo(&tmp, "pub fn hello() {}");
        let counter = TokenCounter::new();

        let files = scan_files(&root);
        assert!(!files.is_empty(), "expected at least one scanned file");

        let (_parse_results, _content_map) = parse_with_cache(&files, &root, &counter, false);

        let cache_file = root.join(".cxpak").join("cache").join("artifacts.json");
        assert!(
            cache_file.exists(),
            "artifacts.json should have been created at {cache_file:?}"
        );
    }

    // ------------------------------------------------------------------
    // test_parse_with_cache_returns_parse_results
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_with_cache_returns_parse_results() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_test_repo(&tmp, "pub fn hello() {}\npub fn world() {}");
        let counter = TokenCounter::new();

        let files = scan_files(&root);
        let (results, _content_map) = parse_with_cache(&files, &root, &counter, false);

        // At least one parseable Rust file should appear in the map.
        assert!(
            !results.is_empty(),
            "expected non-empty parse results, got empty map"
        );
        // The result should contain symbols.
        let any_has_symbols = results.values().any(|pr| !pr.symbols.is_empty());
        assert!(any_has_symbols, "expected at least one symbol to be parsed");
    }

    // ------------------------------------------------------------------
    // test_parse_with_cache_cache_hit
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_with_cache_cache_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_test_repo(&tmp, "pub fn cached() {}");
        let counter = TokenCounter::new();

        let files = scan_files(&root);

        // First call — populates the cache.
        let (results_first, _) = parse_with_cache(&files, &root, &counter, false);

        // Verify cache exists.
        let cache_file = root.join(".cxpak").join("cache").join("artifacts.json");
        assert!(cache_file.exists());

        // Read the cache JSON before the second call.
        let cache_before = fs::read_to_string(&cache_file).unwrap();

        // Second call — should hit the cache.
        let (results_second, _) = parse_with_cache(&files, &root, &counter, false);

        // Both calls should return the same symbol names.
        let symbols_first: Vec<String> = results_first
            .values()
            .flat_map(|pr| pr.symbols.iter().map(|s| s.name.clone()))
            .collect();
        let symbols_second: Vec<String> = results_second
            .values()
            .flat_map(|pr| pr.symbols.iter().map(|s| s.name.clone()))
            .collect();
        assert_eq!(
            symbols_first, symbols_second,
            "cache hit should return identical results"
        );

        // The cache content should not have changed (same mtime/size).
        let cache_after = fs::read_to_string(&cache_file).unwrap();
        // Both should be valid JSON with the same entries; compare by
        // deserialising to avoid whitespace/ordering differences.
        let before: serde_json::Value = serde_json::from_str(&cache_before).unwrap();
        let after: serde_json::Value = serde_json::from_str(&cache_after).unwrap();
        assert_eq!(before, after, "cache should not change on a cache hit");
    }

    // ------------------------------------------------------------------
    // test_parse_with_cache_invalidates_on_change
    // ------------------------------------------------------------------
    #[test]
    fn test_parse_with_cache_invalidates_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_test_repo(&tmp, "pub fn original() {}");
        let counter = TokenCounter::new();

        let files = scan_files(&root);
        let (results_first, _) = parse_with_cache(&files, &root, &counter, false);

        let first_symbols: Vec<String> = results_first
            .values()
            .flat_map(|pr| pr.symbols.iter().map(|s| s.name.clone()))
            .collect();
        assert!(
            first_symbols.iter().any(|n| n == "original"),
            "expected symbol 'original' in first parse, got: {first_symbols:?}"
        );

        // Modify the file so its mtime and/or size changes.
        let file_path = root.join("src").join("lib.rs");
        // Sleep briefly to ensure mtime differs on filesystems with 1-second
        // granularity.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(&file_path, "pub fn renamed() {}").unwrap();

        // Re-scan so ScannedFile reflects new size.
        let files_updated = scan_files(&root);
        let (results_second, _) = parse_with_cache(&files_updated, &root, &counter, false);

        let second_symbols: Vec<String> = results_second
            .values()
            .flat_map(|pr| pr.symbols.iter().map(|s| s.name.clone()))
            .collect();

        assert!(
            second_symbols.iter().any(|n| n == "renamed"),
            "expected symbol 'renamed' after file change, got: {second_symbols:?}"
        );
        assert!(
            !second_symbols.iter().any(|n| n == "original"),
            "stale symbol 'original' should not appear after file change"
        );
    }

    // ------------------------------------------------------------------
    // test_parse_with_cache_multiple_files
    // ------------------------------------------------------------------
    /// Creates a repo with several Rust source files and verifies that the
    /// parallel implementation parses all of them correctly.
    #[test]
    fn test_parse_with_cache_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        // Initialise a git repo via git2 to avoid subprocess contention.
        let repo = git2::Repository::init(&root).expect("git2 init");
        let mut index = repo.index().expect("repo index");

        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        // Create five source files, each with a unique public function.
        let files_and_fns = [
            ("alpha.rs", "pub fn alpha() {}"),
            ("beta.rs", "pub fn beta() {}"),
            ("gamma.rs", "pub fn gamma() {}"),
            ("delta.rs", "pub fn delta() {}"),
            ("epsilon.rs", "pub fn epsilon() {}"),
        ];

        for (filename, source) in &files_and_fns {
            let path = src_dir.join(filename);
            fs::write(&path, source).unwrap();
            index
                .add_path(std::path::Path::new(&format!("src/{filename}")))
                .expect("git add");
        }
        index.write().expect("index write");

        let counter = TokenCounter::new();
        let scanned = scan_files(&root);

        // Expect all five files to be discovered.
        assert_eq!(
            scanned.len(),
            5,
            "expected 5 scanned files, got {}",
            scanned.len()
        );

        let (results, _content_map) = parse_with_cache(&scanned, &root, &counter, false);

        // All five files should appear in the results map.
        assert_eq!(
            results.len(),
            5,
            "expected parse results for all 5 files, got {}",
            results.len()
        );

        // Each file's unique function symbol must be present.
        let all_symbols: Vec<String> = results
            .values()
            .flat_map(|pr| pr.symbols.iter().map(|s| s.name.clone()))
            .collect();

        for expected_fn in ["alpha", "beta", "gamma", "delta", "epsilon"] {
            assert!(
                all_symbols.iter().any(|n| n == expected_fn),
                "expected symbol '{expected_fn}' in parallel parse results, got: {all_symbols:?}"
            );
        }
    }
}
