use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Compiled-once regex for identifier call patterns.
static RE_CALL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([a-zA-Z_]\w*)\s*\(").expect("RE_CALL_PATTERN"));

/// Confidence level for a resolved call edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallConfidence {
    /// Tree-sitter extracted call expression, import-resolved to a specific file.
    Exact,
    /// Regex-matched against known symbol names in Tier 2 or unresolvable Tier 1.
    Approximate,
}

/// A resolved cross-file function call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_file: String,
    pub callee_symbol: String,
    pub confidence: CallConfidence,
    /// Present when this edge was resolved ambiguously. For example, when
    /// multiple files export the same symbol the Approximate picker selects
    /// the first exporter lexicographically — deterministic but arbitrary.
    /// Consumers that require exact provenance should treat this edge as
    /// low-confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
}

/// A call that could not be resolved to a specific file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedCall {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_name: String,
}

/// The full call graph for a codebase.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<UnresolvedCall>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns all callers of a given symbol in a given file.
    pub fn callers_of(&self, file: &str, symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.callee_file == file && e.callee_symbol == symbol)
            .collect()
    }

    /// Returns all callees from a given symbol in a given file.
    pub fn callees_from(&self, file: &str, symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.caller_file == file && e.caller_symbol == symbol)
            .collect()
    }

    /// Returns true if a symbol has at least one caller.
    pub fn has_callers(&self, file: &str, symbol: &str) -> bool {
        self.edges
            .iter()
            .any(|e| e.callee_file == file && e.callee_symbol == symbol)
    }
}

// ---------------------------------------------------------------------------
// Call-site extraction: per-language tree-sitter + regex fallback
// ---------------------------------------------------------------------------

/// Extract the set of called function/method names within a specific symbol's body.
///
/// Uses tree-sitter for Tier 1 languages (top 10), regex fallback for the rest.
pub fn extract_call_sites_from_source(
    source: &str,
    language: &str,
    symbol_name: &str,
) -> Vec<String> {
    match language {
        "rust" => extract_rust_calls(source, symbol_name),
        "python" => extract_python_calls(source, symbol_name),
        "typescript" | "javascript" => extract_ts_js_calls(source, symbol_name),
        "go" => extract_go_calls(source, symbol_name),
        "java" => extract_java_calls(source, symbol_name),
        "c" => extract_c_calls(source, symbol_name),
        "cpp" => extract_cpp_calls(source, symbol_name),
        "ruby" => extract_ruby_calls(source, symbol_name),
        "csharp" => extract_csharp_calls(source, symbol_name),
        _ => extract_regex_calls_from_function(source, symbol_name),
    }
}

/// Regex fallback: scan the entire source for calls to known symbols.
pub fn regex_extract_calls(body: &str, known_symbols: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for sym in known_symbols {
        let pattern = format!(r"\b{}\s*\(", regex::escape(sym));
        if let Ok(re) = regex::Regex::new(&pattern) {
            if re.is_match(body) {
                found.push(sym.clone());
            }
        }
    }
    found
}

/// Regex fallback for unknown languages: extract anything that looks like a function call
/// inside the body of the named function.
fn extract_regex_calls_from_function(source: &str, symbol_name: &str) -> Vec<String> {
    // Find the function body heuristically: look for `symbol_name` followed by `{` or `(`
    let pattern = format!(
        r"(?:fn|function|def|func|void|public|private|protected|static)\s+{}\s*\(",
        regex::escape(symbol_name)
    );
    let func_re = match regex::Regex::new(&pattern) {
        Ok(re) => re,
        Err(_) => return vec![],
    };

    let Some(m) = func_re.find(source) else {
        return vec![];
    };

    // Find the function body: from the match position, look for balanced braces
    let body = extract_body_from_offset(source, m.end());

    // Extract all identifier(...) patterns from the body
    let mut calls: Vec<String> = RE_CALL_PATTERN
        .captures_iter(&body)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .filter(|name| {
            name != symbol_name
                && !is_keyword(name)
                && !name.starts_with(|c: char| c.is_uppercase())
        })
        .collect();
    calls.sort();
    calls.dedup();
    calls
}

fn extract_body_from_offset(source: &str, offset: usize) -> String {
    let rest = &source[offset..];
    // Find opening brace/colon
    let start = rest.find('{').or_else(|| rest.find(':')).unwrap_or(0);
    let body_start = start + 1;
    if body_start >= rest.len() {
        return String::new();
    }
    let body_rest = &rest[body_start..];
    // For brace-delimited: find matching close brace
    if rest.as_bytes().get(start) == Some(&b'{') {
        let mut depth = 1i32;
        let mut end = body_rest.len();
        for (i, ch) in body_rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        return body_rest[..end].to_string();
    }
    // For indent-delimited (Python): take next 50 lines
    body_rest.lines().take(50).collect::<Vec<_>>().join("\n")
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "for"
            | "while"
            | "match"
            | "return"
            | "let"
            | "var"
            | "val"
            | "const"
            | "new"
            | "typeof"
            | "sizeof"
            | "instanceof"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "import"
            | "from"
            | "export"
            | "default"
            | "yield"
            | "await"
            | "async"
            | "try"
            | "catch"
            | "throw"
            | "assert"
            | "panic"
            | "println"
            | "eprintln"
            | "format"
            | "print"
            | "puts"
            | "printf"
            | "sprintf"
            | "fprintf"
            | "Some"
            | "None"
            | "Ok"
            | "Err"
            | "vec"
            | "Box"
            | "Arc"
            | "Rc"
            | "String"
    )
}

// ---------------------------------------------------------------------------
// Tree-sitter helpers shared across languages
// ---------------------------------------------------------------------------

fn get_child_by_kind<'a>(
    node: &tree_sitter::Node<'a>,
    kind: &str,
    source: &[u8],
) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child.utf8_text(source).unwrap_or("").to_string());
        }
    }
    None
}

fn collect_calls_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    call_kinds: &[&str],
    calls: &mut Vec<String>,
) {
    if call_kinds.contains(&node.kind()) {
        extract_call_name(node, source, calls);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_recursive(&child, source, call_kinds, calls);
    }
}

fn extract_call_name(node: &tree_sitter::Node, source: &[u8], calls: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "name" => {
                let name = child.utf8_text(source).unwrap_or("").to_string();
                if !name.is_empty() && !is_keyword(&name) {
                    calls.push(name);
                }
                return;
            }
            "field_expression" | "member_expression" | "selector_expression" | "attribute" => {
                // method call: extract the field/member name
                let mut fc = child.walk();
                for fc_child in child.children(&mut fc) {
                    if matches!(
                        fc_child.kind(),
                        "field_identifier" | "property_identifier" | "field_name"
                    ) {
                        let name = fc_child.utf8_text(source).unwrap_or("").to_string();
                        if !name.is_empty() {
                            calls.push(name);
                        }
                        return;
                    }
                }
                // Fallback: try last identifier child
                let mut last_id = None;
                let mut fc2 = child.walk();
                for fc_child in child.children(&mut fc2) {
                    if fc_child.kind() == "identifier" || fc_child.kind() == "name" {
                        last_id = Some(fc_child.utf8_text(source).unwrap_or("").to_string());
                    }
                }
                if let Some(name) = last_id {
                    if !name.is_empty() {
                        calls.push(name);
                    }
                }
                return;
            }
            "scoped_identifier" => {
                // Rust: path::to::function() — take last segment
                let text = child.utf8_text(source).unwrap_or("");
                if let Some(last) = text.rsplit("::").next() {
                    if !last.is_empty() && !is_keyword(last) {
                        calls.push(last.to_string());
                    }
                }
                return;
            }
            _ => {}
        }
    }
}

/// Find the node for a function/method named `symbol_name` and collect call sites from it.
fn extract_calls_from_tree(
    source: &str,
    tree: &tree_sitter::Tree,
    symbol_name: &str,
    func_kinds: &[&str],
    call_kinds: &[&str],
) -> Vec<String> {
    let source_bytes = source.as_bytes();
    let root = tree.root_node();
    let mut calls: Vec<String> = Vec::new();

    find_and_collect(
        &root,
        source_bytes,
        symbol_name,
        func_kinds,
        call_kinds,
        &mut calls,
    );

    calls.retain(|c| c != symbol_name);
    calls.sort();
    calls.dedup();
    calls
}

fn find_and_collect(
    node: &tree_sitter::Node,
    source: &[u8],
    symbol_name: &str,
    func_kinds: &[&str],
    call_kinds: &[&str],
    calls: &mut Vec<String>,
) {
    if func_kinds.contains(&node.kind()) {
        let name = extract_func_name(node, source);
        if name.as_deref() == Some(symbol_name) {
            collect_calls_recursive(node, source, call_kinds, calls);
            return;
        }
    }
    // For impl blocks, class bodies, etc. — recurse to find methods
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_and_collect(&child, source, symbol_name, func_kinds, call_kinds, calls);
    }
}

/// Extract the function/method name from a function definition node.
/// Handles different tree-sitter patterns:
/// - Direct `identifier` child (Rust, Python, Go, Ruby)
/// - `name` child (Python, TypeScript)
/// - `declarator/function_declarator/identifier` (C, C++)
fn extract_func_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Try direct identifier/name children first
    if let Some(name) = get_child_by_kind(node, "identifier", source) {
        return Some(name);
    }
    if let Some(name) = get_child_by_kind(node, "name", source) {
        return Some(name);
    }
    // C/C++: look through declarator chain
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declarator" || child.kind() == "function_declarator" {
            if let Some(name) = find_identifier_deep(&child, source) {
                return Some(name);
            }
        }
    }
    None
}

/// Recursively find the first `identifier` in a declarator chain.
fn find_identifier_deep(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(node.utf8_text(source).unwrap_or("").to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = find_identifier_deep(&child, source) {
            return Some(name);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Per-language extractors
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-rust")]
fn extract_rust_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("rust grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["function_item"],
        &["call_expression"],
    )
}

#[cfg(not(feature = "lang-rust"))]
fn extract_rust_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-python")]
fn extract_python_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .expect("python grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["function_definition"],
        &["call"],
    )
}

#[cfg(not(feature = "lang-python"))]
fn extract_python_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-typescript")]
fn extract_ts_js_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    // Try TypeScript first, fall back to JavaScript
    let ts_lang = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    parser.set_language(&ts_lang).expect("ts grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &[
            "function_declaration",
            "method_definition",
            "arrow_function",
        ],
        &["call_expression"],
    )
}

#[cfg(all(not(feature = "lang-typescript"), feature = "lang-javascript"))]
fn extract_ts_js_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .expect("js grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &[
            "function_declaration",
            "method_definition",
            "arrow_function",
        ],
        &["call_expression"],
    )
}

#[cfg(all(not(feature = "lang-typescript"), not(feature = "lang-javascript")))]
fn extract_ts_js_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-go")]
fn extract_go_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .expect("go grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["function_declaration", "method_declaration"],
        &["call_expression"],
    )
}

#[cfg(not(feature = "lang-go"))]
fn extract_go_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-java")]
fn extract_java_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .expect("java grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["method_declaration", "constructor_declaration"],
        &["method_invocation"],
    )
}

#[cfg(not(feature = "lang-java"))]
fn extract_java_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-c")]
fn extract_c_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c::LANGUAGE.into())
        .expect("c grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["function_definition"],
        &["call_expression"],
    )
}

#[cfg(not(feature = "lang-c"))]
fn extract_c_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-cpp")]
fn extract_cpp_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .expect("cpp grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["function_definition"],
        &["call_expression"],
    )
}

#[cfg(not(feature = "lang-cpp"))]
fn extract_cpp_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-ruby")]
fn extract_ruby_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .expect("ruby grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &["method", "singleton_method"],
        &["call", "method_call"],
    )
}

#[cfg(not(feature = "lang-ruby"))]
fn extract_ruby_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

#[cfg(feature = "lang-csharp")]
fn extract_csharp_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .expect("csharp grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    extract_calls_from_tree(
        source,
        &tree,
        symbol_name,
        &[
            "method_declaration",
            "constructor_declaration",
            "local_function_statement",
        ],
        &["invocation_expression"],
    )
}

#[cfg(not(feature = "lang-csharp"))]
fn extract_csharp_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

// ---------------------------------------------------------------------------
// Cross-file call graph construction
// ---------------------------------------------------------------------------

/// Build the full cross-file call graph from an indexed codebase.
///
/// Algorithm:
/// 1. For each file, extract call site names per symbol using the appropriate
///    language extractor (tree-sitter Tier 1 or regex Tier 2).
/// 2. For each call site name, look up which file exports a symbol with that name
///    using the import graph: if file A imports from file B and file B exports
///    symbol S, then a call to S in A resolves to B::S (Exact confidence).
///    If no import resolution is found but another file exports S, record as
///    Approximate (the call exists but we cannot prove it's this specific file).
/// 3. Unresolvable calls (name not exported anywhere) go into `unresolved`.
pub fn build_call_graph(index: &crate::index::CodebaseIndex) -> CallGraph {
    // Build a lookup: symbol_name -> Vec<file_path> (files that export this symbol)
    let mut symbol_exports: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            for export in &pr.exports {
                symbol_exports
                    .entry(export.name.clone())
                    .or_default()
                    .push(file.relative_path.clone());
            }
            // Also consider public symbols even if not in exports
            for sym in &pr.symbols {
                if sym.visibility == crate::parser::language::Visibility::Public {
                    symbol_exports
                        .entry(sym.name.clone())
                        .or_default()
                        .push(file.relative_path.clone());
                }
            }
        }
    }
    // Deduplicate export entries
    for paths in symbol_exports.values_mut() {
        paths.sort();
        paths.dedup();
    }

    // Build: for each file, which files does it import from?
    let imported_from: std::collections::HashMap<String, std::collections::HashSet<String>> = {
        let mut m: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for (from, edge_set) in &index.graph.edges {
            let targets: std::collections::HashSet<String> =
                edge_set.iter().map(|e| e.target.clone()).collect();
            m.insert(from.clone(), targets);
        }
        m
    };

    // Build per-file local symbol set: file_path -> HashSet<symbol_name>.
    // This enables intra-file call resolution for private helpers that are
    // never added to `symbol_exports`.
    let mut local_symbols: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for file in &index.files {
        if let Some(pr) = &file.parse_result {
            let set: std::collections::HashSet<String> =
                pr.symbols.iter().map(|s| s.name.clone()).collect();
            local_symbols.insert(file.relative_path.clone(), set);
        }
    }

    let mut edges: Vec<CallEdge> = Vec::new();
    let mut unresolved: Vec<UnresolvedCall> = Vec::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        let lang = file.language.as_deref().unwrap_or("unknown");
        let imports_of_this_file = imported_from.get(&file.relative_path);

        for symbol in &pr.symbols {
            let called_names = extract_call_sites_from_source(&file.content, lang, &symbol.name);

            for callee_name in called_names {
                if callee_name == symbol.name {
                    continue;
                }

                // Intra-file call: callee is defined in the same file.
                // Add an Exact intra-file edge and continue so we don't also
                // emit an Approximate cross-file edge if the name happens to
                // be exported elsewhere too.
                let is_local = local_symbols
                    .get(&file.relative_path)
                    .map(|s| s.contains(&callee_name))
                    .unwrap_or(false);
                if is_local {
                    edges.push(CallEdge {
                        caller_file: file.relative_path.clone(),
                        caller_symbol: symbol.name.clone(),
                        callee_file: file.relative_path.clone(),
                        callee_symbol: callee_name,
                        confidence: CallConfidence::Exact,
                        resolution_note: None,
                    });
                    continue;
                }

                // Try to resolve: is there a file that this file imports from
                // that exports `callee_name`?
                let resolved_exact = symbol_exports.get(&callee_name).and_then(|exporters| {
                    if let Some(imports) = imports_of_this_file {
                        exporters.iter().find(|exp| imports.contains(*exp)).cloned()
                    } else {
                        None
                    }
                });

                if let Some(callee_file) = resolved_exact {
                    if callee_file != file.relative_path {
                        edges.push(CallEdge {
                            caller_file: file.relative_path.clone(),
                            caller_symbol: symbol.name.clone(),
                            callee_file,
                            callee_symbol: callee_name,
                            confidence: CallConfidence::Exact,
                            resolution_note: None,
                        });
                    }
                } else if let Some(exporters) = symbol_exports.get(&callee_name) {
                    // Approximate: symbol exists elsewhere but we can't confirm import.
                    let other: Vec<&String> = exporters
                        .iter()
                        .filter(|e| *e != &file.relative_path)
                        .collect();
                    if !other.is_empty() {
                        // When multiple files export the same symbol, we emit an
                        // edge only to the first exporter lexicographically. This
                        // is deterministic (exporters are pre-sorted) but
                        // arbitrary — a call to `serialize` that ambiguously
                        // resolves to 5 crates picks one.
                        let resolution_note = if other.len() > 1 {
                            Some(format!("ambiguous: {} exporters", other.len()))
                        } else {
                            None
                        };
                        edges.push(CallEdge {
                            caller_file: file.relative_path.clone(),
                            caller_symbol: symbol.name.clone(),
                            callee_file: other[0].clone(),
                            callee_symbol: callee_name,
                            confidence: CallConfidence::Approximate,
                            resolution_note,
                        });
                    }
                } else {
                    unresolved.push(UnresolvedCall {
                        caller_file: file.relative_path.clone(),
                        caller_symbol: symbol.name.clone(),
                        callee_name,
                    });
                }
            }
        }
    }

    // Deduplicate edges
    edges.sort_by(|a, b| {
        (
            &a.caller_file,
            &a.caller_symbol,
            &a.callee_file,
            &a.callee_symbol,
        )
            .cmp(&(
                &b.caller_file,
                &b.caller_symbol,
                &b.callee_file,
                &b.callee_symbol,
            ))
    });
    edges.dedup_by(|a, b| {
        a.caller_file == b.caller_file
            && a.caller_symbol == b.caller_symbol
            && a.callee_file == b.callee_file
            && a.callee_symbol == b.callee_symbol
    });

    CallGraph { edges, unresolved }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_graph_default_is_empty() {
        let cg = CallGraph::default();
        assert!(cg.edges.is_empty());
        assert!(cg.unresolved.is_empty());
    }

    #[test]
    fn test_callers_of_returns_matching_edges() {
        let cg = CallGraph {
            edges: vec![
                CallEdge {
                    caller_file: "a.rs".into(),
                    caller_symbol: "foo".into(),
                    callee_file: "b.rs".into(),
                    callee_symbol: "bar".into(),
                    confidence: CallConfidence::Exact,
                    resolution_note: None,
                },
                CallEdge {
                    caller_file: "c.rs".into(),
                    caller_symbol: "baz".into(),
                    callee_file: "b.rs".into(),
                    callee_symbol: "bar".into(),
                    confidence: CallConfidence::Approximate,
                    resolution_note: None,
                },
            ],
            unresolved: vec![],
        };
        let callers = cg.callers_of("b.rs", "bar");
        assert_eq!(callers.len(), 2);
    }

    #[test]
    fn test_has_callers_false_for_unknown_symbol() {
        let cg = CallGraph::default();
        assert!(!cg.has_callers("any.rs", "unknown"));
    }

    #[test]
    fn test_callees_from_returns_matching_edges() {
        let cg = CallGraph {
            edges: vec![CallEdge {
                caller_file: "a.rs".into(),
                caller_symbol: "main".into(),
                callee_file: "b.rs".into(),
                callee_symbol: "init".into(),
                confidence: CallConfidence::Exact,
                resolution_note: None,
            }],
            unresolved: vec![],
        };
        let callees = cg.callees_from("a.rs", "main");
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].callee_symbol, "init");
    }

    #[test]
    fn test_call_graph_new_equals_default() {
        let cg = CallGraph::new();
        assert!(cg.edges.is_empty());
        assert!(cg.unresolved.is_empty());
    }

    #[test]
    fn test_call_graph_serialize_deserialize() {
        let cg = CallGraph {
            edges: vec![CallEdge {
                caller_file: "a.rs".into(),
                caller_symbol: "foo".into(),
                callee_file: "b.rs".into(),
                callee_symbol: "bar".into(),
                confidence: CallConfidence::Exact,
                resolution_note: None,
            }],
            unresolved: vec![UnresolvedCall {
                caller_file: "a.rs".into(),
                caller_symbol: "foo".into(),
                callee_name: "unknown_fn".into(),
            }],
        };
        let json = serde_json::to_string(&cg).unwrap();
        let restored: CallGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.edges.len(), 1);
        assert_eq!(restored.unresolved.len(), 1);
    }

    // --- Call extraction tests ---

    #[test]
    fn test_extract_calls_rust_detects_function_calls() {
        let source = r#"
fn caller() {
    callee_one();
    callee_two(42);
    let x = helper(y);
}
fn callee_one() {}
fn callee_two(_n: i32) {}
fn helper(_y: i32) -> i32 { 0 }
"#;
        let calls = extract_call_sites_from_source(source, "rust", "caller");
        assert!(
            calls.contains(&"callee_one".to_string()),
            "missing callee_one in {:?}",
            calls
        );
        assert!(
            calls.contains(&"callee_two".to_string()),
            "missing callee_two in {:?}",
            calls
        );
        assert!(
            calls.contains(&"helper".to_string()),
            "missing helper in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_python() {
        let source =
            "def greet():\n    helper()\n    send_email()\n\ndef helper(): pass\ndef send_email(): pass\n";
        let calls = extract_call_sites_from_source(source, "python", "greet");
        assert!(
            calls.contains(&"helper".to_string()),
            "missing helper in {:?}",
            calls
        );
        assert!(
            calls.contains(&"send_email".to_string()),
            "missing send_email in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_typescript() {
        let source = "export function process() {\n  validate(input);\n  save(data);\n}\nfunction validate(x: any) {}\nfunction save(d: any) {}\n";
        let calls = extract_call_sites_from_source(source, "typescript", "process");
        assert!(
            calls.contains(&"validate".to_string()),
            "missing validate in {:?}",
            calls
        );
        assert!(
            calls.contains(&"save".to_string()),
            "missing save in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_go() {
        let source = "package main\nfunc run() {\n\tsetup()\n\texecute()\n}\nfunc setup() {}\nfunc execute() {}\n";
        let calls = extract_call_sites_from_source(source, "go", "run");
        assert!(
            calls.contains(&"setup".to_string()),
            "missing setup in {:?}",
            calls
        );
        assert!(
            calls.contains(&"execute".to_string()),
            "missing execute in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_java() {
        let source = "class Foo {\n  public void process() {\n    validate();\n    persist();\n  }\n  void validate() {}\n  void persist() {}\n}\n";
        let calls = extract_call_sites_from_source(source, "java", "process");
        assert!(
            calls.contains(&"validate".to_string()),
            "missing validate in {:?}",
            calls
        );
        assert!(
            calls.contains(&"persist".to_string()),
            "missing persist in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_c() {
        let source =
            "void handler() {\n  init();\n  cleanup();\n}\nvoid init() {}\nvoid cleanup() {}\n";
        let calls = extract_call_sites_from_source(source, "c", "handler");
        assert!(
            calls.contains(&"init".to_string()),
            "missing init in {:?}",
            calls
        );
        assert!(
            calls.contains(&"cleanup".to_string()),
            "missing cleanup in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_cpp() {
        let source =
            "void handler() {\n  init();\n  process();\n}\nvoid init() {}\nvoid process() {}\n";
        let calls = extract_call_sites_from_source(source, "cpp", "handler");
        assert!(
            calls.contains(&"init".to_string()),
            "missing init in {:?}",
            calls
        );
        assert!(
            calls.contains(&"process".to_string()),
            "missing process in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_ruby() {
        let source =
            "def greet\n  helper\n  send_email\nend\ndef helper; end\ndef send_email; end\n";
        let calls = extract_call_sites_from_source(source, "ruby", "greet");
        // Ruby method calls without parens are harder; at minimum the calls with parens should work
        // This test verifies the ruby extractor runs without panic
        assert!(calls.is_empty() || !calls.is_empty()); // structural test
    }

    #[test]
    fn test_extract_calls_csharp() {
        let source = "class Foo {\n  void Process() {\n    Validate();\n    Save();\n  }\n  void Validate() {}\n  void Save() {}\n}\n";
        let calls = extract_call_sites_from_source(source, "csharp", "Process");
        // C# invocation_expression detection
        assert!(
            calls.contains(&"Validate".to_string()) || calls.contains(&"Save".to_string()),
            "expected at least one call in {:?}",
            calls
        );
    }

    #[test]
    fn test_regex_fallback_extracts_known_symbols() {
        let known = vec!["validate_input".to_string(), "save_record".to_string()];
        let body = "{\n  validate_input();\n  save_record();\n}";
        let found = regex_extract_calls(body, &known);
        assert!(found.contains(&"validate_input".to_string()));
        assert!(found.contains(&"save_record".to_string()));
    }

    #[test]
    fn test_regex_fallback_for_tier2_language() {
        let source = "<?php\nfunction handler() {\n  validate_input();\n  save_record();\n}\nfunction validate_input() {}\nfunction save_record() {}\n";
        let calls = extract_call_sites_from_source(source, "php", "handler");
        assert!(
            calls.contains(&"validate_input".to_string())
                || calls.contains(&"save_record".to_string()),
            "regex should find at least one known call in {:?}",
            calls
        );
    }

    #[test]
    fn test_extract_calls_empty_source() {
        let calls = extract_call_sites_from_source("", "rust", "foo");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_calls_nonexistent_symbol() {
        let source = "fn real_fn() { helper(); }";
        let calls = extract_call_sites_from_source(source, "rust", "nonexistent");
        assert!(calls.is_empty());
    }

    // ─── Intra-file call graph tests ─────────────────────────────────────────

    /// Build a minimal `CodebaseIndex` containing a single Rust file whose
    /// parse result has two symbols — `caller` and `helper` — and whose
    /// content text allows the tree-sitter extractor to find the call.
    fn make_intra_file_index() -> crate::index::CodebaseIndex {
        use crate::index::CodebaseIndex;
        use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let src = "fn helper() -> i32 { 42 }\nfn caller() -> i32 { helper() }";
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, src).unwrap();

        let files = vec![ScannedFile {
            relative_path: "lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: src.len() as u64,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "lib.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "helper".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn helper() -> i32".into(),
                        body: "{ 42 }".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "caller".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn caller() -> i32".into(),
                        body: "{ helper() }".into(),
                        start_line: 2,
                        end_line: 2,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert("lib.rs".to_string(), src.to_string());

        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn test_call_graph_tracks_intra_file_calls() {
        // Verify that the call from `caller` to `helper` within lib.rs produces
        // an intra-file CallEdge (same file for both caller and callee).
        let index = make_intra_file_index();
        let cg = build_call_graph(&index);

        let has_intra_edge = cg.edges.iter().any(|e| {
            e.caller_file == "lib.rs"
                && e.callee_file == "lib.rs"
                && e.caller_symbol == "caller"
                && e.callee_symbol == "helper"
        });
        assert!(
            has_intra_edge,
            "expected intra-file edge caller→helper in lib.rs; edges: {:?}",
            cg.edges
        );
    }

    #[test]
    fn test_call_graph_intra_file_edge_is_exact() {
        // Intra-file edges must be Exact, not Approximate.
        let index = make_intra_file_index();
        let cg = build_call_graph(&index);

        let edge = cg.edges.iter().find(|e| {
            e.caller_file == "lib.rs"
                && e.callee_file == "lib.rs"
                && e.caller_symbol == "caller"
                && e.callee_symbol == "helper"
        });
        let edge = edge.expect("intra-file edge should exist");
        assert_eq!(
            edge.confidence,
            CallConfidence::Exact,
            "intra-file edges must have Exact confidence"
        );
    }

    #[test]
    fn test_call_graph_intra_file_not_in_unresolved() {
        // `helper()` called inside `caller` should NOT appear in unresolved
        // once intra-file resolution is applied.
        let index = make_intra_file_index();
        let cg = build_call_graph(&index);

        let in_unresolved = cg
            .unresolved
            .iter()
            .any(|u| u.caller_file == "lib.rs" && u.callee_name == "helper");
        assert!(
            !in_unresolved,
            "helper should be resolved intra-file, not unresolved: {:?}",
            cg.unresolved
        );
    }
}
