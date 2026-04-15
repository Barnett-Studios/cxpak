use crate::budget::counter::TokenCounter;
use crate::cli::OutputFormat;
use crate::commands::serve::build_index;
use crate::daemon::watcher::{FileChange, FileWatcher};
use crate::index::CodebaseIndex;
use crate::parser::LanguageRegistry;
use std::collections::HashSet;
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
    let mut modified_paths = HashSet::new();
    let mut removed_paths = HashSet::new();

    for change in changes {
        match change {
            FileChange::Created(p) | FileChange::Modified(p) => {
                if let Ok(rel) = p.strip_prefix(base_path) {
                    // Use a lowercase key so that case-insensitive filesystems
                    // (macOS HFS+, Windows NTFS) don't produce duplicate entries
                    // for the same file with different capitalisation.
                    modified_paths.insert(rel.to_string_lossy().to_ascii_lowercase());
                }
            }
            FileChange::Removed(p) => {
                if let Ok(rel) = p.strip_prefix(base_path) {
                    removed_paths.insert(rel.to_string_lossy().to_ascii_lowercase());
                }
            }
        }
    }

    (modified_paths, removed_paths)
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

    for rel_path in removed_paths {
        index.remove_file(rel_path);
        update_count += 1;
    }

    for rel_path in modified_paths {
        if removed_paths.contains(rel_path) {
            continue;
        }
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
    fn test_apply_incremental_update_skip_removed_from_modified() {
        let dir = tempfile::TempDir::new().unwrap();

        let mut index = make_test_index();

        // File is in both modified and removed — should only count as removed
        let mut modified = HashSet::new();
        modified.insert("src/main.rs".to_string());
        let mut removed = HashSet::new();
        removed.insert("src/main.rs".to_string());

        let count = apply_incremental_update(&mut index, dir.path(), &modified, &removed);
        // Only the remove counts, not the modify
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
}
