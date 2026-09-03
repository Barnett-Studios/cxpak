use crate::budget::counter::TokenCounter;
use crate::cli::OutputFormat;
use crate::commands::serve::build_index;
use crate::daemon::watcher::{FileChange, FileWatcher};
use crate::index::CodebaseIndex;
use crate::parser::LanguageRegistry;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

/// Maximum total debounce time per event burst to prevent infinite-event streams
/// from blocking the rebuild indefinitely.
const MAX_DEBOUNCE_ITERS: usize = 40; // 40 × 50 ms = 2 s

pub fn run(
    path: &Path,
    token_budget: usize,
    format: &OutputFormat,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Canonicalize the base path so that absolute paths delivered by `notify`
    // can be stripped with `strip_prefix` without mismatch (e.g. "." vs "/abs/path").
    let canon_path = path
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize watch path {}: {e}", path.display()))?;

    let mut index = build_index(&canon_path)?;

    eprintln!(
        "cxpak: watching {} ({} files indexed, {} tokens, budget={}, format={:?})",
        canon_path.display(),
        index.total_files,
        index.total_tokens,
        token_budget,
        format
    );

    let watcher = FileWatcher::new(&canon_path)?;

    loop {
        if let Some(first) = watcher.recv_timeout(Duration::from_secs(1)) {
            let mut changes = vec![first];
            // Drain the queue in 50 ms slices until it goes quiet, capped at
            // MAX_DEBOUNCE_ITERS to avoid being blocked by pathological event floods.
            let mut iters = 0;
            loop {
                std::thread::sleep(Duration::from_millis(50));
                let batch = watcher.drain();
                if batch.is_empty() || iters >= MAX_DEBOUNCE_ITERS {
                    break;
                }
                changes.extend(batch);
                iters += 1;
            }

            let (modified_paths, removed_paths) = classify_changes(&changes, &canon_path);
            let update_count =
                apply_incremental_update(&mut index, &canon_path, &modified_paths, &removed_paths);

            if update_count > 0 {
                index.rebuild_graph();
                index.pagerank =
                    crate::intelligence::pagerank::compute_pagerank(&index.graph, 0.85, 100);
                let paths: std::collections::HashSet<String> = index
                    .files
                    .iter()
                    .map(|f| f.relative_path.clone())
                    .collect();
                index.test_map =
                    crate::intelligence::test_map::build_test_map(&index.files, &paths);
                eprintln!(
                    "cxpak: updated {} file(s), {} files / {} tokens total",
                    update_count, index.total_files, index.total_tokens
                );
            }
        }
    }
}

/// Classify file changes into modified and removed path sets.
pub(crate) fn classify_changes(
    changes: &[FileChange],
    base_path: &Path,
) -> (HashSet<String>, HashSet<String>) {
    // Keyed on the ASCII-folded path, valued with the spelling the event
    // carried, first one wins. That collapses two events for one file on a
    // case-insensitive volume — the reason the old code lowercased — without
    // storing the folded spelling, which is what broke #33. On a case-sensitive
    // volume two genuinely distinct files that fold together still collapse to
    // one; that is unchanged from the lowercasing this replaces, which collapsed
    // them *and* corrupted the key.
    let mut modified_by_fold: HashMap<String, String> = HashMap::new();
    let mut removed_by_fold: HashMap<String, String> = HashMap::new();

    // The recursive FileWatcher fires on every path under the root, including
    // build/noise trees (target/, .cxpak/, node_modules/, dist/, …), binary
    // assets, lockfiles, and .git internals. The index is built by Scanner::scan,
    // which excludes all of these — git's ignore rules PLUS BUILTIN_IGNORES and
    // an optional .cxpakignore. The watcher must apply the *same* exclusion set,
    // or committed-but-noise files (Cargo.lock, *.png, dist/, *.min.js,
    // .DS_Store — none of which git ignores) leak into the live index, drift it
    // away from a fresh scan, and re-trigger a full rebuild on every build.
    //
    // git2 covers .gitignore / core.excludesFile / .git/info/exclude (incl.
    // nested dirs); the ignore-crate matcher covers BUILTIN_IGNORES +
    // .cxpakignore, built exactly as Scanner::scan builds them. base_path is
    // always a repo root (Scanner::new requires <root>/.git before any watcher
    // starts), so `discover` resolves it — and `discover`, not `open`, so a
    // non-root root can't silently fail-open to no filtering.
    let repo = git2::Repository::discover(base_path).ok();
    let noise = {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(base_path);
        for &pattern in crate::scanner::defaults::BUILTIN_IGNORES {
            let _ = builder.add_line(None, pattern);
        }
        let cxpakignore = base_path.join(".cxpakignore");
        if cxpakignore.is_file() {
            let _ = builder.add(&cxpakignore);
        }
        builder.build().ok()
    };
    let is_ignored = |abs: &Path| -> bool {
        let Ok(rel) = abs.strip_prefix(base_path) else {
            return false;
        };
        // git never reports .git/ itself as "ignored", but it must never index.
        if rel.components().any(|c| c.as_os_str() == ".git") {
            return true;
        }
        // BUILTIN_IGNORES / .cxpakignore — Scanner's non-git exclusions.
        if let Some(gi) = &noise {
            if gi.matched_path_or_any_parents(rel, false).is_ignore() {
                return true;
            }
        }
        // .gitignore / global excludes / .git/info/exclude, nested-aware.
        repo.as_ref()
            .map(|r| r.is_path_ignored(rel).unwrap_or(false))
            .unwrap_or(false)
    };

    for change in changes {
        match change {
            FileChange::Created(p) | FileChange::Modified(p) => {
                if is_ignored(p) {
                    continue;
                }
                if let Ok(rel) = p.strip_prefix(base_path) {
                    // The on-disk case, because that is what the index stores
                    // (`Scanner` keys on `relative_path` verbatim). Lowercasing
                    // here made `remove_file("readme.md")` miss the stored
                    // `README.md` entirely and made a modify fork a second,
                    // orphaned lowercase entry (#33). Duplicate spellings of one
                    // file — the reason the lowercasing was here — are collapsed
                    // by `resolve_stored_key` below, against the index rather
                    // than against a guess.
                    let real = rel.to_string_lossy().to_string();
                    modified_by_fold
                        .entry(real.to_ascii_lowercase())
                        .or_insert(real);
                }
            }
            FileChange::Removed(p) => {
                if is_ignored(p) {
                    continue;
                }
                if let Ok(rel) = p.strip_prefix(base_path) {
                    let real = rel.to_string_lossy().to_string();
                    removed_by_fold
                        .entry(real.to_ascii_lowercase())
                        .or_insert(real);
                }
            }
        }
    }

    (
        modified_by_fold.into_values().collect(),
        removed_by_fold.into_values().collect(),
    )
}

/// The key the index actually stores for `rel`, when it stores one.
///
/// A case-insensitive filesystem can report an event for `readme.md` against a
/// file the scanner stored as `README.md`; upserting the event's spelling would
/// leave two entries for one file. Resolution is deliberately
/// conservative: exactly one case-insensitive match resolves to it; zero
/// matches (a new file) or more than one return `rel` unchanged, because a
/// rewrite there would be a guess.
///
/// A path that already names a stored file exactly comes back unchanged, and
/// needs no branch of its own to do it: it is either the only fold match, so it
/// resolves to itself, or one of several, so the ambiguity rule passes it
/// through. An earlier draft special-cased it and no mutation could kill the
/// test covering that branch — which is what a redundant branch looks like from
/// the outside.
///
/// ASCII folding only, matching the `to_ascii_lowercase` this replaces — a
/// non-ASCII case difference is left alone rather than resolved wrongly.
///
/// Linear in the index for each path. Watch batches are a handful of files, so
/// this is cheaper than building a fold map per call; if a batch ever carries
/// hundreds of paths, build the map once instead.
fn resolve_stored_key(index: &CodebaseIndex, rel: &str) -> String {
    let mut resolved: Option<&str> = None;
    for f in &index.files {
        if f.relative_path.eq_ignore_ascii_case(rel) {
            if resolved.is_some() {
                return rel.to_string();
            }
            resolved = Some(&f.relative_path);
        }
    }
    resolved
        .map(str::to_string)
        .unwrap_or_else(|| rel.to_string())
}

/// Apply incremental changes to the index. Returns the number of files updated.
pub(crate) fn apply_incremental_update(
    index: &mut CodebaseIndex,
    base_path: &Path,
    modified_paths: &HashSet<String>,
    removed_paths: &HashSet<String>,
) -> usize {
    let counter = TokenCounter::new();
    let registry = LanguageRegistry::new();
    let mut update_count = 0;

    // Resolved up front, and against the index as it stands: `remove_file` below
    // mutates `index.files`, so resolving a modified path afterwards could read a
    // different answer than the same path resolved before.
    let removed: HashSet<String> = removed_paths
        .iter()
        .map(|p| resolve_stored_key(index, p))
        .collect();
    let modified: Vec<String> = modified_paths
        .iter()
        .map(|p| resolve_stored_key(index, p))
        .collect();

    for rel_path in &removed {
        index.remove_file(rel_path);
        update_count += 1;
    }

    for rel_path in &modified {
        // No `removed.contains(..)` skip here. One debounced batch coalesces to
        // at most one entry per (path, kind), so a delete-then-write inside the
        // window arrives as Removed(P) AND Created(P) with nothing in the batch
        // to order them by. Letting the removal win dropped a file that exists
        // on disk out of the served index until the next full rescan (#77).
        // The removal loop above has already run, so what decides is the read
        // below — the only truth available at apply time: a path still on disk
        // is re-indexed, a path genuinely gone fails to open and stays removed.
        let abs_path = base_path.join(rel_path);
        if let Ok(content) = std::fs::read_to_string(&abs_path) {
            let lang_name = crate::scanner::detect_language(Path::new(rel_path));
            let parse_result = lang_name.as_deref().and_then(|ln| {
                registry.get(ln).and_then(|lang| {
                    let ts_lang = lang.ts_language();
                    let mut parser = tree_sitter::Parser::new();
                    parser.set_language(&ts_lang).ok()?;
                    let tree = parser.parse(&content, None)?;
                    Some(lang.extract(&content, &tree))
                })
            });

            index.upsert_file(
                rel_path,
                lang_name.as_deref(),
                &content,
                parse_result,
                &counter,
                None,
            );
            update_count += 1;
        }
    }

    update_count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map)
    }

    #[test]
    fn test_classify_changes_created() {
        let base = PathBuf::from("/repo");
        let changes = vec![FileChange::Created(PathBuf::from("/repo/src/new.rs"))];
        let (modified, removed) = classify_changes(&changes, &base);
        assert!(modified.contains("src/new.rs"));
        assert!(removed.is_empty());
    }

    #[test]
    fn test_classify_changes_modified() {
        let base = PathBuf::from("/repo");
        let changes = vec![FileChange::Modified(PathBuf::from("/repo/src/main.rs"))];
        let (modified, removed) = classify_changes(&changes, &base);
        assert!(modified.contains("src/main.rs"));
        assert!(removed.is_empty());
    }

    #[test]
    fn test_classify_changes_removed() {
        let base = PathBuf::from("/repo");
        let changes = vec![FileChange::Removed(PathBuf::from("/repo/src/old.rs"))];
        let (modified, removed) = classify_changes(&changes, &base);
        assert!(modified.is_empty());
        assert!(removed.contains("src/old.rs"));
    }

    #[test]
    fn test_classify_changes_mixed() {
        let base = PathBuf::from("/repo");
        let changes = vec![
            FileChange::Created(PathBuf::from("/repo/a.rs")),
            FileChange::Modified(PathBuf::from("/repo/b.rs")),
            FileChange::Removed(PathBuf::from("/repo/c.rs")),
        ];
        let (modified, removed) = classify_changes(&changes, &base);
        assert_eq!(modified.len(), 2);
        assert!(modified.contains("a.rs"));
        assert!(modified.contains("b.rs"));
        assert_eq!(removed.len(), 1);
        assert!(removed.contains("c.rs"));
    }

    #[test]
    fn test_classify_changes_skips_git_ignored() {
        // git-ignored trees (target/, .cxpak/) and .git internals must be
        // dropped so the watcher never ingests them into the index.
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "/target\n.cxpak/\n").unwrap();
        let base = dir.path();

        let changes = vec![
            FileChange::Modified(base.join("src/main.rs")),
            FileChange::Modified(base.join("target/debug/foo.rs")),
            FileChange::Created(base.join(".cxpak/cache/root/detail.md")),
            FileChange::Removed(base.join(".git/index")),
        ];
        let (modified, removed) = classify_changes(&changes, base);

        assert!(modified.contains("src/main.rs"), "tracked source kept");
        assert!(
            !modified.iter().any(|p| p.starts_with("target")),
            "target/ dropped: {modified:?}"
        );
        assert!(
            !modified.iter().any(|p| p.contains("cxpak")),
            ".cxpak/ dropped: {modified:?}"
        );
        assert!(removed.is_empty(), ".git/ dropped: {removed:?}");
    }

    #[test]
    fn test_classify_changes_skips_builtin_noise_not_gitignored() {
        // BUILTIN_IGNORES files are committed (NOT git-ignored) but the initial
        // Scanner excludes them, so the watcher must too — otherwise Cargo.lock
        // churn on every `cargo build`, images, and minified/dist artifacts leak
        // into the live index. .gitignore here deliberately lists ONLY /target,
        // so the git-only filter would have let all of these through.
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "/target\n").unwrap();
        let base = dir.path();

        let changes = vec![
            FileChange::Modified(base.join("Cargo.lock")),
            FileChange::Modified(base.join("package-lock.json")),
            FileChange::Created(base.join("assets/logo.png")),
            FileChange::Created(base.join("web/dist/app.min.js")),
            FileChange::Created(base.join(".DS_Store")),
            FileChange::Modified(base.join("src/main.rs")),
        ];
        let (modified, _removed) = classify_changes(&changes, base);

        assert!(modified.contains("src/main.rs"), "real source kept");
        for noise in [
            "cargo.lock",
            "package-lock.json",
            "logo.png",
            "min.js",
            "dist/",
            ".ds_store",
        ] {
            assert!(
                !modified.iter().any(|p| p.contains(noise)),
                "builtin noise {noise:?} must be dropped: {modified:?}"
            );
        }
    }

    #[test]
    fn test_classify_changes_outside_base_ignored() {
        let base = PathBuf::from("/repo");
        let changes = vec![FileChange::Created(PathBuf::from("/other/file.rs"))];
        let (modified, removed) = classify_changes(&changes, &base);
        assert!(modified.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn test_apply_incremental_update_remove() {
        let mut index = make_test_index();
        assert_eq!(index.total_files, 1);

        let modified = HashSet::new();
        let mut removed = HashSet::new();
        removed.insert("src/main.rs".to_string());

        let count = apply_incremental_update(&mut index, Path::new("/tmp"), &modified, &removed);
        assert_eq!(count, 1);
        assert_eq!(index.total_files, 0);
    }

    #[test]
    fn test_apply_incremental_update_add_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("new.rs");
        std::fs::write(&file_path, "fn new_func() {}").unwrap();

        let mut index = make_test_index();
        let initial_files = index.total_files;

        let mut modified = HashSet::new();
        modified.insert("new.rs".to_string());
        let removed = HashSet::new();

        let count = apply_incremental_update(&mut index, dir.path(), &modified, &removed);
        assert_eq!(count, 1);
        assert_eq!(index.total_files, initial_files + 1);
    }

    #[test]
    fn test_apply_incremental_update_removed_and_gone_from_disk_stays_removed() {
        let dir = tempfile::TempDir::new().unwrap();

        let mut index = make_test_index();

        // The path is in both sets and is NOT on disk — the delete half of a
        // remove+recreate batch where nothing came back. The re-read below the
        // removal loop fails to open it, so the removal stands and only it counts.
        let mut modified = HashSet::new();
        modified.insert("src/main.rs".to_string());
        let mut removed = HashSet::new();
        removed.insert("src/main.rs".to_string());

        let count = apply_incremental_update(&mut index, dir.path(), &modified, &removed);
        assert_eq!(count, 1);
        assert_eq!(index.total_files, 0);
    }

    #[test]
    fn test_apply_incremental_update_nonexistent_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut index = make_test_index();

        let mut modified = HashSet::new();
        modified.insert("does_not_exist.rs".to_string());
        let removed = HashSet::new();

        let count = apply_incremental_update(&mut index, dir.path(), &modified, &removed);
        // File doesn't exist, read_to_string fails, so no update
        assert_eq!(count, 0);
    }

    /// Case-insensitive FS dedup: two events for the same file with different case must
    /// produce a single entry in the modified set.
    #[test]
    fn test_classify_changes_deduplicates_case_variants() {
        let base = PathBuf::from("/repo");
        let changes = vec![
            FileChange::Modified(PathBuf::from("/repo/Src/Main.rs")),
            FileChange::Modified(PathBuf::from("/repo/src/main.rs")),
        ];
        let (modified, _removed) = classify_changes(&changes, &base);
        assert_eq!(
            modified.len(),
            1,
            "case variants of the same path must dedup to one entry, got: {modified:?}"
        );
    }

    /// classify_changes with a canonical absolute base path returns a non-empty result.
    #[test]
    fn test_classify_changes_with_canonical_base() {
        let dir = tempfile::TempDir::new().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let abs_file = canon.join("src").join("new.rs");

        let changes = vec![FileChange::Created(abs_file)];
        let (modified, removed) = classify_changes(&changes, &canon);
        assert!(
            modified.contains("src/new.rs"),
            "canonical base must allow strip_prefix, got modified={modified:?}"
        );
        assert!(removed.is_empty());
    }

    /// The debounce cap constant must be positive (compile-time guarantee).
    const _: () = assert!(MAX_DEBOUNCE_ITERS > 0);

    /// After `apply_incremental_update` removes a file, `rebuild_graph` must
    /// purge stale edges from the dependency graph.
    ///
    /// This verifies the FIX-WAVE5 #1 fix: without the rebuild_graph() call
    /// after apply_incremental_update(), the graph retains edges from/to the
    /// removed file indefinitely.
    #[test]
    fn test_graph_rebuilt_after_remove() {
        use crate::budget::counter::TokenCounter;
        use crate::index::CodebaseIndex;
        use crate::scanner::ScannedFile;
        use crate::schema::EdgeType;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let counter = TokenCounter::new();
        // Three files: a.rs → b.rs → c.rs
        let files = vec![
            ScannedFile {
                relative_path: "a.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/a.rs"),
                language: Some("rust".to_string()),
                size_bytes: 10,
            },
            ScannedFile {
                relative_path: "b.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/b.rs"),
                language: Some("rust".to_string()),
                size_bytes: 10,
            },
            ScannedFile {
                relative_path: "c.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/c.rs"),
                language: Some("rust".to_string()),
                size_bytes: 10,
            },
        ];
        let mut content_map = HashMap::new();
        content_map.insert("a.rs".to_string(), "fn a() {}".to_string());
        content_map.insert("b.rs".to_string(), "fn b() {}".to_string());
        content_map.insert("c.rs".to_string(), "fn c() {}".to_string());
        let mut index =
            CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);

        // Manually inject edges so the graph has known edges
        index.graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        index.graph.add_edge("b.rs", "c.rs", EdgeType::Import);

        assert!(
            index.graph.edges.contains_key("a.rs"),
            "edge a→b must exist before remove"
        );

        // Remove b.rs via apply_incremental_update, then rebuild graph
        let mut removed = HashSet::new();
        removed.insert("b.rs".to_string());
        let count = apply_incremental_update(
            &mut index,
            std::path::Path::new("/tmp"),
            &HashSet::new(),
            &removed,
        );
        assert_eq!(count, 1);

        // Now rebuild graph as the watch loop does after update_count > 0
        index.rebuild_graph();

        // b.rs edges must be gone after rebuild
        assert!(
            !index.graph.edges.contains_key("b.rs"),
            "stale edges from removed file b.rs must be purged after rebuild_graph"
        );
    }
}

#[cfg(test)]
mod case_preservation_tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// An index holding files whose stored keys are the real on-disk case.
    fn index_with(paths: &[&str]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let mut content_map = HashMap::new();
        let files = paths
            .iter()
            .map(|p| {
                content_map.insert((*p).to_string(), "fn main() {}".to_string());
                ScannedFile {
                    relative_path: (*p).to_string(),
                    absolute_path: PathBuf::from("/repo").join(p),
                    language: Some("rust".to_string()),
                    size_bytes: 100,
                }
            })
            .collect();
        CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map)
    }

    fn stored(index: &CodebaseIndex) -> Vec<String> {
        let mut v: Vec<String> = index
            .files
            .iter()
            .map(|f| f.relative_path.clone())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn classify_changes_keeps_the_on_disk_case() {
        let base = PathBuf::from("/repo");
        let (modified, removed) = classify_changes(
            &[
                FileChange::Modified(base.join("README.md")),
                FileChange::Removed(base.join("src/Main.rs")),
            ],
            &base,
        );
        assert!(
            modified.contains("README.md"),
            "modified set lost the on-disk case: {modified:?}"
        );
        assert!(
            removed.contains("src/Main.rs"),
            "removed set lost the on-disk case: {removed:?}"
        );
    }

    #[test]
    fn batch_dedup_keeps_the_first_spelling_seen() {
        // Which spelling survives a collapse does not affect correctness — a
        // known file is resolved against the index anyway — but it must be
        // decided by something, or the same batch can key the index differently
        // between runs. Input order decides it.
        let base = PathBuf::from("/repo");
        let (modified, _) = classify_changes(
            &[
                FileChange::Modified(base.join("Src/Main.rs")),
                FileChange::Modified(base.join("src/main.rs")),
            ],
            &base,
        );
        assert_eq!(
            modified.iter().collect::<Vec<_>>(),
            vec!["Src/Main.rs"],
            "the surviving spelling is not the first event's"
        );
    }

    #[test]
    fn a_removed_mixed_case_file_leaves_the_index() {
        let mut index = index_with(&["README.md", "src/main.rs"]);
        let base = PathBuf::from("/repo");
        let (modified, removed) =
            classify_changes(&[FileChange::Removed(base.join("README.md"))], &base);
        apply_incremental_update(&mut index, &base, &modified, &removed);
        assert_eq!(
            stored(&index),
            vec!["src/main.rs".to_string()],
            "README.md survived its own deletion — it is served as live content forever"
        );
    }

    // Resolution is tested directly rather than through a modify round-trip.
    // `apply_incremental_update` reads the file before upserting, so on a
    // case-sensitive volume — which every CI runner here is — a differently
    // cased path simply fails to open and the round-trip passes without ever
    // exercising the rule. That test would be green on the broken code.

    #[test]
    fn resolution_maps_a_differently_cased_event_onto_the_stored_key() {
        // The case a case-insensitive filesystem produces: an event spelled
        // `readme.md` for a file the scanner stored as `README.md`. Taking the
        // event's spelling forks the entry, which is what the lowercasing this
        // fix removes was defending against.
        let index = index_with(&["README.md", "src/main.rs"]);
        assert_eq!(resolve_stored_key(&index, "readme.md"), "README.md");
        assert_eq!(resolve_stored_key(&index, "SRC/MAIN.RS"), "src/main.rs");
    }

    #[test]
    fn resolution_returns_an_exactly_matching_path_unchanged() {
        // Stored in this order deliberately: the exact hit is seen first, so an
        // implementation that let a later fold match win would return the other
        // spelling and take the wrong entry.
        let index = index_with(&["readme.md", "README.md"]);
        assert_eq!(resolve_stored_key(&index, "readme.md"), "readme.md");
        assert_eq!(resolve_stored_key(&index, "README.md"), "README.md");
    }

    #[test]
    fn a_created_mixed_case_file_enters_the_index_with_its_real_case() {
        // The case `resolve_stored_key` cannot rescue: a file the index has
        // never seen has nothing to resolve against, so whatever
        // `classify_changes` emits is what gets stored — permanently. This is
        // why the lowercasing had to go rather than merely be compensated for.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("Widget.rs"), "pub fn widget() {}\n").unwrap();

        let mut index = index_with(&["src/main.rs"]);
        let (modified, removed) = classify_changes(
            &[FileChange::Created(dir.path().join("Widget.rs"))],
            dir.path(),
        );
        apply_incremental_update(&mut index, dir.path(), &modified, &removed);

        assert!(
            stored(&index).contains(&"Widget.rs".to_string()),
            "created file was indexed under a case that is not on disk: {:?}",
            stored(&index)
        );
    }

    #[test]
    fn resolution_leaves_an_ambiguous_or_unknown_path_alone() {
        // Two stored files fold together and neither matches exactly: any
        // rewrite would be a guess, so the path is passed through.
        let index = index_with(&["README.md", "ReadMe.md"]);
        assert_eq!(resolve_stored_key(&index, "readme.md"), "readme.md");
        // A file the index has never seen — a create — must reach upsert with
        // the case the event carried.
        assert_eq!(resolve_stored_key(&index, "src/New.rs"), "src/New.rs");

        // Matching is whole-path, not substring: `main.rs` is not `src/main.rs`,
        // and resolving it onto one would silently retarget the update at a
        // different file. Nothing else in this module fails if that breaks.
        let nested = index_with(&["src/main.rs"]);
        assert_eq!(resolve_stored_key(&nested, "main.rs"), "main.rs");
    }

    #[test]
    fn an_exact_match_wins_when_both_cases_are_indexed() {
        // The round-trip through `remove_file` for the case a case-sensitive
        // volume can produce. Subsumed as a discriminator by
        // `resolution_returns_an_exactly_matching_path_unchanged` — no mutation
        // kills this one alone — and kept as the end-to-end check that the
        // resolved key is what actually reaches the index.
        let mut index = index_with(&["README.md", "readme.md"]);
        let base = PathBuf::from("/repo");
        let mut removed = HashSet::new();
        removed.insert("readme.md".to_string());
        apply_incremental_update(&mut index, &base, &HashSet::new(), &removed);
        assert_eq!(
            stored(&index),
            vec!["README.md".to_string()],
            "removing readme.md took the wrong entry"
        );
    }

    /// One debounce batch can legitimately carry both a removal and a
    /// re-creation of one path: `collect_debounced` coalesces to at most one
    /// entry per (path, kind), so a delete-then-write inside the window
    /// produces exactly this pair — the documented contract, not an edge
    /// case — and the batch carries nothing to break the tie with. What is
    /// on disk when the update is applied is the only truth available, so the
    /// update must reflect it rather than let the removal win by construction.
    #[test]
    fn a_batch_that_removes_and_recreates_a_file_keeps_it() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/main.rs");
        std::fs::write(&path, "fn main() { println!(\"rewritten\"); }\n").unwrap();

        let mut index = index_with(&["src/main.rs", "src/other.rs"]);
        let (modified, removed) = classify_changes(
            &[
                FileChange::Removed(path.clone()),
                FileChange::Created(path.clone()),
            ],
            dir.path(),
        );
        // Without this the fixture could assert nothing: if classification ever
        // stopped emitting the path in BOTH sets, the body below would pass
        // while never reaching the branch under test.
        assert!(
            modified.contains("src/main.rs") && removed.contains("src/main.rs"),
            "fixture must put the path in both sets: modified={modified:?} removed={removed:?}"
        );

        apply_incremental_update(&mut index, dir.path(), &modified, &removed);

        assert!(
            stored(&index).contains(&"src/main.rs".to_string()),
            "a file present on disk when the update was applied is missing from the index: {:?}",
            stored(&index)
        );
    }

    /// The same batch for a mixed-case path, and it is not the lowercase test
    /// with different letters. This path was exempt by accident before #33 —
    /// the lowercasing made `remove_file` miss, so the removal half silently
    /// did nothing and no re-add was ever needed. With that accidental
    /// exemption gone the removal now lands, which is what makes the defect
    /// reachable here at all.
    ///
    /// The events carry `readme.md` — the spelling a case-insensitive volume
    /// reports for a file the scanner stored as `README.md` — so the batch also
    /// runs through `resolve_stored_key`. Asserting the exact stored set pins
    /// both halves: the file survives, AND it survives under its on-disk case
    /// rather than forking a second lowercase entry. Resolving the modified
    /// paths after the removals instead of before fails this test and no other.
    #[test]
    fn a_batch_that_removes_and_recreates_a_mixed_case_file_keeps_it() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# rewritten\n").unwrap();
        let event_path = dir.path().join("readme.md");

        let mut index = index_with(&["README.md", "src/main.rs"]);
        let (modified, removed) = classify_changes(
            &[
                FileChange::Removed(event_path.clone()),
                FileChange::Created(event_path.clone()),
            ],
            dir.path(),
        );
        assert!(
            modified.contains("readme.md") && removed.contains("readme.md"),
            "fixture must put the path in both sets: modified={modified:?} removed={removed:?}"
        );

        apply_incremental_update(&mut index, dir.path(), &modified, &removed);

        assert_eq!(
            stored(&index),
            vec!["README.md".to_string(), "src/main.rs".to_string()],
            "a mixed-case file present on disk was dropped or re-entered under the wrong case"
        );
    }

    /// The control for the two above: what decides is the *pair*, not mere
    /// presence on disk. A removal with no re-creation in the same batch must
    /// still remove the entry even when a stale file sits at the path — which
    /// is what a watcher sees when the removal event is delivered before the
    /// unlink is visible, or when an editor's atomic-save temp file is still
    /// there. An over-fix that kept anything readable passes both tests above
    /// and fails only this one.
    #[test]
    fn a_removal_alone_still_removes_a_file_that_is_still_on_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# still here\n").unwrap();

        let mut index = index_with(&["README.md", "src/main.rs"]);
        let (modified, removed) = classify_changes(
            &[FileChange::Removed(dir.path().join("README.md"))],
            dir.path(),
        );
        apply_incremental_update(&mut index, dir.path(), &modified, &removed);

        assert_eq!(
            stored(&index),
            vec!["src/main.rs".to_string()],
            "a removal with no re-creation in the batch must remove the entry"
        );
    }
}
