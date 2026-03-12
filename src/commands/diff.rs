use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::cli::OutputFormat;
use crate::git;
use crate::index::ranking;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::scanner::Scanner;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

/// A single file's changes from a git diff.
pub struct FileChange {
    /// Relative path of the changed file.
    pub path: String,
    /// The diff text (unified diff format lines).
    pub diff_text: String,
}

/// Extract changed files and their diffs.
/// If `git_ref` is None, diffs working tree against HEAD.
/// If `git_ref` is Some, diffs that ref's tree against HEAD's tree.
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

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    git_ref: Option<&str>,
    token_budget: usize,
    format: &OutputFormat,
    out: Option<&Path>,
    verbose: bool,
    all: bool,
    focus: Option<&str>,
    timing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let total_start = std::time::Instant::now();

    // 1. Extract git changes
    let extract_start = std::time::Instant::now();
    if verbose {
        eprintln!("cxpak: extracting git changes in {}", path.display());
    }
    let changes = extract_changes(path, git_ref)?;

    if changes.is_empty() {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(b"No changes detected.\n")?;
        return Ok(());
    }

    if timing {
        eprintln!("cxpak [timing]: extract    {:.1?}", extract_start.elapsed());
    }
    if verbose {
        eprintln!("cxpak: {} changed file(s)", changes.len());
    }

    // 2. Scan repo
    let scan_start = std::time::Instant::now();
    if verbose {
        eprintln!("cxpak: scanning {}", path.display());
    }
    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;
    if verbose {
        eprintln!("cxpak: found {} files", files.len());
    }
    if timing {
        eprintln!("cxpak [timing]: scan       {:.1?}", scan_start.elapsed());
    }

    let counter = TokenCounter::new();

    // 3. Parse with cache
    let parse_start = std::time::Instant::now();
    let parse_results = crate::cache::parse::parse_with_cache(&files, path, &counter, verbose);
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }

    // 4. Build index
    let index_start = std::time::Instant::now();
    let mut index = CodebaseIndex::build(files, parse_results, &counter);
    if verbose {
        eprintln!(
            "cxpak: indexed {} files, ~{} tokens total",
            index.total_files, index.total_tokens
        );
    }
    if timing {
        eprintln!("cxpak [timing]: index      {:.1?}", index_start.elapsed());
    }

    // 5. Build dependency graph
    let graph_start = std::time::Instant::now();
    let graph = crate::commands::trace::build_dependency_graph(&index);
    if timing {
        eprintln!("cxpak [timing]: graph      {:.1?}", graph_start.elapsed());
    }

    // 5b. Rank files and apply focus
    let git_ctx = git::extract_git_context(path, 20).ok();
    let file_paths: Vec<String> = index
        .files
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    let mut scores = ranking::rank_files(&file_paths, &graph, git_ctx.as_ref());
    if let Some(focus_path) = focus {
        ranking::apply_focus(&mut scores, focus_path, &graph);
    }

    // Sort index files by score so higher-ranked context files get budget priority
    let score_map: std::collections::HashMap<&str, f64> = scores
        .iter()
        .map(|s| (s.path.as_str(), s.composite))
        .collect();
    index.files.sort_by(|a, b| {
        let sa = score_map.get(a.relative_path.as_str()).unwrap_or(&0.0);
        let sb = score_map.get(b.relative_path.as_str()).unwrap_or(&0.0);
        sb.partial_cmp(sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    // 6. Determine the set of changed file paths (relative)
    let changed_paths: HashSet<String> = changes.iter().map(|c| c.path.clone()).collect();

    // 7. Walk graph from changed files: 1-hop or full BFS
    let relevant_paths: HashSet<String> = if all {
        let start: Vec<&str> = changed_paths.iter().map(String::as_str).collect();
        graph.reachable_from(&start)
    } else {
        let mut one_hop: HashSet<String> = changed_paths.clone();
        for file in &changed_paths {
            if let Some(deps) = graph.dependencies(file) {
                one_hop.extend(deps.iter().cloned());
            }
            for dep in graph.dependents(file) {
                one_hop.insert(dep.to_string());
            }
        }
        one_hop
    };

    // Context files: reachable but not themselves changed
    let context_paths: HashSet<String> =
        relevant_paths.difference(&changed_paths).cloned().collect();

    if verbose {
        eprintln!(
            "cxpak: {} context file(s) in dependency subgraph",
            context_paths.len()
        );
    }

    // 8. Build diff section text
    let render_start = std::time::Instant::now();
    let mut full_diff = String::new();
    for change in &changes {
        full_diff.push_str(&format!(
            "### {}\n\n```diff\n{}\n```\n\n",
            change.path, change.diff_text
        ));
    }

    // 9. Budget: diff first, then context signatures with the remainder
    let (diff_content, diff_used, _) =
        degrader::truncate_to_budget(&full_diff, token_budget, &counter, "diff");

    let context_budget = token_budget.saturating_sub(diff_used);
    let signatures = render_context_signatures(&index, &context_paths, context_budget, &counter);

    // 10. Metadata
    let git_ref_display = git_ref.unwrap_or("working tree");
    let metadata = format!(
        "- **Ref:** `{}`\n- **Changed files:** {}\n- **Context files:** {}\n",
        git_ref_display,
        changed_paths.len(),
        context_paths.len()
    );

    // 11. Assemble and render
    let sections = OutputSections {
        metadata,
        directory_tree: String::new(),
        module_map: String::new(),
        dependency_graph: String::new(),
        key_files: diff_content,
        signatures,
        git_context: String::new(),
    };

    let rendered = output::render(&sections, format);
    if timing {
        eprintln!("cxpak [timing]: render     {:.1?}", render_start.elapsed());
        eprintln!("cxpak [timing]: total      {:.1?}", total_start.elapsed());
    }

    match out {
        Some(out_path) => {
            std::fs::write(out_path, &rendered)?;
            if verbose {
                eprintln!("cxpak: written to {}", out_path.display());
            }
        }
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(rendered.as_bytes())?;
        }
    }

    Ok(())
}

/// Render public signatures of context files (reachable but not changed).
fn render_context_signatures(
    index: &CodebaseIndex,
    context_paths: &HashSet<String>,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut full = String::new();

    for file in &index.files {
        if !context_paths.contains(&file.relative_path) {
            continue;
        }
        let Some(pr) = &file.parse_result else {
            continue;
        };

        let public_syms: Vec<_> = pr
            .symbols
            .iter()
            .filter(|s| s.visibility == crate::parser::language::Visibility::Public)
            .collect();

        if public_syms.is_empty() {
            continue;
        }

        full.push_str(&format!("### {}\n\n", file.relative_path));
        for sym in public_syms {
            full.push_str(&format!("```\n{}\n```\n\n", sym.signature));
        }
    }

    let (budgeted, _, _) =
        degrader::truncate_to_budget(&full, budget, counter, "context signatures");
    budgeted
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
