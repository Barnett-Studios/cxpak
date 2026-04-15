use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::cli::OutputFormat;
use crate::git;
use crate::index::graph::DependencyGraph;
use crate::index::ranking;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::scanner::Scanner;
use crate::schema::EdgeType;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    target: &str,
    token_budget: usize,
    format: &OutputFormat,
    out: Option<&Path>,
    verbose: bool,
    all: bool,
    focus: Option<&str>,
    timing: bool,
    workspace: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let total_start = Instant::now();

    // 1. Scan
    let scan_start = Instant::now();
    if verbose {
        eprintln!("cxpak: scanning {}", path.display());
    }
    let scanner = Scanner::new(path)?;
    let files = scanner.scan_workspace(workspace)?;
    if verbose {
        eprintln!("cxpak: found {} files", files.len());
    }
    if timing {
        eprintln!("cxpak [timing]: scan       {:.1?}", scan_start.elapsed());
    }

    if files.is_empty() {
        return Err("no source files found".into());
    }

    // 2. Parse (cache-aware)
    let parse_start = Instant::now();
    let (parse_results, content_map) =
        crate::cache::parse::parse_with_cache(&files, path, &counter, verbose);
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }

    // 3. Index
    let index_start = Instant::now();
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    index.conventions = crate::conventions::build_convention_profile(&index, path);
    index.co_changes = index.conventions.git_health.co_changes.clone();

    if verbose {
        eprintln!(
            "cxpak: indexed {} files, ~{} tokens total",
            index.total_files, index.total_tokens
        );
    }
    if timing {
        eprintln!("cxpak [timing]: index      {:.1?}", index_start.elapsed());
    }

    // 4. Refresh the cache with accurate post-index token counts.
    {
        let cache_dir = path.join(".cxpak").join("cache");
        let existing = crate::cache::FileCache::load(&cache_dir);
        let mut updated = crate::cache::FileCache::new();
        for entry in existing.entries {
            let token_count = index
                .files
                .iter()
                .find(|f| f.relative_path == entry.relative_path)
                .map(|f| f.token_count)
                .unwrap_or(entry.token_count);
            updated.entries.push(crate::cache::CacheEntry {
                token_count,
                ..entry
            });
        }
        if let Err(e) = updated.save(&cache_dir) {
            if verbose {
                eprintln!("cxpak: warning: failed to save cache: {e}");
            }
        }
    }

    // 5. Graph is cached on index
    let graph_start = Instant::now();
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
    let mut scores = ranking::rank_files(&file_paths, &index.graph, git_ctx.as_ref());
    if let Some(focus_path) = focus {
        ranking::apply_focus(&mut scores, focus_path, &index.graph);
    }

    // Build path→score map for ordering relevant files by importance
    let score_map: std::collections::HashMap<&str, f64> = scores
        .iter()
        .map(|s| (s.path.as_str(), s.composite))
        .collect();

    index.files.sort_by(|a, b| {
        let sa = score_map.get(a.relative_path.as_str()).unwrap_or(&0.0);
        let sb = score_map.get(b.relative_path.as_str()).unwrap_or(&0.0);
        sb.partial_cmp(sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    // 6. Find the target: symbol name first, then content match
    let search_start = Instant::now();
    let symbol_matches: Vec<_> = index.find_symbol(target);
    let matched_files: HashSet<&str> = if !symbol_matches.is_empty() {
        if verbose {
            eprintln!(
                "cxpak: found '{}' as a symbol in {} file(s)",
                target,
                symbol_matches
                    .iter()
                    .map(|(f, _)| *f)
                    .collect::<HashSet<_>>()
                    .len()
            );
        }
        symbol_matches.iter().map(|(f, _)| *f).collect()
    } else {
        let content_matches = index.find_content_matches(target);
        if content_matches.is_empty() {
            return Err(format!("'{}' not found in codebase", target).into());
        }
        if verbose {
            eprintln!(
                "cxpak: '{}' not found as symbol; matched as content in {} file(s)",
                target,
                content_matches.len()
            );
        }
        content_matches.into_iter().collect()
    };

    // 7. Walk dependency graph from matched files
    let graph = &index.graph;
    let relevant_paths: HashSet<String> = if all {
        // Full BFS in both directions
        let start: Vec<&str> = matched_files.iter().copied().collect();
        graph.reachable_from(&start)
    } else {
        // 1-hop: direct dependencies and dependents only
        let mut one_hop: HashSet<String> = matched_files.iter().map(|&s| s.to_string()).collect();
        for &file in &matched_files {
            if let Some(deps) = graph.dependencies(file) {
                one_hop.extend(deps.iter().map(|e| e.target.clone()));
            }
            for dep in graph.dependents(file) {
                one_hop.insert(dep.target.to_string());
            }
        }
        one_hop
    };

    if verbose {
        eprintln!(
            "cxpak: {} relevant file(s) in dependency subgraph",
            relevant_paths.len()
        );
    }
    if timing {
        eprintln!("cxpak [timing]: search     {:.1?}", search_start.elapsed());
    }

    // 8. Build output sections
    let render_start = Instant::now();
    let metadata = render_trace_metadata(target, &matched_files, relevant_paths.len());
    let source_code = render_symbol_source(
        &index,
        &symbol_matches,
        &matched_files,
        token_budget / 2,
        &counter,
    );
    let signatures =
        render_relevant_signatures(&index, &relevant_paths, token_budget / 4, &counter);
    let dep_subgraph =
        render_dependency_subgraph(&index, graph, &relevant_paths, token_budget / 8, &counter);

    let sections = OutputSections {
        metadata,
        directory_tree: String::new(),
        module_map: String::new(),
        dependency_graph: dep_subgraph,
        key_files: source_code,
        signatures,
        git_context: String::new(),
    };

    // 9. Render and output
    if timing {
        eprintln!("cxpak [timing]: render     {:.1?}", render_start.elapsed());
    }
    let rendered = output::render(&sections, format);
    if timing {
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

fn render_trace_metadata(
    target: &str,
    matched_files: &HashSet<&str>,
    relevant_count: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("- **Target:** `{}`\n", target));
    out.push_str(&format!(
        "- **Matched in:** {} file(s)\n",
        matched_files.len()
    ));
    for &f in matched_files {
        out.push_str(&format!("  - `{}`\n", f));
    }
    out.push_str(&format!(
        "- **Relevant files (dependency subgraph):** {}\n",
        relevant_count
    ));
    out
}

/// Render source code for the matched symbols, or the full content of matched
/// files when no symbol match was available.
fn render_symbol_source(
    index: &CodebaseIndex,
    symbol_matches: &[(&str, &crate::parser::language::Symbol)],
    matched_files: &HashSet<&str>,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut full = String::new();

    if !symbol_matches.is_empty() {
        // Group by file
        let mut by_file: HashMap<&str, Vec<&crate::parser::language::Symbol>> = HashMap::new();
        for &(path, sym) in symbol_matches {
            by_file.entry(path).or_default().push(sym);
        }

        let mut file_order: Vec<&str> = by_file.keys().copied().collect();
        file_order.sort_unstable();

        for file_path in file_order {
            let syms = &by_file[file_path];
            full.push_str(&format!("### {}\n\n", file_path));
            for sym in syms {
                full.push_str(&format!("```\n{}\n{}\n```\n\n", sym.signature, sym.body));
            }
        }
    } else {
        // Fall back: include full content of each matched file
        for file in &index.files {
            if matched_files.contains(file.relative_path.as_str()) {
                full.push_str(&format!("### {}\n\n```\n", file.relative_path));
                full.push_str(&file.content);
                full.push_str("\n```\n\n");
            }
        }
    }

    let (budgeted, _, _) = degrader::truncate_to_budget(&full, budget, counter, "source code");
    budgeted
}

/// Render public signatures of relevant (dependency-reachable) files, excluding
/// the files that already have their full source shown.
fn render_relevant_signatures(
    index: &CodebaseIndex,
    relevant_paths: &HashSet<String>,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut full = String::new();

    for file in &index.files {
        if !relevant_paths.contains(&file.relative_path) {
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

    let (budgeted, _, _) = degrader::truncate_to_budget(&full, budget, counter, "signatures");
    budgeted
}

fn edge_type_display(et: &EdgeType) -> String {
    match et {
        EdgeType::Import => "import".to_string(),
        EdgeType::ForeignKey => "foreign_key".to_string(),
        EdgeType::ViewReference => "view_reference".to_string(),
        EdgeType::TriggerTarget => "trigger_target".to_string(),
        EdgeType::IndexTarget => "index_target".to_string(),
        EdgeType::FunctionReference => "function_reference".to_string(),
        EdgeType::EmbeddedSql => "embedded_sql".to_string(),
        EdgeType::OrmModel => "orm_model".to_string(),
        EdgeType::MigrationSequence => "migration_sequence".to_string(),
        EdgeType::CrossLanguage(bt) => format!("cross_language:{bt:?}"),
    }
}

/// Render the dependency edges for files in the relevant subgraph.
fn render_dependency_subgraph(
    index: &CodebaseIndex,
    graph: &DependencyGraph,
    relevant_paths: &HashSet<String>,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut full = String::new();

    for file in &index.files {
        if !relevant_paths.contains(&file.relative_path) {
            continue;
        }

        // Collect typed edges from the dependency graph that point to relevant files.
        let typed_edges: Vec<_> = graph
            .dependencies(&file.relative_path)
            .map(|deps| {
                let mut v: Vec<_> = deps
                    .iter()
                    .filter(|e| relevant_paths.contains(&e.target))
                    .collect();
                v.sort_by(|a, b| a.target.cmp(&b.target));
                v
            })
            .unwrap_or_default();

        if typed_edges.is_empty() {
            continue;
        }

        full.push_str(&format!("**{}** imports:\n", file.relative_path));
        for edge in typed_edges {
            match &edge.edge_type {
                EdgeType::Import => {
                    full.push_str(&format!("- `{}`\n", edge.target));
                }
                et => {
                    full.push_str(&format!(
                        "- `{}` (via: {})\n",
                        edge.target,
                        edge_type_display(et)
                    ));
                }
            }
        }
        full.push('\n');
    }

    let (budgeted, _, _) =
        degrader::truncate_to_budget(&full, budget, counter, "dependency subgraph");
    budgeted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::graph::DependencyGraph;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use crate::schema::EdgeType;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_trace_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 80,
            },
            ScannedFile {
                relative_path: "src/util.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/util.rs"),
                language: Some("rust".to_string()),
                size_bytes: 40,
            },
        ];
        let mut parse_map = HashMap::new();
        parse_map.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "main".to_string(),
                    kind: SymbolKind::Function,
                    signature: "fn main()".to_string(),
                    body: "println!(\"hello\");".to_string(),
                    visibility: Visibility::Public,
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![Import {
                    source: "src/lib".to_string(),
                    names: vec!["run".to_string()],
                }],
                exports: vec![],
            },
        );
        parse_map.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "run".to_string(),
                    kind: SymbolKind::Function,
                    signature: "pub fn run()".to_string(),
                    body: "do_stuff();".to_string(),
                    visibility: Visibility::Public,
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![Import {
                    source: "src/util".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );
        // util.rs: no parse result (tests the `continue` path)
        let mut content_map = HashMap::new();
        content_map.insert(
            "src/main.rs".to_string(),
            "fn main() { run(); }".to_string(),
        );
        content_map.insert(
            "src/lib.rs".to_string(),
            "pub fn run() { do_stuff(); }".to_string(),
        );
        content_map.insert("src/util.rs".to_string(), "fn helper() {}".to_string());
        CodebaseIndex::build_with_content(files, parse_map, &counter, content_map)
    }

    #[test]
    fn test_render_trace_metadata() {
        let mut matched = HashSet::new();
        matched.insert("src/main.rs");
        matched.insert("src/lib.rs");
        let result = render_trace_metadata("my_func", &matched, 5);
        assert!(result.contains("**Target:** `my_func`"));
        assert!(result.contains("**Matched in:** 2 file(s)"));
        assert!(result.contains("`src/main.rs`"));
        assert!(result.contains("**Relevant files (dependency subgraph):** 5"));
    }

    #[test]
    fn test_render_symbol_source_with_symbols() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        let sym = &index.files[0].parse_result.as_ref().unwrap().symbols[0];
        let matches = vec![("src/main.rs", sym)];
        let matched_files: HashSet<&str> = ["src/main.rs"].into_iter().collect();
        let result = render_symbol_source(&index, &matches, &matched_files, 50000, &counter);
        assert!(result.contains("### src/main.rs"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_render_symbol_source_fallback_full_content() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        // No symbol matches → falls back to full file content
        let matched_files: HashSet<&str> = ["src/util.rs"].into_iter().collect();
        let result = render_symbol_source(&index, &[], &matched_files, 50000, &counter);
        assert!(result.contains("### src/util.rs"));
        assert!(result.contains("fn helper()"));
    }

    #[test]
    fn test_render_relevant_signatures() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        // Only lib.rs is in relevant_paths (has public symbols)
        let relevant: HashSet<String> = ["src/lib.rs".to_string()].into_iter().collect();
        let result = render_relevant_signatures(&index, &relevant, 50000, &counter);
        assert!(result.contains("### src/lib.rs"));
        assert!(result.contains("pub fn run()"));
    }

    #[test]
    fn test_render_relevant_signatures_no_parse_result() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        // util.rs has no parse_result → hits the `continue` at line 362
        let relevant: HashSet<String> = ["src/util.rs".to_string()].into_iter().collect();
        let result = render_relevant_signatures(&index, &relevant, 50000, &counter);
        assert!(result.is_empty());
    }

    fn make_test_graph() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        // main.rs imports lib.rs
        g.add_edge("src/main.rs", "src/lib.rs", EdgeType::Import);
        // lib.rs imports util.rs
        g.add_edge("src/lib.rs", "src/util.rs", EdgeType::Import);
        g
    }

    #[test]
    fn test_render_dependency_subgraph_with_import_edges() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        let graph = make_test_graph();
        let relevant: HashSet<String> = [
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "src/util.rs".to_string(),
        ]
        .into_iter()
        .collect();
        let result = render_dependency_subgraph(&index, &graph, &relevant, 50000, &counter);
        // main.rs → src/lib.rs (Import edge, no via: label)
        assert!(result.contains("**src/main.rs** imports:"));
        assert!(result.contains("- `src/lib.rs`"));
        assert!(!result.contains("(via:"));
        // lib.rs → src/util.rs
        assert!(result.contains("**src/lib.rs** imports:"));
        assert!(result.contains("- `src/util.rs`"));
    }

    #[test]
    fn test_render_dependency_subgraph_with_typed_edge() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/main.rs", "src/lib.rs", EdgeType::ForeignKey);
        let relevant: HashSet<String> = ["src/main.rs".to_string(), "src/lib.rs".to_string()]
            .into_iter()
            .collect();
        let result = render_dependency_subgraph(&index, &graph, &relevant, 50000, &counter);
        assert!(result.contains("(via: foreign_key)"));
    }

    #[test]
    fn test_render_dependency_subgraph_no_edges_for_file() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        let graph = DependencyGraph::new(); // empty graph
        let relevant: HashSet<String> = ["src/util.rs".to_string()].into_iter().collect();
        let result = render_dependency_subgraph(&index, &graph, &relevant, 50000, &counter);
        assert!(result.is_empty());
    }

    #[test]
    fn test_render_dependency_subgraph_no_relevant_targets() {
        let counter = TokenCounter::new();
        let index = make_trace_index();
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/main.rs", "src/lib.rs", EdgeType::Import);
        // Only main.rs in relevant, lib.rs is not → edge filtered out
        let relevant: HashSet<String> = ["src/main.rs".to_string()].into_iter().collect();
        let result = render_dependency_subgraph(&index, &graph, &relevant, 50000, &counter);
        assert!(result.is_empty());
    }

    #[test]
    fn test_edge_type_display_all_variants() {
        assert_eq!(edge_type_display(&EdgeType::Import), "import");
        assert_eq!(edge_type_display(&EdgeType::ForeignKey), "foreign_key");
        assert_eq!(
            edge_type_display(&EdgeType::ViewReference),
            "view_reference"
        );
        assert_eq!(
            edge_type_display(&EdgeType::TriggerTarget),
            "trigger_target"
        );
        assert_eq!(edge_type_display(&EdgeType::IndexTarget), "index_target");
        assert_eq!(
            edge_type_display(&EdgeType::FunctionReference),
            "function_reference"
        );
        assert_eq!(edge_type_display(&EdgeType::EmbeddedSql), "embedded_sql");
        assert_eq!(edge_type_display(&EdgeType::OrmModel), "orm_model");
        assert_eq!(
            edge_type_display(&EdgeType::MigrationSequence),
            "migration_sequence"
        );
    }

    /// run() must return Err (not call process::exit) when the target symbol is not found.
    #[test]
    fn run_returns_err_for_not_found_symbol() {
        use crate::cli::OutputFormat;
        let dir = tempfile::TempDir::new().unwrap();

        // Initialise a bare git repo so git::extract_git_context doesn't error out
        // before we even reach the symbol lookup.
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir.path())
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .ok();

        // Write a Rust file and commit it so the scanner can find files.
        std::fs::write(dir.path().join("main.rs"), "fn hello() {}").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty-message", "-m", "init"])
            .current_dir(dir.path())
            .output()
            .ok();

        let result = run(
            dir.path(),
            "completely_nonexistent_zzz_symbol_xyz",
            50_000,
            &OutputFormat::Markdown,
            None,
            false,
            false,
            None,
            false,
            None,
        );
        assert!(
            result.is_err(),
            "run() must return Err for not-found symbol, not exit(1)"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("not found in codebase"),
            "error message should mention 'not found in codebase', got: {msg}"
        );
    }
}
