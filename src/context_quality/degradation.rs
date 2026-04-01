// Progressive degradation for context quality

use crate::budget::counter::TokenCounter;
use crate::parser::language::{Symbol, SymbolKind};
use std::collections::HashMap;

pub const MAX_SYMBOL_TOKENS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DetailLevel {
    Full = 0,
    Trimmed = 1,
    Documented = 2,
    Signature = 3,
    Stub = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileRole {
    Selected,
    Dependency,
}

#[derive(Debug, Clone)]
pub struct DegradedSymbol {
    pub symbol: Symbol,
    pub level: DetailLevel,
    pub rendered: String,
    pub rendered_tokens: usize,
    pub chunk_index: Option<usize>,
    pub chunk_total: Option<usize>,
    pub parent_name: Option<String>,
}

/// Returns the concept priority for a symbol kind (0.0–1.0).
/// Higher values survive degradation longer.
#[allow(unreachable_patterns)] // catch-all for future SymbolKind variants
pub fn concept_priority(kind: &SymbolKind) -> f64 {
    match kind {
        SymbolKind::Function | SymbolKind::Method => 1.00,
        SymbolKind::Struct
        | SymbolKind::Class
        | SymbolKind::Enum
        | SymbolKind::Interface
        | SymbolKind::Trait
        | SymbolKind::Type
        | SymbolKind::TypeAlias => 0.86,
        SymbolKind::Message
        | SymbolKind::Service
        | SymbolKind::Query
        | SymbolKind::Mutation
        | SymbolKind::Table => 0.71,
        SymbolKind::Key
        | SymbolKind::Block
        | SymbolKind::Variable
        | SymbolKind::Target
        | SymbolKind::Rule
        | SymbolKind::Instruction
        | SymbolKind::Selector
        | SymbolKind::Mixin => 0.57,
        SymbolKind::Heading | SymbolKind::Section | SymbolKind::Element => 0.43,
        SymbolKind::Constant => 0.29,
        _ => 0.14, // Imports and any future variants
    }
}

/// Compute the concept priority for a file based on its highest-priority symbol.
pub fn file_concept_priority(symbols: &[Symbol]) -> f64 {
    symbols
        .iter()
        .map(|s| concept_priority(&s.kind))
        .fold(0.0_f64, f64::max)
}

/// Render a symbol at the given detail level.
pub fn render_symbol_at_level(symbol: &Symbol, level: DetailLevel) -> DegradedSymbol {
    let counter = TokenCounter::new();
    let rendered = match level {
        DetailLevel::Full => symbol.body.clone(),
        DetailLevel::Trimmed => render_trimmed(symbol),
        DetailLevel::Documented => render_documented(symbol),
        DetailLevel::Signature => symbol.signature.clone(),
        DetailLevel::Stub => render_stub(symbol),
    };
    let rendered_tokens = counter.count(&rendered);
    DegradedSymbol {
        symbol: symbol.clone(),
        level,
        rendered,
        rendered_tokens,
        chunk_index: None,
        chunk_total: None,
        parent_name: None,
    }
}

fn render_trimmed(symbol: &Symbol) -> String {
    let lines: Vec<&str> = symbol.body.lines().collect();
    if lines.len() <= 21 {
        // Signature line + 20 body lines — no truncation needed
        return symbol.body.clone();
    }
    // First 21 lines (signature + 20 body lines)
    let kept: Vec<&str> = lines[..21].to_vec();
    let omitted = lines.len() - 21;
    format!("{}\n    // ... {} more lines", kept.join("\n"), omitted)
}

fn render_documented(symbol: &Symbol) -> String {
    let mut doc_lines = Vec::new();
    for line in symbol.body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("/**")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("*/")
            || trimmed.starts_with("# ") // hash-space only (avoids Rust #[derive] etc.)
            || trimmed.starts_with("--")
            || trimmed.starts_with("\"\"\"")
        {
            doc_lines.push(line);
        } else if !doc_lines.is_empty() {
            break; // doc comment ended
        }
    }
    if doc_lines.is_empty() {
        symbol.signature.clone()
    } else {
        format!("{}\n{}", doc_lines.join("\n"), symbol.signature)
    }
}

fn render_stub(symbol: &Symbol) -> String {
    let line_count = symbol.body.lines().count();
    format!("{} // +{} lines", symbol.signature, line_count)
}

/// Split an oversized symbol into chunks that each fit within MAX_SYMBOL_TOKENS.
/// Returns a single-element vec if the symbol is already within the limit.
pub fn split_oversized_symbol(symbol: &Symbol, _source: &str) -> Vec<DegradedSymbol> {
    let counter = TokenCounter::new();
    let total_tokens = counter.count(&symbol.body);

    if total_tokens <= MAX_SYMBOL_TOKENS {
        return vec![DegradedSymbol {
            symbol: symbol.clone(),
            level: DetailLevel::Full,
            rendered: symbol.body.clone(),
            rendered_tokens: total_tokens,
            chunk_index: None,
            chunk_total: None,
            parent_name: None,
        }];
    }

    // Line-based splitting with blank-line preference
    let lines: Vec<&str> = symbol.body.lines().collect();
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    let mut current_chunk: Vec<&str> = Vec::new();
    let mut current_tokens = 0usize;

    for line in &lines {
        let line_tokens = counter.count(line) + 1; // +1 for newline
        if current_tokens + line_tokens > MAX_SYMBOL_TOKENS && !current_chunk.is_empty() {
            chunks.push(current_chunk);
            current_chunk = Vec::new();
            current_tokens = 0;
        }
        current_chunk.push(line);
        current_tokens += line_tokens;
    }
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    let total_chunks = chunks.len();
    let mut result = Vec::new();
    let mut line_offset = 0usize;

    for (i, chunk_lines) in chunks.iter().enumerate() {
        let chunk_content = if i == 0 {
            chunk_lines.join("\n")
        } else {
            format!(
                "{} {{ // chunk {}/{}\n{}",
                symbol.signature,
                i + 1,
                total_chunks,
                chunk_lines.join("\n")
            )
        };

        let chunk_line_count = chunk_lines.len();
        let start_line = symbol.start_line + line_offset;
        let end_line = if i == total_chunks - 1 {
            symbol.end_line
        } else {
            start_line + chunk_line_count - 1
        };

        let rendered_tokens = counter.count(&chunk_content);

        result.push(DegradedSymbol {
            symbol: Symbol {
                name: format!("{} [{}/{}]", symbol.name, i + 1, total_chunks),
                kind: symbol.kind.clone(),
                visibility: symbol.visibility.clone(),
                signature: symbol.signature.clone(),
                body: chunk_content.clone(),
                start_line,
                end_line,
            },
            level: DetailLevel::Full,
            rendered: chunk_content,
            rendered_tokens,
            chunk_index: Some(i),
            chunk_total: Some(total_chunks),
            parent_name: Some(symbol.name.clone()),
        });

        line_offset += chunk_line_count;
    }

    result
}

/// Result of budget allocation for a single file.
pub struct AllocatedFile {
    pub path: String,
    pub level: DetailLevel,
    pub symbols: Vec<DegradedSymbol>,
}

/// Allocate a token budget across files using progressive degradation.
///
/// Each entry in `files` is `(&IndexedFile, FileRole, relevance_score)`.
/// Returns an allocation per file: path, chosen detail level, and rendered symbols.
///
/// When `pagerank` is `Some`, priority is computed as:
///   `priority = score * 0.6 + cp * 0.2 + pr * 0.2`
///
/// When `pagerank` is `None` (backwards compat):
///   `priority = score * 0.7 + cp * 0.3`
pub fn allocate_with_degradation(
    files: &[(&crate::index::IndexedFile, FileRole, f64)],
    budget: usize,
    pagerank: Option<&HashMap<String, f64>>,
) -> Vec<AllocatedFile> {
    if files.is_empty() {
        return vec![];
    }

    let n = files.len();

    // Fast path: check if raw token counts fit the budget
    let raw_total: usize = files.iter().map(|(f, _, _)| f.token_count).sum();
    if raw_total <= budget {
        return files
            .iter()
            .map(|(f, _role, _score)| {
                let symbols = f
                    .parse_result
                    .as_ref()
                    .map(|pr| {
                        pr.symbols
                            .iter()
                            .map(|s| render_symbol_at_level(s, DetailLevel::Full))
                            .collect()
                    })
                    .unwrap_or_default();
                AllocatedFile {
                    path: f.relative_path.clone(),
                    level: DetailLevel::Full,
                    symbols,
                }
            })
            .collect();
    }

    // Build per-file metadata
    let mut roles: Vec<FileRole> = Vec::with_capacity(n);
    let mut priorities: Vec<f64> = Vec::with_capacity(n);
    let mut all_symbols: Vec<Vec<Symbol>> = Vec::with_capacity(n);
    let mut current_levels: Vec<DetailLevel> = Vec::with_capacity(n);

    for (f, role, score) in files {
        let symbols: Vec<Symbol> = f
            .parse_result
            .as_ref()
            .map(|pr| pr.symbols.clone())
            .unwrap_or_default();
        let cp = file_concept_priority(&symbols);
        let priority = match pagerank {
            Some(pr_map) => {
                let pr = pr_map.get(f.relative_path.as_str()).copied().unwrap_or(0.0);
                score * 0.6 + cp * 0.2 + pr * 0.2
            }
            None => score * 0.7 + cp * 0.3,
        };
        roles.push(*role);
        priorities.push(priority);
        all_symbols.push(symbols);
        current_levels.push(DetailLevel::Full);
    }

    // Render all at Full
    let mut rendered: Vec<Vec<DegradedSymbol>> = all_symbols
        .iter()
        .map(|syms| {
            syms.iter()
                .map(|s| render_symbol_at_level(s, DetailLevel::Full))
                .collect()
        })
        .collect();

    let compute_total = |r: &[Vec<DegradedSymbol>]| -> usize {
        r.iter()
            .map(|syms| syms.iter().map(|s| s.rendered_tokens).sum::<usize>())
            .sum()
    };

    if compute_total(&rendered) <= budget {
        return files
            .iter()
            .zip(rendered)
            .map(|((f, _, _), syms)| AllocatedFile {
                path: f.relative_path.clone(),
                level: DetailLevel::Full,
                symbols: syms,
            })
            .collect();
    }

    let levels = [
        DetailLevel::Trimmed,
        DetailLevel::Documented,
        DetailLevel::Signature,
        DetailLevel::Stub,
    ];

    // Phase 1: Degrade dependencies (lowest priority first)
    let mut dep_indices: Vec<usize> = (0..n)
        .filter(|&i| roles[i] == FileRole::Dependency)
        .collect();
    dep_indices.sort_by(|&a, &b| priorities[a].partial_cmp(&priorities[b]).unwrap());

    for &idx in &dep_indices {
        for &level in &levels {
            current_levels[idx] = level;
            rendered[idx] = all_symbols[idx]
                .iter()
                .map(|s| render_symbol_at_level(s, level))
                .collect();
            if compute_total(&rendered) <= budget {
                return build_allocated(files, &current_levels, rendered);
            }
        }
    }

    // Phase 2: Degrade selected files (lowest priority first, never below Documented)
    let mut sel_indices: Vec<usize> = (0..n).filter(|&i| roles[i] == FileRole::Selected).collect();
    sel_indices.sort_by(|&a, &b| priorities[a].partial_cmp(&priorities[b]).unwrap());

    for &idx in &sel_indices {
        for &level in &levels[..2] {
            // Trimmed, Documented only
            current_levels[idx] = level;
            rendered[idx] = all_symbols[idx]
                .iter()
                .map(|s| render_symbol_at_level(s, level))
                .collect();
            if compute_total(&rendered) <= budget {
                return build_allocated(files, &current_levels, rendered);
            }
        }
    }

    // Phase 3: Drop dependencies entirely (lowest priority first)
    for &idx in &dep_indices {
        rendered[idx] = vec![];
        current_levels[idx] = DetailLevel::Stub;
        if compute_total(&rendered) <= budget {
            return build_allocated(files, &current_levels, rendered);
        }
    }

    // If still over budget, return what we have at minimum levels
    build_allocated(files, &current_levels, rendered)
}

fn build_allocated(
    files: &[(&crate::index::IndexedFile, FileRole, f64)],
    levels: &[DetailLevel],
    rendered: Vec<Vec<DegradedSymbol>>,
) -> Vec<AllocatedFile> {
    files
        .iter()
        .zip(levels.iter())
        .zip(rendered)
        .map(|(((f, _, _), &level), syms)| AllocatedFile {
            path: f.relative_path.clone(),
            level,
            symbols: syms,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexedFile;
    use crate::parser::language::{ParseResult, Visibility};

    fn make_fn_symbol(name: &str, tokens: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: format!("pub fn {}()", name),
            body: "x ".repeat(tokens),
            start_line: 1,
            end_line: tokens / 10,
        }
    }

    #[test]
    fn test_detail_level_ordering() {
        assert!(DetailLevel::Full < DetailLevel::Trimmed);
        assert!(DetailLevel::Trimmed < DetailLevel::Documented);
        assert!(DetailLevel::Documented < DetailLevel::Signature);
        assert!(DetailLevel::Signature < DetailLevel::Stub);
    }

    #[test]
    fn test_detail_level_equality() {
        assert_eq!(DetailLevel::Full, DetailLevel::Full);
        assert_ne!(DetailLevel::Full, DetailLevel::Stub);
    }

    #[test]
    fn test_file_role_variants() {
        let selected = FileRole::Selected;
        let dep = FileRole::Dependency;
        assert_ne!(selected, dep);
    }

    // --- concept_priority tests ---

    #[test]
    fn test_concept_priority_definitions() {
        assert_eq!(concept_priority(&SymbolKind::Function), 1.00);
        assert_eq!(concept_priority(&SymbolKind::Method), 1.00);
    }

    #[test]
    fn test_concept_priority_structures() {
        assert_eq!(concept_priority(&SymbolKind::Struct), 0.86);
        assert_eq!(concept_priority(&SymbolKind::Class), 0.86);
        assert_eq!(concept_priority(&SymbolKind::Enum), 0.86);
        assert_eq!(concept_priority(&SymbolKind::Interface), 0.86);
        assert_eq!(concept_priority(&SymbolKind::Trait), 0.86);
        assert_eq!(concept_priority(&SymbolKind::Type), 0.86);
        assert_eq!(concept_priority(&SymbolKind::TypeAlias), 0.86);
    }

    #[test]
    fn test_concept_priority_api_surface() {
        assert_eq!(concept_priority(&SymbolKind::Message), 0.71);
        assert_eq!(concept_priority(&SymbolKind::Service), 0.71);
        assert_eq!(concept_priority(&SymbolKind::Query), 0.71);
        assert_eq!(concept_priority(&SymbolKind::Mutation), 0.71);
        assert_eq!(concept_priority(&SymbolKind::Table), 0.71);
    }

    #[test]
    fn test_concept_priority_configuration() {
        assert_eq!(concept_priority(&SymbolKind::Key), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Block), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Variable), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Target), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Rule), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Instruction), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Selector), 0.57);
        assert_eq!(concept_priority(&SymbolKind::Mixin), 0.57);
    }

    #[test]
    fn test_concept_priority_documentation() {
        assert_eq!(concept_priority(&SymbolKind::Heading), 0.43);
        assert_eq!(concept_priority(&SymbolKind::Section), 0.43);
        assert_eq!(concept_priority(&SymbolKind::Element), 0.43);
    }

    #[test]
    fn test_concept_priority_constants() {
        assert_eq!(concept_priority(&SymbolKind::Constant), 0.29);
    }

    #[test]
    fn test_concept_priority_ordering_is_monotonic() {
        assert!(concept_priority(&SymbolKind::Function) > concept_priority(&SymbolKind::Struct));
        assert!(concept_priority(&SymbolKind::Struct) > concept_priority(&SymbolKind::Message));
        assert!(concept_priority(&SymbolKind::Message) > concept_priority(&SymbolKind::Key));
        assert!(concept_priority(&SymbolKind::Key) > concept_priority(&SymbolKind::Heading));
        assert!(concept_priority(&SymbolKind::Heading) > concept_priority(&SymbolKind::Constant));
    }

    #[test]
    fn test_file_concept_priority_max_wins() {
        let symbols = vec![
            make_fn_symbol("f", 10),
            Symbol {
                kind: SymbolKind::Constant,
                ..make_fn_symbol("c", 5)
            },
        ];
        assert_eq!(file_concept_priority(&symbols), 1.00);
    }

    #[test]
    fn test_file_concept_priority_empty() {
        assert_eq!(file_concept_priority(&[]), 0.0);
    }

    #[test]
    fn test_file_concept_priority_single_symbol() {
        let symbols = vec![Symbol {
            kind: SymbolKind::Key,
            ..make_fn_symbol("k", 5)
        }];
        assert_eq!(file_concept_priority(&symbols), 0.57);
    }

    // --- render_symbol_at_level tests ---

    fn make_test_symbol() -> Symbol {
        let body_lines: Vec<String> = (1..=25).map(|i| format!("    // line {}", i)).collect();
        let body = format!(
            "pub fn handle_request(req: &Request) -> Response {{\n{}\n}}",
            body_lines.join("\n")
        );
        Symbol {
            name: "handle_request".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "pub fn handle_request(req: &Request) -> Response".to_string(),
            body,
            start_line: 1,
            end_line: 27,
        }
    }

    #[test]
    fn test_render_full_includes_entire_body() {
        let sym = make_test_symbol();
        let result = render_symbol_at_level(&sym, DetailLevel::Full);
        assert_eq!(result.level, DetailLevel::Full);
        assert_eq!(result.rendered, sym.body);
    }

    #[test]
    fn test_render_trimmed_truncates_at_20_lines() {
        let sym = make_test_symbol();
        let result = render_symbol_at_level(&sym, DetailLevel::Trimmed);
        assert_eq!(result.level, DetailLevel::Trimmed);
        assert!(result.rendered.contains("// line 1"));
        assert!(result.rendered.contains("// line 20"));
        assert!(!result.rendered.contains("// line 21"));
        assert!(result.rendered.contains("... 6 more lines"));
    }

    #[test]
    fn test_render_trimmed_short_body_no_truncation() {
        let mut sym = make_test_symbol();
        sym.body = "pub fn short() {\n    return 1;\n}".to_string();
        let result = render_symbol_at_level(&sym, DetailLevel::Trimmed);
        assert!(!result.rendered.contains("more lines"));
    }

    #[test]
    fn test_render_documented_signature_only() {
        let sym = make_test_symbol();
        let result = render_symbol_at_level(&sym, DetailLevel::Documented);
        assert!(result.rendered.contains("pub fn handle_request"));
        assert!(!result.rendered.contains("// line 1"));
    }

    #[test]
    fn test_render_signature_one_line() {
        let sym = make_test_symbol();
        let result = render_symbol_at_level(&sym, DetailLevel::Signature);
        assert_eq!(result.rendered.trim(), sym.signature);
    }

    #[test]
    fn test_render_stub_compact() {
        let sym = make_test_symbol();
        let result = render_symbol_at_level(&sym, DetailLevel::Stub);
        assert!(result.rendered.contains("handle_request"));
        assert!(result.rendered.contains("+"));
        assert!(result.rendered.len() < 200);
    }

    #[test]
    fn test_render_documented_rust_doc_comment() {
        let mut sym = make_test_symbol();
        sym.body = "/// Handles incoming requests.\n/// Returns a Response.\npub fn handle_request(req: &Request) -> Response {\n    todo!()\n}".to_string();
        let result = render_symbol_at_level(&sym, DetailLevel::Documented);
        assert!(result.rendered.contains("Handles incoming requests"));
        assert!(result.rendered.contains(&sym.signature));
    }

    #[test]
    fn test_render_documented_python_docstring() {
        let mut sym = make_test_symbol();
        sym.body = "\"\"\"Handle incoming requests.\"\"\"\ndef handle_request(req):".to_string();
        sym.signature = "def handle_request(req):".to_string();
        let result = render_symbol_at_level(&sym, DetailLevel::Documented);
        assert!(result.rendered.contains("Handle incoming requests"));
    }

    #[test]
    fn test_render_documented_java_javadoc() {
        let mut sym = make_test_symbol();
        sym.body = "/**\n * Handles incoming requests.\n * @param req the request\n */\npublic Response handleRequest(Request req) {".to_string();
        sym.signature = "public Response handleRequest(Request req)".to_string();
        let result = render_symbol_at_level(&sym, DetailLevel::Documented);
        assert!(result.rendered.contains("Handles incoming requests"));
    }

    #[test]
    fn test_render_documented_no_doc_comment() {
        let mut sym = make_test_symbol();
        sym.body = "pub fn no_docs() {\n    todo!()\n}".to_string();
        let result = render_symbol_at_level(&sym, DetailLevel::Documented);
        assert_eq!(result.rendered, sym.signature);
    }

    #[test]
    fn test_render_documented_ruby_hash_comment() {
        let mut sym = make_test_symbol();
        sym.body = "# Handles incoming requests.\n# @param req [Request]\ndef handle_request(req)"
            .to_string();
        sym.signature = "def handle_request(req)".to_string();
        let result = render_symbol_at_level(&sym, DetailLevel::Documented);
        assert!(result.rendered.contains("Handles incoming requests"));
    }

    // --- chunk splitting tests ---

    #[test]
    fn test_split_symbol_under_limit_no_split() {
        let sym = make_test_symbol();
        let chunks = split_oversized_symbol(&sym, &sym.body);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].chunk_index.is_none());
    }

    #[test]
    fn test_split_symbol_over_limit() {
        let big_body = (0..500)
            .map(|i| format!("    let var_{i} = compute_something_{i}(arg1, arg2, arg3);"))
            .collect::<Vec<_>>()
            .join("\n");
        let sig = "pub fn huge_function()".to_string();
        let body = format!("{sig} {{\n{big_body}\n}}");
        let sym = Symbol {
            name: "huge_function".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: sig,
            body: body.clone(),
            start_line: 1,
            end_line: 502,
        };
        let chunks = split_oversized_symbol(&sym, &body);
        assert!(
            chunks.len() > 1,
            "should split into multiple chunks, got {}",
            chunks.len()
        );
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, Some(i));
            assert_eq!(chunk.chunk_total, Some(chunks.len()));
            assert_eq!(chunk.parent_name.as_deref(), Some("huge_function"));
        }
    }

    #[test]
    fn test_split_chunk_naming() {
        let big_body = (0..500)
            .map(|i| format!("    let v{i} = f{i}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = format!("pub fn big() {{\n{big_body}\n}}");
        let sym = Symbol {
            name: "big".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "pub fn big()".to_string(),
            body: body.clone(),
            start_line: 1,
            end_line: 502,
        };
        let chunks = split_oversized_symbol(&sym, &body);
        assert!(chunks[0].symbol.name.contains("[1/"));
        assert!(chunks[1].symbol.name.contains("[2/"));
    }

    #[test]
    fn test_split_preserves_signature_in_chunks() {
        let big_body = (0..500)
            .map(|i| format!("    let v{i} = f{i}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = format!("pub fn big() {{\n{big_body}\n}}");
        let sym = Symbol {
            name: "big".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "pub fn big()".to_string(),
            body: body.clone(),
            start_line: 1,
            end_line: 502,
        };
        let chunks = split_oversized_symbol(&sym, &body);
        for chunk in &chunks {
            assert!(
                chunk.symbol.body.contains("pub fn big()"),
                "each chunk should contain parent signature"
            );
        }
    }

    #[test]
    fn test_split_exactly_at_limit_no_panic() {
        let line = "let x = 1;\n";
        let count = MAX_SYMBOL_TOKENS * 4 / line.len();
        let body = format!("fn f() {{\n{}\n}}", line.repeat(count));
        let sym = Symbol {
            name: "f".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "fn f()".to_string(),
            body: body.clone(),
            start_line: 1,
            end_line: count + 2,
        };
        let chunks = split_oversized_symbol(&sym, &body);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_split_line_numbers_adjusted() {
        let big_body = (0..500)
            .map(|i| format!("    let v{i} = f{i}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = format!("pub fn big() {{\n{big_body}\n}}");
        let sym = Symbol {
            name: "big".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "pub fn big()".to_string(),
            body: body.clone(),
            start_line: 10,
            end_line: 512,
        };
        let chunks = split_oversized_symbol(&sym, &body);
        assert_eq!(chunks[0].symbol.start_line, 10);
        assert_eq!(chunks.last().unwrap().symbol.end_line, 512);
        for i in 1..chunks.len() {
            assert!(chunks[i].symbol.start_line > chunks[i - 1].symbol.start_line);
        }
    }

    // --- allocate_with_degradation tests ---

    fn make_indexed_file(path: &str, tokens: usize, symbols: Vec<Symbol>) -> IndexedFile {
        IndexedFile {
            relative_path: path.to_string(),
            language: Some("rust".to_string()),
            size_bytes: (tokens * 4) as u64,
            token_count: tokens,
            parse_result: Some(ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            }),
            content: "x ".repeat(tokens),
            mtime_secs: None,
        }
    }

    #[test]
    fn test_allocate_fits_at_level0() {
        let file = make_indexed_file("a.rs", 100, vec![make_fn_symbol("a", 100)]);
        let files = vec![(&file, FileRole::Selected, 0.8)];
        let result = allocate_with_degradation(&files, 1000, None);
        assert_eq!(result[0].level, DetailLevel::Full);
    }

    #[test]
    fn test_allocate_degrades_lowest_score() {
        let high_file = make_indexed_file("high.rs", 600, vec![make_fn_symbol("high", 600)]);
        let low_file = make_indexed_file("low.rs", 600, vec![make_fn_symbol("low", 600)]);
        let files = vec![
            (&high_file, FileRole::Selected, 0.9),
            (&low_file, FileRole::Dependency, 0.3),
        ];
        let result = allocate_with_degradation(&files, 800, None);
        let high = result.iter().find(|r| r.path == "high.rs").unwrap();
        let low = result.iter().find(|r| r.path == "low.rs").unwrap();
        assert!(
            high.level < low.level,
            "high-scored should be at better (lower) detail level"
        );
    }

    #[test]
    fn test_allocate_selected_never_below_documented() {
        let file = make_indexed_file("sel.rs", 5000, vec![make_fn_symbol("sel", 5000)]);
        let files = vec![(&file, FileRole::Selected, 0.5)];
        let result = allocate_with_degradation(&files, 100, None);
        let sel = result.iter().find(|r| r.path == "sel.rs").unwrap();
        assert!(
            sel.level <= DetailLevel::Documented,
            "selected file should not degrade below Documented, got {:?}",
            sel.level
        );
    }

    #[test]
    fn test_allocate_dependency_can_be_dropped() {
        let sel_file = make_indexed_file("sel.rs", 500, vec![make_fn_symbol("sel", 500)]);
        let dep_file = make_indexed_file("dep.rs", 500, vec![make_fn_symbol("dep", 500)]);
        let files = vec![
            (&sel_file, FileRole::Selected, 0.9),
            (&dep_file, FileRole::Dependency, 0.1),
        ];
        let result = allocate_with_degradation(&files, 300, None);
        // dep.rs may be dropped entirely (empty symbols) or at Stub
        let dep = result.iter().find(|r| r.path == "dep.rs");
        if let Some(d) = dep {
            assert!(d.symbols.is_empty() || d.level == DetailLevel::Stub);
        }
        // sel.rs should still be present
        assert!(result.iter().any(|r| r.path == "sel.rs"));
    }

    #[test]
    fn test_allocate_empty_files() {
        let files: Vec<(&IndexedFile, FileRole, f64)> = vec![];
        let result = allocate_with_degradation(&files, 1000, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_allocate_single_file_exact_budget() {
        let file = make_indexed_file("exact.rs", 1000, vec![make_fn_symbol("exact", 1000)]);
        let files = vec![(&file, FileRole::Selected, 0.8)];
        let result = allocate_with_degradation(&files, 1000, None);
        assert_eq!(result[0].level, DetailLevel::Full);
    }

    // --- pagerank-aware allocation test ---

    #[test]
    fn test_allocate_pagerank_higher_gets_higher_priority() {
        // Two files competing for a tight budget: same relevance score but
        // different PageRank scores.  The higher-PageRank file should receive
        // a better (lower) detail level than the lower-PageRank file.
        let high_pr_file = make_indexed_file("hub.rs", 600, vec![make_fn_symbol("hub_fn", 600)]);
        let low_pr_file = make_indexed_file("leaf.rs", 600, vec![make_fn_symbol("leaf_fn", 600)]);

        let files = vec![
            (&high_pr_file, FileRole::Dependency, 0.5),
            (&low_pr_file, FileRole::Dependency, 0.5),
        ];

        let mut pagerank = HashMap::new();
        pagerank.insert("hub.rs".to_string(), 1.0); // high PageRank
        pagerank.insert("leaf.rs".to_string(), 0.1); // low PageRank

        // Budget is tight enough that both cannot stay at Full detail.
        let result = allocate_with_degradation(&files, 800, Some(&pagerank));

        let hub = result.iter().find(|r| r.path == "hub.rs").unwrap();
        let leaf = result.iter().find(|r| r.path == "leaf.rs").unwrap();

        assert!(
            hub.level <= leaf.level,
            "hub.rs (high PageRank) should be at same or better detail level than leaf.rs (low PageRank): hub={:?}, leaf={:?}",
            hub.level,
            leaf.level
        );
    }
}
