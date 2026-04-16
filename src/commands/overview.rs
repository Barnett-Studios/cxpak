use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::budget::BudgetAllocation;
use crate::cache::{CacheEntry, FileCache};
use crate::cli::OutputFormat;
use crate::git;
use crate::index::graph::DependencyGraph;
use crate::index::ranking;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::scanner::Scanner;
use crate::schema::EdgeType;
use std::io::Write;
use std::path::Path;

struct SectionContent {
    budgeted: String,
    full: String,
    was_truncated: bool,
}

fn detail_file_ext(format: &OutputFormat) -> &'static str {
    match format {
        OutputFormat::Markdown => "md",
        OutputFormat::Xml => "xml",
        OutputFormat::Json => "json",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    token_budget: usize,
    format: &OutputFormat,
    out: Option<&Path>,
    verbose: bool,
    focus: Option<&str>,
    timing: bool,
    health: bool,
    workspace: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let total_start = std::time::Instant::now();

    // 1. Scan
    let scan_start = std::time::Instant::now();
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
    let parse_start = std::time::Instant::now();
    let (parse_results, content_map) =
        crate::cache::parse::parse_with_cache(&files, path, &counter, verbose);
    if timing {
        eprintln!("cxpak [timing]: parse      {:.1?}", parse_start.elapsed());
    }

    // 3. Index
    let index_start = std::time::Instant::now();
    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    index.conventions = crate::conventions::build_convention_profile(&index, path);
    index.co_changes = index.conventions.git_health.co_changes.clone();

    // 3b. Rank files by importance and sort so high-value files get budget first
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

    // Build path→score map and sort index.files by descending composite score
    let score_map: std::collections::HashMap<&str, f64> = scores
        .iter()
        .map(|s| (s.path.as_str(), s.composite))
        .collect();
    index.files.sort_by(|a, b| {
        let sa = score_map.get(a.relative_path.as_str()).unwrap_or(&0.0);
        let sb = score_map.get(b.relative_path.as_str()).unwrap_or(&0.0);
        sb.partial_cmp(sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    if verbose {
        eprintln!(
            "cxpak: indexed {} files, ~{} tokens total",
            index.total_files, index.total_tokens
        );
    }
    if timing {
        eprintln!("cxpak [timing]: index      {:.1?}", index_start.elapsed());
    }

    if token_budget < index.total_tokens / 10 {
        eprintln!(
            "cxpak: warning: repo estimated at ~{}k tokens, budget is {}k. Output will be heavily truncated.",
            index.total_tokens / 1000,
            token_budget / 1000
        );
    }

    // 4. Budget + render sections
    let render_start = std::time::Instant::now();
    let pack_mode = index.total_tokens > token_budget;

    let alloc = BudgetAllocation::allocate(token_budget);

    let ext = detail_file_ext(format);
    let metadata = render_metadata(&index, token_budget, pack_mode);
    let directory_tree = render_directory_tree(
        &index,
        alloc.directory_tree,
        &counter,
        pack_mode,
        &format!("tree.{ext}"),
    );
    let module_map = render_module_map(
        &index,
        alloc.module_map,
        &counter,
        pack_mode,
        &format!("modules.{ext}"),
    );
    let dependency_graph = render_dependency_graph(
        &index,
        &index.graph,
        alloc.dependency_graph,
        &counter,
        pack_mode,
        &format!("dependencies.{ext}"),
    );
    let key_files = render_key_files(
        &index,
        alloc.key_files,
        &counter,
        pack_mode,
        &format!("key-files.{ext}"),
    );
    let signatures = render_signatures(
        &index,
        alloc.signatures,
        &counter,
        pack_mode,
        &format!("signatures.{ext}"),
    );
    let git_context = render_git_context(
        path,
        alloc.git_context,
        &counter,
        pack_mode,
        &format!("git.{ext}"),
    );

    let sections = OutputSections {
        metadata,
        directory_tree: directory_tree.budgeted.clone(),
        module_map: module_map.budgeted.clone(),
        dependency_graph: dependency_graph.budgeted.clone(),
        key_files: key_files.budgeted.clone(),
        signatures: signatures.budgeted.clone(),
        git_context: git_context.budgeted.clone(),
    };

    // 4b. Clean stale .cxpak/ directory before any writes, preserving cache/
    let cxpak_dir = path.join(".cxpak");
    if cxpak_dir.exists() {
        for entry in std::fs::read_dir(&cxpak_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name != "cache" {
                let p = entry.path();
                if p.is_dir() {
                    std::fs::remove_dir_all(&p)?;
                } else {
                    std::fs::remove_file(&p)?;
                }
            }
        }
    }

    // Refresh the cache with accurate post-index token counts.
    {
        let cache_dir = path.join(".cxpak").join("cache");
        let existing = FileCache::load(&cache_dir);
        let mut updated = FileCache::new();
        for entry in existing.entries {
            let token_count = index
                .files
                .iter()
                .find(|f| f.relative_path == entry.relative_path)
                .map(|f| f.token_count)
                .unwrap_or(entry.token_count);
            updated.entries.push(CacheEntry {
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

    // Write detail files in pack mode
    if pack_mode {
        let cxpak_dir = path.join(".cxpak");
        std::fs::create_dir_all(&cxpak_dir)?;

        let detail_sections: &[(&str, &SectionContent, String)] = &[
            ("Directory Tree", &directory_tree, format!("tree.{ext}")),
            (
                "Module / Component Map",
                &module_map,
                format!("modules.{ext}"),
            ),
            (
                "Dependency Graph",
                &dependency_graph,
                format!("dependencies.{ext}"),
            ),
            ("Key Files", &key_files, format!("key-files.{ext}")),
            (
                "Function / Type Signatures",
                &signatures,
                format!("signatures.{ext}"),
            ),
            ("Git Context", &git_context, format!("git.{ext}")),
        ];

        for (title, section, filename) in detail_sections {
            if section.was_truncated {
                let rendered_detail = output::render_single_section(title, &section.full, format);
                let detail_path = cxpak_dir.join(filename.as_str());
                std::fs::write(&detail_path, &rendered_detail)?;
                if verbose {
                    eprintln!("cxpak: wrote {}", detail_path.display());
                }
            }
        }

        crate::util::ensure_gitignore_entry(path)?;

        if verbose {
            eprintln!("cxpak: pack mode — detail files in {}", cxpak_dir.display());
        }
    }

    if timing {
        eprintln!("cxpak [timing]: render     {:.1?}", render_start.elapsed());
    }

    // 5. Render to format
    let mut rendered = output::render(&sections, format);

    // 5b. Append health score if requested
    if health {
        let health_score = crate::intelligence::health::compute_health(&index);
        rendered.push_str("\n\n## Codebase Health\n\n");
        rendered.push_str(&format!("Composite: {:.1}/10\n", health_score.composite));
        rendered.push_str(&format!(
            "  conventions:     {:.1}/10\n",
            health_score.conventions
        ));
        rendered.push_str(&format!(
            "  test_coverage:   {:.1}/10\n",
            health_score.test_coverage
        ));
        rendered.push_str(&format!(
            "  churn_stability: {:.1}/10\n",
            health_score.churn_stability
        ));
        rendered.push_str(&format!(
            "  coupling:        {:.1}/10\n",
            health_score.coupling
        ));
        rendered.push_str(&format!(
            "  cycles:          {:.1}/10\n",
            health_score.cycles
        ));
        if let Some(dc) = health_score.dead_code {
            rendered.push_str(&format!("  dead_code:       {:.1}/10\n", dc));
        } else {
            rendered.push_str("  dead_code:       N/A (available in v1.3.0)\n");
        }
    }

    // 6. Output
    if timing {
        eprintln!("cxpak [timing]: total      {:.1?}", total_start.elapsed());
    }
    match out {
        Some(path) => {
            std::fs::write(path, &rendered)?;
            if verbose {
                eprintln!("cxpak: written to {}", path.display());
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

fn render_metadata(index: &CodebaseIndex, token_budget: usize, pack_mode: bool) -> String {
    let mut out = String::new();

    out.push_str(&format!("- **Files:** {}\n", index.total_files));
    out.push_str(&format!(
        "- **Total size:** {:.1} KB\n",
        index.total_bytes as f64 / 1024.0
    ));
    out.push_str(&format!(
        "- **Estimated tokens:** ~{}k\n",
        index.total_tokens / 1000
    ));

    if pack_mode {
        let budget_display = if token_budget >= 1000 {
            format!("{}k", token_budget / 1000)
        } else {
            format!("{}", token_budget)
        };
        out.push_str(&format!("- **Token budget:** {}\n", budget_display));
        out.push_str("- **Detail files:** `.cxpak/` (full untruncated analysis)\n");
    }

    if !index.language_stats.is_empty() {
        out.push_str("- **Languages:**\n");
        let mut langs: Vec<_> = index.language_stats.iter().collect();
        langs.sort_by(|a, b| b.1.file_count.cmp(&a.1.file_count));
        for (lang, stats) in &langs {
            let pct = if index.total_files > 0 {
                (stats.file_count as f64 / index.total_files as f64 * 100.0) as usize
            } else {
                0
            };
            out.push_str(&format!(
                "  - {} — {} files ({}%)\n",
                lang, stats.file_count, pct
            ));
        }
    }

    out
}

fn render_directory_tree(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
    detail_filename: &str,
) -> SectionContent {
    let mut full = String::new();
    for file in &index.files {
        full.push_str(&file.relative_path);
        full.push('\n');
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(
            &full,
            budget,
            counter,
            "directory tree",
            detail_filename,
        )
    } else {
        degrader::truncate_to_budget(&full, budget, counter, "directory tree")
    };

    SectionContent {
        was_truncated: omitted > 0,
        budgeted,
        full,
    }
}

fn render_module_map(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
    detail_filename: &str,
) -> SectionContent {
    let mut full = String::new();

    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            if pr.symbols.is_empty() {
                continue;
            }
            full.push_str(&format!("### {}\n", file.relative_path));
            for sym in &pr.symbols {
                let vis = match sym.visibility {
                    crate::parser::language::Visibility::Public => "pub ",
                    crate::parser::language::Visibility::Private => "",
                };
                full.push_str(&format!("- {}{:?}: `{}`\n", vis, sym.kind, sym.name));
            }
            full.push('\n');
        }
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(
            &full,
            budget,
            counter,
            "module map",
            detail_filename,
        )
    } else {
        degrader::truncate_to_budget(&full, budget, counter, "module map")
    };

    SectionContent {
        was_truncated: omitted > 0,
        budgeted,
        full,
    }
}

fn render_dependency_graph(
    index: &CodebaseIndex,
    graph: &DependencyGraph,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
    detail_filename: &str,
) -> SectionContent {
    let mut full = String::new();

    for file in &index.files {
        // Collect typed outgoing edges for this file, sorted for deterministic output.
        let edges: Vec<_> = graph
            .dependencies(&file.relative_path)
            .map(|deps| {
                let mut v: Vec<_> = deps.iter().collect();
                v.sort_by(|a, b| a.target.cmp(&b.target));
                v
            })
            .unwrap_or_default();

        if edges.is_empty() {
            continue;
        }

        full.push_str(&format!("**{}** imports:\n", file.relative_path));
        for edge in edges {
            match &edge.edge_type {
                EdgeType::Import => {
                    full.push_str(&format!("- {}\n", edge.target));
                }
                et => {
                    let label = match et {
                        EdgeType::ForeignKey => "foreign_key".to_string(),
                        EdgeType::ViewReference => "view_reference".to_string(),
                        EdgeType::TriggerTarget => "trigger_target".to_string(),
                        EdgeType::IndexTarget => "index_target".to_string(),
                        EdgeType::FunctionReference => "function_reference".to_string(),
                        EdgeType::EmbeddedSql => "embedded_sql".to_string(),
                        EdgeType::OrmModel => "orm_model".to_string(),
                        EdgeType::MigrationSequence => "migration_sequence".to_string(),
                        EdgeType::CrossLanguage(bt) => format!("cross_language:{bt:?}"),
                        EdgeType::Import => continue, // Import edges filtered upstream
                    };
                    full.push_str(&format!("- {} ({})\n", edge.target, label));
                }
            }
        }
        full.push('\n');
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(
            &full,
            budget,
            counter,
            "dependency graph",
            detail_filename,
        )
    } else {
        degrader::truncate_to_budget(&full, budget, counter, "dependency graph")
    };

    SectionContent {
        was_truncated: omitted > 0,
        budgeted,
        full,
    }
}

fn render_key_files(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
    detail_filename: &str,
) -> SectionContent {
    let key_files: Vec<_> = index
        .files
        .iter()
        .filter(|f| CodebaseIndex::is_key_file(&f.relative_path))
        .collect();

    // Generate full (unbudgeted) content
    let mut full = String::new();
    for file in &key_files {
        full.push_str(&format!("### {}\n\n```\n", file.relative_path));
        full.push_str(&file.content);
        full.push_str("\n```\n\n");
    }

    // Generate budgeted content (existing logic, but with pointer markers in pack mode)
    let mut budgeted_out = String::new();
    let mut remaining = budget;
    let mut was_truncated = false;

    for file in &key_files {
        let header = format!("### {}\n\n```\n", file.relative_path);
        let footer = "\n```\n\n";
        let header_tokens = counter.count(&header) + counter.count(footer);

        if remaining <= header_tokens {
            was_truncated = true;
            if pack_mode {
                budgeted_out.push_str(&degrader::omission_pointer(
                    &format!("key file: {}", file.relative_path),
                    detail_filename,
                    file.token_count,
                ));
            } else {
                budgeted_out.push_str(&degrader::omission_marker(
                    &format!("key file: {}", file.relative_path),
                    file.token_count,
                    budget + file.token_count,
                ));
            }
            budgeted_out.push('\n');
            continue;
        }

        let content_budget = remaining - header_tokens;
        let (content, used, omitted) = if pack_mode {
            degrader::truncate_to_budget_with_pointer(
                &file.content,
                content_budget,
                counter,
                &format!("key file: {}", file.relative_path),
                detail_filename,
            )
        } else {
            degrader::truncate_to_budget(
                &file.content,
                content_budget,
                counter,
                &format!("key file: {}", file.relative_path),
            )
        };

        if omitted > 0 {
            was_truncated = true;
        }

        budgeted_out.push_str(&header);
        budgeted_out.push_str(&content);
        budgeted_out.push_str(footer);

        remaining = remaining.saturating_sub(used + header_tokens);
    }

    SectionContent {
        budgeted: budgeted_out,
        full,
        was_truncated,
    }
}

fn render_signatures(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
    detail_filename: &str,
) -> SectionContent {
    let mut full = String::new();

    for file in &index.files {
        if let Some(pr) = &file.parse_result {
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
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(
            &full,
            budget,
            counter,
            "signatures",
            detail_filename,
        )
    } else {
        degrader::truncate_to_budget(&full, budget, counter, "signatures")
    };

    SectionContent {
        was_truncated: omitted > 0,
        budgeted,
        full,
    }
}

fn render_git_context(
    path: &Path,
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
    detail_filename: &str,
) -> SectionContent {
    let ctx = match git::extract_git_context(path, 20) {
        Ok(ctx) => ctx,
        Err(_) => {
            return SectionContent {
                budgeted: String::new(),
                full: String::new(),
                was_truncated: false,
            }
        }
    };

    let mut full = String::new();

    full.push_str("### Recent Commits\n\n");
    for commit in &ctx.commits {
        full.push_str(&format!(
            "- `{}` {} — {} ({})\n",
            commit.hash, commit.message, commit.author, commit.date
        ));
    }

    full.push_str("\n### Most Changed Files\n\n");
    for file in &ctx.file_churn {
        full.push_str(&format!(
            "- `{}` — {} commits\n",
            file.path, file.commit_count
        ));
    }

    full.push_str("\n### Contributors\n\n");
    for contrib in &ctx.contributors {
        full.push_str(&format!(
            "- {} — {} commits\n",
            contrib.name, contrib.commit_count
        ));
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(
            &full,
            budget,
            counter,
            "git context",
            detail_filename,
        )
    } else {
        degrader::truncate_to_budget(&full, budget, counter, "git context")
    };

    SectionContent {
        was_truncated: omitted > 0,
        budgeted,
        full,
    }
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

    fn make_test_index() -> (CodebaseIndex, TokenCounter) {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 200,
            },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "greet".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn greet()".to_string(),
                    body: "{ println!(\"hello\"); }".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![Import {
                    source: "std".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        content_map.insert(
            "src/lib.rs".to_string(),
            "use std;\npub fn greet() { println!(\"hello\"); }".to_string(),
        );
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        (index, counter)
    }

    #[test]
    fn test_render_metadata() {
        let (index, _) = make_test_index();
        let result = render_metadata(&index, 50000, false);
        assert!(result.contains("Files:"), "expected Files in metadata");
        assert!(
            result.contains("Languages:"),
            "expected Languages in metadata"
        );
    }

    #[test]
    fn test_render_metadata_pack_mode() {
        let (index, _) = make_test_index();
        let result = render_metadata(&index, 10, true);
        assert!(result.contains("Files:"));
    }

    #[test]
    fn test_render_directory_tree() {
        let (index, counter) = make_test_index();
        let result = render_directory_tree(&index, 10000, &counter, false, "tree.md");
        assert!(
            !result.budgeted.is_empty(),
            "expected directory tree content"
        );
        assert!(
            result.budgeted.contains("src"),
            "expected src directory in tree"
        );
    }

    #[test]
    fn test_render_module_map() {
        let (index, counter) = make_test_index();
        let result = render_module_map(&index, 10000, &counter, false, "modules.md");
        assert!(!result.full.is_empty() || result.budgeted.is_empty());
    }

    #[test]
    fn test_render_dependency_graph_with_import_edge() {
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
        ];
        let parse_results = HashMap::new();
        let content_map = HashMap::from([
            ("src/lib.rs".to_string(), "use util;".to_string()),
            ("src/util.rs".to_string(), "fn helper() {}".to_string()),
        ]);
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        let mut graph = DependencyGraph::new();
        graph.add_edge("src/lib.rs", "src/util.rs", EdgeType::Import);
        let result = render_dependency_graph(&index, &graph, 10000, &counter, false, "deps.md");
        assert!(
            result.full.contains("src/util.rs"),
            "expected import target in graph output"
        );
        // Import edges should not show the type label
        assert!(
            !result.full.contains("(import)"),
            "import edges should not show type label"
        );
    }

    #[test]
    fn test_render_dependency_graph_with_schema_edge() {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "api/orders.py".to_string(),
                absolute_path: PathBuf::from("/tmp/api/orders.py"),
                language: Some("python".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "schema/tables.sql".to_string(),
                absolute_path: PathBuf::from("/tmp/schema/tables.sql"),
                language: Some("sql".to_string()),
                size_bytes: 200,
            },
        ];
        let parse_results = HashMap::new();
        let content_map = HashMap::from([
            (
                "api/orders.py".to_string(),
                "SELECT * FROM orders".to_string(),
            ),
            (
                "schema/tables.sql".to_string(),
                "CREATE TABLE orders (id INT);".to_string(),
            ),
        ]);
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        let mut graph = DependencyGraph::new();
        graph.add_edge("api/orders.py", "schema/tables.sql", EdgeType::EmbeddedSql);
        let result = render_dependency_graph(&index, &graph, 10000, &counter, false, "deps.md");
        assert!(
            result.full.contains("embedded_sql"),
            "expected edge type label in graph output"
        );
        assert!(
            result.full.contains("schema/tables.sql"),
            "expected target in graph output"
        );
    }

    #[test]
    fn test_render_dependency_graph_empty() {
        let (index, counter) = make_test_index();
        let graph = DependencyGraph::new();
        let result = render_dependency_graph(&index, &graph, 10000, &counter, false, "deps.md");
        assert!(
            result.full.is_empty(),
            "empty graph should produce no output"
        );
    }

    #[test]
    fn test_render_git_context_non_repo() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let result = render_git_context(dir.path(), 10000, &counter, false, "git.md");
        assert!(
            result.budgeted.is_empty(),
            "non-repo should return empty git context"
        );
    }

    #[test]
    fn test_render_key_files() {
        let (index, counter) = make_test_index();
        let result = render_key_files(&index, 10000, &counter, false, "files.md");
        assert!(!result.budgeted.is_empty(), "expected key files content");
    }

    #[test]
    fn test_render_signatures() {
        let (index, counter) = make_test_index();
        let result = render_signatures(&index, 10000, &counter, false, "sigs.md");
        assert!(
            result.full.contains("greet") || result.budgeted.contains("greet"),
            "expected greet signature"
        );
    }

    #[test]
    fn test_detail_file_ext() {
        assert_eq!(detail_file_ext(&OutputFormat::Markdown), "md");
        assert_eq!(detail_file_ext(&OutputFormat::Xml), "xml");
        assert_eq!(detail_file_ext(&OutputFormat::Json), "json");
    }

    #[test]
    fn test_render_metadata_zero_total_files() {
        let counter = TokenCounter::new();
        let index =
            CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
        let result = render_metadata(&index, 50000, false);
        assert!(result.contains("Files:"));
    }

    #[test]
    fn test_render_key_files_truncated() {
        // Use a tiny budget so key file content gets truncated → covers lines 520-521
        let (index, counter) = make_test_index();
        let result = render_key_files(&index, 5, &counter, false, "files.md");
        assert!(result.was_truncated);
    }

    #[test]
    fn test_render_metadata_with_language_stats_zero_files() {
        // Build an index then set total_files to 0 to trigger line 318
        let (mut index, _) = make_test_index();
        index.total_files = 0;
        let result = render_metadata(&index, 50000, false);
        assert!(result.contains("Languages:"));
        // Should show 0% for each language
        assert!(result.contains("0%"));
    }
}
