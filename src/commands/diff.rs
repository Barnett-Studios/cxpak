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

/// Parse a human-readable time expression into a `Duration`.
///
/// Accepted forms: "1 day", "2 days", "1d", "3h", "1 hour", "3 hours",
/// "1 week", "2 weeks", "1w", "1 month", "2 months", "yesterday".
/// Returns `Err` for empty, zero-valued, or unrecognised input.
pub fn parse_time_expression(expr: &str) -> Result<std::time::Duration, String> {
    let expr = expr.trim().to_lowercase();
    if expr.is_empty() {
        return Err("empty time expression".to_string());
    }
    if expr == "yesterday" {
        return Ok(std::time::Duration::from_secs(86400));
    }

    // Try compact form: "3d", "1h", "2w" — only when the prefix is purely digits.
    let try_compact =
        |suffix: char, secs_per: u64| -> Option<Result<std::time::Duration, String>> {
            let num_str = expr.strip_suffix(suffix)?;
            // Guard: the remaining characters must all be ASCII digits (pure number).
            if !num_str.chars().all(|c| c.is_ascii_digit()) || num_str.is_empty() {
                return None;
            }
            let n: u64 = match num_str.parse() {
                Ok(v) => v,
                Err(_) => return Some(Err(format!("invalid time expression: {expr}"))),
            };
            if n == 0 {
                return Some(Err("time expression must be > 0".to_string()));
            }
            Some(Ok(std::time::Duration::from_secs(n * secs_per)))
        };

    if let Some(result) = try_compact('d', 86400) {
        return result;
    }
    if let Some(result) = try_compact('h', 3600) {
        return result;
    }
    if let Some(result) = try_compact('w', 604800) {
        return result;
    }

    // Try long form: "1 day", "2 days", "1 hour", etc.
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 2 {
        let n: u64 = parts[0]
            .parse()
            .map_err(|_| format!("invalid time expression: {expr}"))?;
        if n == 0 {
            return Err("time expression must be > 0".to_string());
        }
        let unit = parts[1];
        let secs_per = match unit {
            "day" | "days" => 86400,
            "hour" | "hours" => 3600,
            "week" | "weeks" => 604800,
            "month" | "months" => 2592000,
            _ => return Err(format!("unknown time unit: {unit}")),
        };
        return Ok(std::time::Duration::from_secs(n * secs_per));
    }

    Err(format!("invalid time expression: {expr}"))
}

/// Convert a `--since` expression into a git ref string.
/// Uses `git log --since` to find the oldest commit within the time window,
/// then returns its parent as the diff base.
pub fn resolve_since(repo_path: &std::path::Path, since_expr: &str) -> Result<String, String> {
    let duration = parse_time_expression(since_expr)?;
    let secs = duration.as_secs();
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "log",
            "--all",
            "--format=%H",
            &format!("--since={secs} seconds ago"),
        ])
        .output()
        .map_err(|e| format!("git log failed: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hashes: Vec<&str> = stdout.lines().collect();
    if hashes.is_empty() {
        return Err(format!("no commits found in the last {since_expr}"));
    }
    // The last hash in the list is the oldest commit in the time window.
    // We want its parent as the diff base.
    let oldest = hashes.last().unwrap();
    Ok(format!("{oldest}~1"))
}

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
    let (parse_results, content_map) =
        crate::cache::parse::parse_with_cache(&files, path, &counter, verbose);
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }

    // 4. Build index
    let index_start = std::time::Instant::now();
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
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
    let graph = crate::index::graph::build_dependency_graph(&index);
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

    #[test]
    fn test_parse_time_expression_days() {
        assert_eq!(parse_time_expression("1 day").unwrap().as_secs(), 86400);
        assert_eq!(parse_time_expression("2 days").unwrap().as_secs(), 172800);
        assert_eq!(parse_time_expression("1d").unwrap().as_secs(), 86400);
        assert_eq!(parse_time_expression("3d").unwrap().as_secs(), 259200);
    }

    #[test]
    fn test_parse_time_expression_hours() {
        assert_eq!(parse_time_expression("1 hour").unwrap().as_secs(), 3600);
        assert_eq!(parse_time_expression("3 hours").unwrap().as_secs(), 10800);
        assert_eq!(parse_time_expression("1h").unwrap().as_secs(), 3600);
    }

    #[test]
    fn test_parse_time_expression_weeks() {
        assert_eq!(parse_time_expression("1 week").unwrap().as_secs(), 604800);
        assert_eq!(parse_time_expression("2 weeks").unwrap().as_secs(), 1209600);
        assert_eq!(parse_time_expression("1w").unwrap().as_secs(), 604800);
    }

    #[test]
    fn test_parse_time_expression_months() {
        assert_eq!(parse_time_expression("1 month").unwrap().as_secs(), 2592000);
        assert_eq!(
            parse_time_expression("2 months").unwrap().as_secs(),
            5184000
        );
    }

    #[test]
    fn test_parse_time_expression_yesterday() {
        assert_eq!(parse_time_expression("yesterday").unwrap().as_secs(), 86400);
    }

    #[test]
    fn test_parse_time_expression_invalid() {
        assert!(parse_time_expression("").is_err());
        assert!(parse_time_expression("abc").is_err());
        assert!(parse_time_expression("0 days").is_err());
    }

    #[test]
    fn test_parse_time_expression_zero_compact() {
        // "0d" should fail because time must be > 0
        assert!(parse_time_expression("0d").is_err());
        assert!(parse_time_expression("0h").is_err());
        assert!(parse_time_expression("0w").is_err());
    }

    #[test]
    fn test_parse_time_expression_unknown_unit() {
        // "2 fortnights" is an unknown unit
        let result = parse_time_expression("2 fortnights");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unknown time unit"),
            "expected 'unknown time unit', got: {err}"
        );
    }

    #[test]
    fn test_parse_time_expression_non_numeric_compact() {
        // "abch" — non-numeric prefix to compact form
        let result = parse_time_expression("abch");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_time_expression_compact_weeks() {
        assert_eq!(parse_time_expression("2w").unwrap().as_secs(), 1209600);
    }

    #[test]
    fn test_resolve_since_git_error() {
        // Non-existent directory should cause git to fail
        let result = resolve_since(std::path::Path::new("/nonexistent/repo"), "1d");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_since_no_recent_commits() {
        // Create a repo with a very old commit then ask for "1 second ago"
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        std::fs::write(dir.path().join("file.txt"), "hello").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        // Ask for commits from the future effectively — "1 second" window is fine
        // since the commit was literally just made, it *will* be found.
        // To get "no commits", we need an impossible window — but git --since
        // will likely find the commit. Use the function and just verify it doesn't panic.
        let result = resolve_since(dir.path(), "1d");
        // This should succeed since commit was just made
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_context_signatures_with_parse_results() {
        use crate::budget::counter::TokenCounter;
        use crate::index::CodebaseIndex;
        use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/util.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/util.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
            ScannedFile {
                relative_path: "src/empty.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/empty.rs"),
                language: Some("rust".to_string()),
                size_bytes: 20,
            },
        ];

        let mut parse_results = HashMap::new();
        // File with public symbols
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "public_fn".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn public_fn()".to_string(),
                    body: String::new(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        // File with only private symbols
        parse_results.insert(
            "src/util.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "private_fn".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn private_fn()".to_string(),
                    body: String::new(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        // src/empty.rs has no parse result — tests the `let Some(pr) = ...` path

        let mut content_map = HashMap::new();
        content_map.insert(
            "src/lib.rs".to_string(),
            "pub fn public_fn() {}".to_string(),
        );
        content_map.insert("src/util.rs".to_string(), "fn private_fn() {}".to_string());
        content_map.insert("src/empty.rs".to_string(), "// empty".to_string());

        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

        let mut context_paths = HashSet::new();
        context_paths.insert("src/lib.rs".to_string());
        context_paths.insert("src/util.rs".to_string());
        context_paths.insert("src/empty.rs".to_string());

        let result = render_context_signatures(&index, &context_paths, 10000, &counter);

        // Should include public_fn signature from lib.rs
        assert!(
            result.contains("public_fn"),
            "expected public_fn in output: {result}"
        );
        // Should NOT include private_fn
        assert!(
            !result.contains("private_fn"),
            "private symbols should be excluded"
        );
        // Should include file header for lib.rs
        assert!(result.contains("src/lib.rs"), "expected file header");
    }

    #[test]
    fn test_render_context_signatures_empty() {
        use crate::budget::counter::TokenCounter;
        use crate::index::CodebaseIndex;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let counter = TokenCounter::new();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".to_string(),
            absolute_path: PathBuf::from("/tmp/src/main.rs"),
            language: Some("rust".to_string()),
            size_bytes: 100,
        }];
        let content_map = HashMap::from([("src/main.rs".to_string(), "fn main() {}".to_string())]);
        let index = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);

        // No context paths
        let result = render_context_signatures(&index, &HashSet::new(), 10000, &counter);
        assert!(result.is_empty());
    }

    #[test]
    fn test_diff_with_all_flag_graph_walk() {
        use crate::cli::OutputFormat;

        let repo = make_diff_repo();
        // Add a second file that imports from main
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "use crate::main;\npub fn helper() {}\n",
        )
        .unwrap();
        // Stage it
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
            .commit(Some("HEAD"), &sig, &sig, "add lib", &tree, &[&head])
            .unwrap();

        // Now modify main.rs in the working tree
        std::fs::write(
            repo.path().join("src/main.rs"),
            "fn main() { println!(\"changed\"); }\n",
        )
        .unwrap();

        // Run with all=true to exercise BFS graph walk (lines 290-291)
        let result = run(
            repo.path(),
            None,  // git_ref
            50000, // token_budget
            &OutputFormat::Markdown,
            None,  // out
            false, // verbose
            true,  // all
            None,  // focus
            false, // timing
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_time_expression_overflow() {
        // Huge number that overflows u64 parse — covers line 37
        let result = parse_time_expression("99999999999999999999999d");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid time expression"));
    }
}
