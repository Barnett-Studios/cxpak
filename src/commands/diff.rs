use std::collections::HashMap;
use std::path::Path;

/// A single file's changes from a git diff.
// Task 8 will wire this into the CLI pipeline; suppress dead-code until then.
#[allow(dead_code)]
pub struct FileChange {
    /// Relative path of the changed file.
    pub path: String,
    /// The diff text (unified diff format lines).
    pub diff_text: String,
}

/// Extract changed files and their diffs.
/// If `git_ref` is None, diffs working tree against HEAD.
/// If `git_ref` is Some, diffs that ref's tree against HEAD's tree.
// Task 8 will call this from the CLI pipeline; suppress dead-code until then.
#[allow(dead_code)]
pub fn extract_changes(
    repo_path: &Path,
    git_ref: Option<&str>,
) -> Result<Vec<FileChange>, Box<dyn std::error::Error>> {
    let repo = git2::Repository::open(repo_path)?;

    let head_commit = repo.head()?.peel_to_commit()?;
    let head_tree = head_commit.tree()?;

    let diff = match git_ref {
        Some(refname) => {
            let obj = repo.revparse_single(refname)?;
            let ref_commit = obj.peel_to_commit()?;
            let ref_tree = ref_commit.tree()?;
            repo.diff_tree_to_tree(Some(&ref_tree), Some(&head_tree), None)?
        }
        None => repo.diff_tree_to_workdir_with_index(Some(&head_tree), None)?,
    };

    // `Diff::print` uses a single callback that receives every line event.
    // This avoids the two-simultaneous-mutable-closure borrow problem that
    // `Diff::foreach` would impose.  We track the current file path via the
    // `delta` argument on each line callback and accumulate text per path.
    let mut diff_map: HashMap<String, String> = HashMap::new();

    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        let path_str = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if path_str.is_empty() {
            return true;
        }

        let origin = line.origin();
        // Only capture actual diff content lines (added, removed, context).
        // Skip file headers (origin 'F'), hunk headers (origin 'H'), etc.
        let prefix = match origin {
            '+' => "+",
            '-' => "-",
            ' ' => " ",
            _ => return true,
        };

        let content = std::str::from_utf8(line.content()).unwrap_or("");
        let entry = diff_map.entry(path_str).or_default();
        entry.push_str(prefix);
        entry.push_str(content);

        true
    })?;

    let mut changes: Vec<FileChange> = diff_map
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(path, diff_text)| FileChange { path, diff_text })
        .collect();

    // Sort for deterministic ordering.
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_diff_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        dir
    }

    #[test]
    fn test_no_changes() {
        let repo = make_diff_repo();
        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_modified_file() {
        let repo = make_diff_repo();
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        )
        .unwrap();
        let changes = extract_changes(repo.path(), None).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/main.rs");
        assert!(changes[0].diff_text.contains("println"));
    }

    #[test]
    fn test_new_file() {
        let repo = make_diff_repo();
        std::fs::write(repo.path().join("src/lib.rs"), "pub fn hello() {}\n").unwrap();
        // Stage it so it shows in diff
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new("src/lib.rs")).unwrap();
        index.write().unwrap();

        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(changes.iter().any(|c| c.path == "src/lib.rs"));
    }

    #[test]
    fn test_multiple_changes() {
        let repo = make_diff_repo();
        std::fs::write(repo.path().join("src/main.rs"), "fn main() { todo!(); }\n").unwrap();
        std::fs::write(repo.path().join("src/lib.rs"), "pub fn foo() {}\n").unwrap();
        // Stage new file
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let mut index = git_repo.index().unwrap();
        index.add_path(std::path::Path::new("src/lib.rs")).unwrap();
        index.write().unwrap();

        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(changes.len() >= 2);
    }

    #[test]
    fn test_diff_with_ref() {
        let repo = make_diff_repo();
        // Make second commit
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"v2\"); }\n",
        )
        .unwrap();
        let git_repo = git2::Repository::open(repo.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        let mut index = git_repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = git_repo.find_tree(tree_id).unwrap();
        let head = git_repo.head().unwrap().peel_to_commit().unwrap();
        git_repo
            .commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&head])
            .unwrap();

        // Diff HEAD~1 vs HEAD
        let changes = extract_changes(repo.path(), Some("HEAD~1")).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/main.rs");
    }

    #[test]
    fn test_diff_text_has_plus_minus() {
        let repo = make_diff_repo();
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        )
        .unwrap();
        let changes = extract_changes(repo.path(), None).unwrap();
        assert!(!changes.is_empty());
        let diff = &changes[0].diff_text;
        assert!(
            diff.contains('+') || diff.contains('-'),
            "diff should have +/- markers"
        );
    }

    #[test]
    fn test_not_a_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = extract_changes(dir.path(), None);
        assert!(result.is_err());
    }
}
