use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::cache::{CacheEntry, FileCache};
use crate::cli::OutputFormat;
use crate::index::graph::DependencyGraph;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::parser::LanguageRegistry;
use crate::scanner::Scanner;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

pub fn run(
    path: &Path,
    target: &str,
    token_budget: usize,
    format: &OutputFormat,
    out: Option<&Path>,
    verbose: bool,
    all: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();

    // 1. Scan
    if verbose {
        eprintln!("cxpak: scanning {}", path.display());
    }
    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;
    if verbose {
        eprintln!("cxpak: found {} files", files.len());
    }

    if files.is_empty() {
        return Err("no source files found".into());
    }

    // 2. Parse (cache-aware)
    if verbose {
        eprintln!("cxpak: parsing with tree-sitter");
    }
    let cache_dir = path.join(".cxpak").join("cache");
    let existing_cache = FileCache::load(&cache_dir);
    let cache_map = existing_cache.as_map();

    let registry = LanguageRegistry::new();
    let mut parse_results = HashMap::new();
    let mut new_cache_entries: Vec<CacheEntry> = Vec::new();

    for file in &files {
        let mtime = file_mtime(&file.absolute_path);
        let size_bytes = file.size_bytes;

        // Check for a valid cache hit
        let cached_parse = if let Some(entry) = cache_map.get(file.relative_path.as_str()) {
            if entry.mtime == mtime && entry.size_bytes == size_bytes {
                Some((entry.parse_result.clone(), entry.token_count))
            } else {
                None
            }
        } else {
            None
        };

        let parse_result = if let Some((pr, _token_count)) = cached_parse {
            pr
        } else {
            // Cache miss — parse with tree-sitter
            let mut result = None;
            if let Some(lang_name) = &file.language {
                if let Some(lang) = registry.get(lang_name) {
                    let source = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
                    let mut parser = tree_sitter::Parser::new();
                    if parser.set_language(&lang.ts_language()).is_ok() {
                        if let Some(tree) = parser.parse(&source, None) {
                            result = Some(lang.extract(&source, &tree));
                        }
                    }
                }
            }
            result
        };

        if let Some(ref pr) = parse_result {
            parse_results.insert(file.relative_path.clone(), pr.clone());
        }

        new_cache_entries.push(CacheEntry {
            relative_path: file.relative_path.clone(),
            mtime,
            size_bytes,
            language: file.language.clone(),
            // token_count will be filled after indexing; use cached value if available
            token_count: cache_map
                .get(file.relative_path.as_str())
                .map(|e| e.token_count)
                .unwrap_or(0),
            parse_result,
        });
    }

    if verbose {
        eprintln!("cxpak: parsed {} files", parse_results.len());
    }

    // 3. Index
    let index = CodebaseIndex::build(files, parse_results, &counter);

    if verbose {
        eprintln!(
            "cxpak: indexed {} files, ~{} tokens total",
            index.total_files, index.total_tokens
        );
    }

    // 4. Save updated cache
    {
        let mut new_cache = FileCache::new();
        // Update token_count from the index now that indexing is complete
        for entry in new_cache_entries {
            let token_count = index
                .files
                .iter()
                .find(|f| f.relative_path == entry.relative_path)
                .map(|f| f.token_count)
                .unwrap_or(entry.token_count);
            new_cache.entries.push(CacheEntry {
                token_count,
                ..entry
            });
        }
        if let Err(e) = new_cache.save(&cache_dir) {
            if verbose {
                eprintln!("cxpak: warning: failed to save cache: {e}");
            }
        }
    }

    // 5. Build dependency graph
    let graph = build_dependency_graph(&index);

    // 6. Find the target: symbol name first, then content match
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
            eprintln!("cxpak: error: '{}' not found in codebase", target);
            std::process::exit(1);
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
    let relevant_paths: HashSet<String> = if all {
        // Full BFS in both directions
        let start: Vec<&str> = matched_files.iter().copied().collect();
        graph.reachable_from(&start)
    } else {
        // 1-hop: direct dependencies and dependents only
        let mut one_hop: HashSet<String> = matched_files.iter().map(|&s| s.to_string()).collect();
        for &file in &matched_files {
            if let Some(deps) = graph.dependencies(file) {
                one_hop.extend(deps.iter().cloned());
            }
            for dep in graph.dependents(file) {
                one_hop.insert(dep.to_string());
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

    // 8. Build output sections
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
        render_dependency_subgraph(&index, &relevant_paths, token_budget / 8, &counter);

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
    let rendered = output::render(&sections, format);

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

fn file_mtime(path: &std::path::Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Build a `DependencyGraph` from the index by resolving import source paths to
/// indexed file paths.  We do a best-effort match: convert the module path
/// (e.g. `crate::scanner`) to a file path (e.g. `src/scanner/mod.rs` or
/// `src/scanner.rs`) and look up whether such a file exists.
fn build_dependency_graph(index: &CodebaseIndex) -> DependencyGraph {
    let all_paths: HashSet<&str> = index
        .files
        .iter()
        .map(|f| f.relative_path.as_str())
        .collect();

    let mut graph = DependencyGraph::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };

        for import in &pr.imports {
            // Try to resolve the import source to an actual file in the index.
            // We convert path separators and try common file extensions.
            let candidate_base = import.source.replace("::", "/").replace('.', "/");
            let candidates = [
                format!("{candidate_base}.rs"),
                format!("{candidate_base}/mod.rs"),
                format!("src/{candidate_base}.rs"),
                format!("src/{candidate_base}/mod.rs"),
                format!("{candidate_base}.ts"),
                format!("{candidate_base}.js"),
                format!("{candidate_base}.py"),
                format!("{candidate_base}.go"),
                format!("{candidate_base}.java"),
            ];

            for candidate in &candidates {
                if all_paths.contains(candidate.as_str()) {
                    graph.add_edge(&file.relative_path, candidate);
                    break;
                }
            }
        }
    }

    graph
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

/// Render the dependency edges for files in the relevant subgraph.
fn render_dependency_subgraph(
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
        let relevant_imports: Vec<_> = pr
            .imports
            .iter()
            .filter(|imp| {
                // Only show imports that resolved to a relevant file
                let candidate_base = imp.source.replace("::", "/").replace('.', "/");
                let candidates = [
                    format!("{candidate_base}.rs"),
                    format!("{candidate_base}/mod.rs"),
                    format!("src/{candidate_base}.rs"),
                    format!("src/{candidate_base}/mod.rs"),
                    format!("{candidate_base}.ts"),
                    format!("{candidate_base}.js"),
                    format!("{candidate_base}.py"),
                    format!("{candidate_base}.go"),
                    format!("{candidate_base}.java"),
                ];
                candidates.iter().any(|c| relevant_paths.contains(c))
            })
            .collect();

        if relevant_imports.is_empty() {
            continue;
        }

        full.push_str(&format!("**{}** imports:\n", file.relative_path));
        for imp in relevant_imports {
            if imp.names.is_empty() {
                full.push_str(&format!("- `{}`\n", imp.source));
            } else {
                full.push_str(&format!("- `{}` — {}\n", imp.source, imp.names.join(", ")));
            }
        }
        full.push('\n');
    }

    let (budgeted, _, _) =
        degrader::truncate_to_budget(&full, budget, counter, "dependency subgraph");
    budgeted
}
