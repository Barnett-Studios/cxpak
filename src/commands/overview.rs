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

struct SectionContent {
    budgeted: String,
    full: String,
    was_truncated: bool,
}

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
    let pack_mode = index.total_tokens > token_budget;

    let alloc = BudgetAllocation::allocate(token_budget);

    let metadata = render_metadata(&index, token_budget, pack_mode);
    let directory_tree = render_directory_tree(&index, alloc.directory_tree, &counter, pack_mode);
    let module_map = render_module_map(&index, alloc.module_map, &counter, pack_mode);
    let dependency_graph =
        render_dependency_graph(&index, alloc.dependency_graph, &counter, pack_mode);
    let key_files = render_key_files(&index, alloc.key_files, &counter, pack_mode);
    let signatures = render_signatures(&index, alloc.signatures, &counter, pack_mode);
    let git_context = render_git_context(path, alloc.git_context, &counter, pack_mode);

    let sections = OutputSections {
        metadata,
        directory_tree: directory_tree.budgeted.clone(),
        module_map: module_map.budgeted.clone(),
        dependency_graph: dependency_graph.budgeted.clone(),
        key_files: key_files.budgeted.clone(),
        signatures: signatures.budgeted.clone(),
        git_context: git_context.budgeted.clone(),
    };

    // 4b. Write detail files in pack mode
    if pack_mode {
        let cxpak_dir = path.join(".cxpak");
        std::fs::create_dir_all(&cxpak_dir)?;

        let detail_sections: &[(&str, &SectionContent, &str)] = &[
            ("Directory Tree", &directory_tree, "tree.md"),
            ("Module / Component Map", &module_map, "modules.md"),
            ("Dependency Graph", &dependency_graph, "dependencies.md"),
            ("Key Files", &key_files, "key-files.md"),
            ("Function / Type Signatures", &signatures, "signatures.md"),
            ("Git Context", &git_context, "git.md"),
        ];

        for (title, section, filename) in detail_sections {
            if section.was_truncated {
                let rendered_detail = output::render_single_section(title, &section.full, format);
                let detail_path = cxpak_dir.join(filename);
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
) -> SectionContent {
    let mut full = String::new();
    for file in &index.files {
        full.push_str(&file.relative_path);
        full.push('\n');
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(&full, budget, counter, "directory tree", "tree.md")
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
        degrader::truncate_to_budget_with_pointer(&full, budget, counter, "module map", "modules.md")
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
    budget: usize,
    counter: &TokenCounter,
    pack_mode: bool,
) -> SectionContent {
    let mut full = String::new();

    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            if pr.imports.is_empty() {
                continue;
            }
            full.push_str(&format!("**{}** imports:\n", file.relative_path));
            for imp in &pr.imports {
                if imp.names.is_empty() {
                    full.push_str(&format!("- `{}`\n", imp.source));
                } else {
                    full.push_str(&format!(
                        "- `{}` — {}\n",
                        imp.source,
                        imp.names.join(", ")
                    ));
                }
            }
            full.push('\n');
        }
    }

    let (budgeted, _, omitted) = if pack_mode {
        degrader::truncate_to_budget_with_pointer(
            &full,
            budget,
            counter,
            "dependency graph",
            "dependencies.md",
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

    for file in &key_files {
        let header = format!("### {}\n\n```\n", file.relative_path);
        let footer = "\n```\n\n";
        let header_tokens = counter.count(&header) + counter.count(footer);

        if remaining <= header_tokens {
            if pack_mode {
                budgeted_out.push_str(&degrader::omission_pointer(
                    &format!("key file: {}", file.relative_path),
                    "key-files.md",
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
        let (content, used, _omitted) = if pack_mode {
            degrader::truncate_to_budget_with_pointer(
                &file.content,
                content_budget,
                counter,
                &format!("key file: {}", file.relative_path),
                "key-files.md",
            )
        } else {
            degrader::truncate_to_budget(
                &file.content,
                content_budget,
                counter,
                &format!("key file: {}", file.relative_path),
            )
        };

        budgeted_out.push_str(&header);
        budgeted_out.push_str(&content);
        budgeted_out.push_str(footer);

        remaining = remaining.saturating_sub(used + header_tokens);
    }

    let was_truncated = counter.count(&budgeted_out) < counter.count(&full);

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
            "signatures.md",
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
        degrader::truncate_to_budget_with_pointer(&full, budget, counter, "git context", "git.md")
    } else {
        degrader::truncate_to_budget(&full, budget, counter, "git context")
    };

    SectionContent {
        was_truncated: omitted > 0,
        budgeted,
        full,
    }
}
