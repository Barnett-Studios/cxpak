// Progressive degradation for context quality

use crate::budget::counter::TokenCounter;
use crate::parser::language::{Symbol, SymbolKind};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::Visibility;

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
}
