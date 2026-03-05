use crate::budget::counter::TokenCounter;
use crate::budget::degrader;
use crate::budget::BudgetAllocation;
use crate::cli::OutputFormat;
use crate::git;
use crate::index::CodebaseIndex;
use crate::output::{self, OutputSections};
use crate::parser::LanguageRegistry;
use crate::scanner::Scanner;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub fn run(
    path: &Path,
    token_budget: usize,
    format: &OutputFormat,
    out: Option<&Path>,
    verbose: bool,
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

    // 2. Parse
    if verbose {
        eprintln!("cxpak: parsing with tree-sitter");
    }
    let registry = LanguageRegistry::new();
    let mut parse_results = HashMap::new();

    for file in &files {
        if let Some(lang_name) = &file.language {
            if let Some(lang) = registry.get(lang_name) {
                let source = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
                let mut parser = tree_sitter::Parser::new();
                if parser.set_language(&lang.ts_language()).is_ok() {
                    if let Some(tree) = parser.parse(&source, None) {
                        let result = lang.extract(&source, &tree);
                        parse_results.insert(file.relative_path.clone(), result);
                    }
                }
            }
        }
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

    if token_budget < index.total_tokens / 10 {
        eprintln!(
            "cxpak: warning: repo estimated at ~{}k tokens, budget is {}k. Output will be heavily truncated.",
            index.total_tokens / 1000,
            token_budget / 1000
        );
    }

    // 4. Budget + render sections
    let alloc = BudgetAllocation::allocate(token_budget);

    let metadata = render_metadata(&index);
    let directory_tree = render_directory_tree(&index, alloc.directory_tree, &counter);
    let module_map = render_module_map(&index, alloc.module_map, &counter);
    let dependency_graph = render_dependency_graph(&index, alloc.dependency_graph, &counter);
    let key_files = render_key_files(&index, alloc.key_files, &counter);
    let signatures = render_signatures(&index, alloc.signatures, &counter);
    let git_context = render_git_context(path, alloc.git_context, &counter);

    let sections = OutputSections {
        metadata,
        directory_tree,
        module_map,
        dependency_graph,
        key_files,
        signatures,
        git_context,
    };

    // 5. Render to format
    let rendered = output::render(&sections, format);

    // 6. Output
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

fn render_metadata(index: &CodebaseIndex) -> String {
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
) -> String {
    let mut tree = String::new();
    for file in &index.files {
        tree.push_str(&file.relative_path);
        tree.push('\n');
    }

    let (result, _, _) = degrader::truncate_to_budget(&tree, budget, counter, "directory tree");
    result
}

fn render_module_map(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut out = String::new();

    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            if pr.symbols.is_empty() {
                continue;
            }
            out.push_str(&format!("### {}\n", file.relative_path));
            for sym in &pr.symbols {
                let vis = match sym.visibility {
                    crate::parser::language::Visibility::Public => "pub ",
                    crate::parser::language::Visibility::Private => "",
                };
                out.push_str(&format!(
                    "- {}{:?}: `{}`\n",
                    vis, sym.kind, sym.name
                ));
            }
            out.push('\n');
        }
    }

    let (result, _, _) = degrader::truncate_to_budget(&out, budget, counter, "module map");
    result
}

fn render_dependency_graph(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut out = String::new();

    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            if pr.imports.is_empty() {
                continue;
            }
            out.push_str(&format!("**{}** imports:\n", file.relative_path));
            for imp in &pr.imports {
                if imp.names.is_empty() {
                    out.push_str(&format!("- `{}`\n", imp.source));
                } else {
                    out.push_str(&format!(
                        "- `{}` — {}\n",
                        imp.source,
                        imp.names.join(", ")
                    ));
                }
            }
            out.push('\n');
        }
    }

    let (result, _, _) =
        degrader::truncate_to_budget(&out, budget, counter, "dependency graph");
    result
}

fn render_key_files(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut out = String::new();
    let mut remaining = budget;

    let key_files: Vec<_> = index
        .files
        .iter()
        .filter(|f| CodebaseIndex::is_key_file(&f.relative_path))
        .collect();

    for file in key_files {
        let header = format!("### {}\n\n```\n", file.relative_path);
        let footer = "\n```\n\n";
        let header_tokens = counter.count(&header) + counter.count(footer);

        if remaining <= header_tokens {
            out.push_str(&degrader::omission_marker(
                &format!("key file: {}", file.relative_path),
                file.token_count,
                budget + file.token_count,
            ));
            out.push('\n');
            continue;
        }

        let content_budget = remaining - header_tokens;
        let (content, used, _omitted) = degrader::truncate_to_budget(
            &file.content,
            content_budget,
            counter,
            &format!("key file: {}", file.relative_path),
        );

        out.push_str(&header);
        out.push_str(&content);
        out.push_str(footer);

        remaining = remaining.saturating_sub(used + header_tokens);
    }

    out
}

fn render_signatures(
    index: &CodebaseIndex,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let mut out = String::new();

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

            out.push_str(&format!("### {}\n\n", file.relative_path));
            for sym in public_syms {
                out.push_str(&format!("```\n{}\n```\n\n", sym.signature));
            }
        }
    }

    let (result, _, _) = degrader::truncate_to_budget(&out, budget, counter, "signatures");
    result
}

fn render_git_context(
    path: &Path,
    budget: usize,
    counter: &TokenCounter,
) -> String {
    let ctx = match git::extract_git_context(path, 20) {
        Ok(ctx) => ctx,
        Err(_) => return String::new(),
    };

    let mut out = String::new();

    out.push_str("### Recent Commits\n\n");
    for commit in &ctx.commits {
        out.push_str(&format!(
            "- `{}` {} — {} ({})\n",
            commit.hash, commit.message, commit.author, commit.date
        ));
    }

    out.push_str("\n### Most Changed Files\n\n");
    for file in &ctx.file_churn {
        out.push_str(&format!(
            "- `{}` — {} commits\n",
            file.path, file.commit_count
        ));
    }

    out.push_str("\n### Contributors\n\n");
    for contrib in &ctx.contributors {
        out.push_str(&format!(
            "- {} — {} commits\n",
            contrib.name, contrib.commit_count
        ));
    }

    let (result, _, _) = degrader::truncate_to_budget(&out, budget, counter, "git context");
    result
}
