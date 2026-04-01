use crate::budget::counter::TokenCounter;
use crate::cli::OutputFormat;
use crate::commands::serve::build_index;
use crate::daemon::watcher::{FileChange, FileWatcher};
use crate::index::CodebaseIndex;
use crate::parser::LanguageRegistry;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

pub fn run(
    path: &Path,
    _token_budget: usize,
    _format: &OutputFormat,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut index = build_index(path)?;

    eprintln!(
        "cxpak: watching {} ({} files indexed, {} tokens)",
        path.display(),
        index.total_files,
        index.total_tokens
    );

    let watcher = FileWatcher::new(path)?;

    loop {
        if let Some(first) = watcher.recv_timeout(Duration::from_secs(1)) {
            let mut changes = vec![first];
            std::thread::sleep(Duration::from_millis(50));
            changes.extend(watcher.drain());

            let (modified_paths, removed_paths) = classify_changes(&changes, path);
            let update_count =
                apply_incremental_update(&mut index, path, &modified_paths, &removed_paths);

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
                    modified_paths.insert(rel.to_string_lossy().to_string());
                }
            }
            FileChange::Removed(p) => {
                if let Ok(rel) = p.strip_prefix(base_path) {
                    removed_paths.insert(rel.to_string_lossy().to_string());
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
}
