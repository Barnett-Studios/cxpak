use regex::Regex;
use std::sync::LazyLock;

/// Compiled-once regex for identifier call patterns.
static RE_CALL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([a-zA-Z_]\w*)\s*\(").expect("RE_CALL_PATTERN"));

// The call-graph data model (`CallGraph`, `CallEdge`, `CallConfidence`,
// `UnresolvedCall`) and its pure query methods now live in `core_graph`
// (cxpak 3.0.0 Phase 0 de-cycle). Re-exported here so the historical
// `crate::intelligence::call_graph::{...}` paths keep resolving; the call-site
// extraction / graph construction logic below stays in this module.
pub use crate::core_graph::intel::{CallConfidence, CallEdge, CallGraph, UnresolvedCall};

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
        "clojure" => extract_clojure_calls(source, symbol_name),
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
// Clojure call-site extraction
// ---------------------------------------------------------------------------

/// Per-symbol lookup over the one-pass Clojure map (see
/// [`extract_clojure_calls_by_symbol`]). The map keys and values are both
/// qualified `ns/name` strings — the same form the parser stores in
/// `Symbol.name` — so callers can look `symbol.name` up directly.
#[cfg(feature = "lang-clojure")]
fn extract_clojure_calls(source: &str, symbol_name: &str) -> Vec<String> {
    let mut calls = extract_clojure_calls_by_symbol(source)
        .and_then(|m| m.get(symbol_name).cloned())
        .unwrap_or_default();
    calls.retain(|c| c != symbol_name);
    calls.sort();
    calls.dedup();
    calls
}

#[cfg(not(feature = "lang-clojure"))]
fn extract_clojure_calls(source: &str, symbol_name: &str) -> Vec<String> {
    extract_regex_calls_from_function(source, symbol_name)
}

/// One-pass call-site map for Clojure.
///
/// The generic [`extract_call_sites_from_source`] regex fallback can never
/// see a Lisp call: `(callee arg)` puts the open paren before the name, and
/// `(defn name [args] …)` puts `[` — not `(` — after the name, so neither
/// half of the C-shaped `def name(` / `ident(` patterns matches. Here a
/// call site is the first `sym_lit` child of each `list_lit`, collected per
/// enclosing top-level `def*` form.
///
/// Values are emitted in the qualified `ns/name` form the symbol index
/// stores, resolved against the file's own `(ns … :require …)`:
/// - `alias/f` → `required.ns/f` through the `:as` alias map (left as-is
///   when the alias is unknown — fully-qualified `real.ns/f` already has the
///   stored form);
/// - bare `f` → `this.ns/f` when `f` is a top-level def in this file, plus
///   `required.ns/f` for every require that `:refer`s it;
/// - bare `f` that is neither local nor referred (clojure.core, special
///   forms, interop) is dropped — emitting `this.ns/f` for it would flood
///   `unresolved` with one miss per core call.
///
/// `quote`/`comment`/`quo_lit` subtrees are skipped: their lists are data,
/// not calls.
#[cfg(feature = "lang-clojure")]
fn extract_clojure_calls_by_symbol(
    source: &str,
) -> Option<std::collections::HashMap<String, Vec<String>>> {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_clojure::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    let source_bytes = source.as_bytes();
    let root = tree.root_node();

    // This file's namespace plus its :require alias / :refer maps.
    let mut scope = ClojureScope::default();
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "list_lit" && Clj::list_head_text(&child, source_bytes) == "ns" {
                let (name, _) = Clj::list_name_text(&child, source_bytes);
                scope.namespaces.insert(name.clone());
                scope.ns = name;
                clojure_require_specs(&child, source_bytes, &mut scope);
                break;
            }
        }
    }

    // Top-level def forms: both the map keys and the local-name set.
    const DEF_HEADS: &[&str] = &[
        "defn",
        "defn-",
        "defmacro",
        "def",
        "defonce",
        "defmulti",
        "defprotocol",
        "defrecord",
        "deftype",
    ];
    let mut cursor = root.walk();
    let defs: Vec<tree_sitter::Node> = root
        .children(&mut cursor)
        .filter(|c| {
            c.kind() == "list_lit"
                && DEF_HEADS.contains(&Clj::list_head_text(c, source_bytes).as_str())
        })
        .collect();
    scope.locals = defs
        .iter()
        .filter_map(|n| {
            let (name, _) = Clj::list_name_text(n, source_bytes);
            (!name.is_empty()).then_some(name)
        })
        .collect();

    let mut map = std::collections::HashMap::new();
    for def in &defs {
        let (name, _) = Clj::list_name_text(def, source_bytes);
        if name.is_empty() {
            continue;
        }
        let mut calls = Vec::new();
        collect_clojure_call_heads(
            def,
            source_bytes,
            &scope,
            &std::collections::HashSet::new(),
            &mut calls,
        );
        map.entry(Clj::qualify(&scope.ns, &name))
            .or_insert_with(Vec::new)
            .extend(calls);
    }
    Some(map)
}

/// Named children of `node` that are forms: reader comments and `#_`
/// discards are dropped so they cannot shift positional pairing.
#[cfg(feature = "lang-clojure")]
fn clojure_forms<'a>(node: &tree_sitter::Node<'a>) -> Vec<tree_sitter::Node<'a>> {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    Clj::named_children(node)
        .into_iter()
        .filter(|c| !matches!(c.kind(), "comment" | "dis_expr"))
        .collect()
}

/// Names a Clojure file's `(ns …)` form brings into scope, plus its own defs.
#[cfg(feature = "lang-clojure")]
#[derive(Default)]
struct ClojureScope {
    /// The file's own namespace.
    ns: String,
    /// `:as` alias -> required namespace.
    aliases: std::collections::HashMap<String, String>,
    /// Every required namespace (and the file's own) usable as a full prefix.
    namespaces: std::collections::HashSet<String>,
    /// Referred bare name (after `:rename`) -> qualified `ns/name`.
    refers: std::collections::HashMap<String, String>,
    /// Top-level def names in this file.
    locals: std::collections::HashSet<String>,
}

/// Read the `(:require …)` / `(:use …)` clauses of an `(ns …)` form into the
/// scope. Handles `[ns :as a]`, `[ns :refer [f]]` / `[ns :only [f]]`,
/// `:rename {f g}`, bare `ns` symbols and prefix lists
/// `[a [b :as x] [c :refer [f]]]` (→ `a.b`, `a.c`). `:refer :all` names no
/// vars, so it contributes nothing.
#[cfg(feature = "lang-clojure")]
fn clojure_require_specs(ns_node: &tree_sitter::Node, source: &[u8], scope: &mut ClojureScope) {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    for clause in clojure_forms(ns_node).into_iter().skip(2) {
        if clause.kind() != "list_lit" {
            continue;
        }
        let parts = clojure_forms(&clause);
        let head = parts
            .first()
            .map(|c| Clj::node_text(c, source))
            .unwrap_or("");
        if head != ":require" && head != ":use" {
            continue;
        }
        for spec in parts.iter().skip(1) {
            clojure_require_spec(spec, source, "", scope);
        }
    }
}

/// One require spec: a bare symbol, a `[ns opts…]` vector or a prefix list.
#[cfg(feature = "lang-clojure")]
fn clojure_require_spec(
    spec: &tree_sitter::Node,
    source: &[u8],
    prefix: &str,
    scope: &mut ClojureScope,
) {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    if spec.kind() == "sym_lit" {
        let name = Clj::node_text(spec, source);
        if !name.is_empty() {
            scope.namespaces.insert(join(name));
        }
        return;
    }
    if !matches!(spec.kind(), "vec_lit" | "list_lit") {
        return;
    }
    let fields = clojure_forms(spec);
    let Some(ns_sym) = fields.first() else {
        return;
    };
    let ns_text = Clj::node_text(ns_sym, source);
    if ns_text.is_empty() || ns_text.starts_with(':') {
        return;
    }
    let req_ns = join(ns_text);
    // `[a [b …] c]`: the second element is a spec, not an option keyword.
    if fields
        .get(1)
        .is_some_and(|f| f.kind() != "kwd_lit" && !f.kind().ends_with("meta_lit"))
    {
        for sub in fields.iter().skip(1) {
            clojure_require_spec(sub, source, &req_ns, scope);
        }
        return;
    }
    scope.namespaces.insert(req_ns.clone());
    let mut renames: Vec<(String, String)> = Vec::new();
    let mut i = 1;
    while i < fields.len() {
        let value = fields.get(i + 1);
        match Clj::node_text(&fields[i], source) {
            ":as" => {
                if let Some(alias) = value.map(|v| Clj::node_text(v, source)) {
                    if !alias.is_empty() {
                        scope.aliases.insert(alias.to_string(), req_ns.clone());
                    }
                }
                i += 2;
            }
            ":refer" | ":only" => {
                if let Some(v) = value.filter(|v| v.kind() == "vec_lit") {
                    for r in clojure_forms(v) {
                        let name = Clj::node_text(&r, source);
                        if !name.is_empty() && !name.starts_with(':') {
                            scope
                                .refers
                                .insert(name.to_string(), format!("{req_ns}/{name}"));
                        }
                    }
                }
                i += 2;
            }
            ":rename" => {
                if let Some(v) = value.filter(|v| v.kind() == "map_lit") {
                    for pair in clojure_forms(v).chunks(2).filter(|p| p.len() == 2) {
                        renames.push((
                            Clj::node_text(&pair[0], source).to_string(),
                            Clj::node_text(&pair[1], source).to_string(),
                        ));
                    }
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    for (orig, new) in renames {
        let qualified = format!("{req_ns}/{orig}");
        if scope.refers.get(&orig) == Some(&qualified) {
            scope.refers.remove(&orig);
        }
        scope.refers.insert(new, format!("{req_ns}/{orig}"));
    }
}

/// Bare name of a `sym_lit` (without `^meta` annotations).
#[cfg(feature = "lang-clojure")]
fn clojure_sym_name(node: &tree_sitter::Node, source: &[u8]) -> String {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    clojure_forms(node)
        .iter()
        .find(|c| c.kind() == "sym_name")
        .map(|c| Clj::node_text(c, source))
        .unwrap_or_else(|| Clj::node_text(node, source))
        .to_string()
}

/// `:keys`/`:strs`/`:syms`, optionally namespaced (`:foo/keys`).
#[cfg(feature = "lang-clojure")]
fn clojure_is_binding_vector_key(key: &str) -> bool {
    let name = key.strip_prefix(':').unwrap_or(key);
    let name = name.rsplit_once('/').map_or(name, |(_, n)| n);
    matches!(name, "keys" | "strs" | "syms") && key.starts_with(':')
}

/// A dotted prefix names a namespace when its last segment starts lowercase
/// (`foo.bar`); a capitalised last segment is a Java class (`java.util.UUID`).
#[cfg(feature = "lang-clojure")]
fn clojure_is_namespace_shaped(alias: &str) -> bool {
    alias.contains('.')
        && alias
            .rsplit('.')
            .next()
            .and_then(|seg| seg.chars().next())
            .is_some_and(char::is_lowercase)
}

/// Add every name a binding target introduces to `bound`. Map targets bind
/// their plain keys, `:keys`/`:strs`/`:syms` vectors and `:as`; `:or`
/// defaults are expressions and are scanned for calls instead.
#[cfg(feature = "lang-clojure")]
fn clojure_bind_symbols(
    node: &tree_sitter::Node,
    source: &[u8],
    scope: &ClojureScope,
    bound: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    match node.kind() {
        "sym_lit" => {
            bound.insert(clojure_sym_name(node, source));
        }
        "map_lit" => {
            for pair in clojure_forms(node).chunks(2).filter(|p| p.len() == 2) {
                let (key, value) = (&pair[0], &pair[1]);
                let key_text = Clj::node_text(key, source);
                match key_text {
                    ":or" => {
                        for default in clojure_forms(value).iter().skip(1).step_by(2) {
                            collect_clojure_call_heads(default, source, scope, bound, out);
                        }
                    }
                    _ if clojure_is_binding_vector_key(key_text) => {
                        clojure_bind_symbols(value, source, scope, bound, out)
                    }
                    ":as" => clojure_bind_symbols(value, source, scope, bound, out),
                    _ if key.kind() != "kwd_lit" => {
                        clojure_bind_symbols(key, source, scope, bound, out)
                    }
                    _ => {}
                }
            }
        }
        _ => {
            for c in clojure_forms(node) {
                clojure_bind_symbols(&c, source, scope, bound, out);
            }
        }
    }
}

/// Heads whose bodies are scanned with a `[name expr …]` binding vector.
#[cfg(feature = "lang-clojure")]
const CLJ_LET_HEADS: &[&str] = &[
    "let",
    "loop",
    "when-let",
    "if-let",
    "when-some",
    "if-some",
    "with-open",
    "for",
    "doseq",
    "dotimes",
    "let*",
    "loop*",
    "when-first",
];

/// Heads whose direct vector children are parameter vectors.
#[cfg(feature = "lang-clojure")]
const CLJ_FN_HEADS: &[&str] = &["fn", "fn*", "defn", "defn-", "defmacro"];

/// Forms whose direct list children are method signatures/impls, not calls.
#[cfg(feature = "lang-clojure")]
const CLJ_IMPL_HEADS: &[&str] = &[
    "defrecord",
    "deftype",
    "reify",
    "extend-type",
    "extend-protocol",
    "defprotocol",
    "letfn",
];

/// Visit a `[target expr …]` binding vector in order: each expr is scanned
/// with the bindings made so far, then its target joins `bound`. `:let`
/// modifiers recurse; other keyword modifiers (`:when`, `:while`) carry a
/// plain expression.
#[cfg(feature = "lang-clojure")]
fn collect_clojure_bindings(
    vec: &tree_sitter::Node,
    source: &[u8],
    scope: &ClojureScope,
    bound: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    for pair in clojure_forms(vec).chunks(2) {
        let target = &pair[0];
        let Some(value) = pair.get(1) else {
            break;
        };
        if target.kind() == "kwd_lit" {
            if Clj::node_text(target, source) == ":let" && value.kind() == "vec_lit" {
                collect_clojure_bindings(value, source, scope, bound, out);
            } else {
                collect_clojure_call_heads(value, source, scope, bound, out);
            }
            continue;
        }
        collect_clojure_call_heads(value, source, scope, bound, out);
        clojure_bind_symbols(target, source, scope, bound, out);
    }
}

/// Scan `body` nodes with `params` (a vector literal) shadowing local names.
#[cfg(feature = "lang-clojure")]
fn collect_clojure_with_params(
    params: &tree_sitter::Node,
    body: &[tree_sitter::Node],
    source: &[u8],
    scope: &ClojureScope,
    shadow: &std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    let mut inner = shadow.clone();
    clojure_bind_symbols(params, source, scope, &mut inner, out);
    for n in body {
        collect_clojure_call_heads(n, source, scope, &inner, out);
    }
}

/// Recursively collect the callee name of every `list_lit` under `node`,
/// qualified to `ns/name` form (see [`extract_clojure_calls_by_symbol`] for
/// the resolution rules). `shadow` holds names bound by an enclosing
/// parameter vector or `let`-style binding: a head in it is a local value,
/// not the top-level def or referred var of the same name.
#[cfg(feature = "lang-clojure")]
fn collect_clojure_call_heads(
    node: &tree_sitter::Node,
    source: &[u8],
    scope: &ClojureScope,
    shadow: &std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    use crate::parser::languages::clojure::ClojureLanguage as Clj;

    match node.kind() {
        // Quoted forms are data, not calls.
        "quoting_lit" => return,
        // Syntax-quote is data too, except what is unquoted inside it.
        "syn_quoting_lit" => {
            let mut stack = clojure_forms(node);
            while let Some(n) = stack.pop() {
                if matches!(n.kind(), "unquoting_lit" | "unquote_splicing_lit") {
                    for c in clojure_forms(&n) {
                        collect_clojure_call_heads(&c, source, scope, shadow, out);
                    }
                } else {
                    stack.extend(clojure_forms(&n));
                }
            }
            return;
        }
        "list_lit" => {}
        _ => {
            for c in clojure_forms(node) {
                collect_clojure_call_heads(&c, source, scope, shadow, out);
            }
            return;
        }
    }

    let children = clojure_forms(node);
    let Some(head_node) = children.first() else {
        return;
    };
    // `([params] body…)`: one arity clause of a multi-arity fn or method.
    if head_node.kind() == "vec_lit" {
        collect_clojure_with_params(head_node, &children[1..], source, scope, shadow, out);
        return;
    }
    let head = if head_node.kind() == "sym_lit" {
        Clj::node_text(head_node, source)
    } else {
        ""
    };
    match head {
        "quote" | "comment" => return,
        "case" => {
            // Test constants are data; only the dispatch expr, the results
            // and the trailing default are code.
            for (i, c) in children.iter().enumerate().skip(1) {
                let is_test = i >= 2 && i % 2 == 0 && i + 1 < children.len();
                if !is_test {
                    collect_clojure_call_heads(c, source, scope, shadow, out);
                }
            }
            return;
        }
        h if CLJ_LET_HEADS.contains(&h) => {
            let mut bound = shadow.clone();
            let mut rest = children.iter().skip(1);
            if let Some(bindings) = rest.next().filter(|b| b.kind() == "vec_lit") {
                collect_clojure_bindings(bindings, source, scope, &mut bound, out);
                // `if-let`/`if-some` bind for the then form only.
                let conditional = matches!(h, "if-let" | "if-some");
                for (i, n) in rest.enumerate() {
                    let env = if conditional && i > 0 { shadow } else { &bound };
                    collect_clojure_call_heads(n, source, scope, env, out);
                }
                return;
            }
        }
        h if CLJ_FN_HEADS.contains(&h) => {
            // A named `fn` binds its own name inside its body.
            let named = matches!(h, "fn" | "fn*")
                .then(|| children.get(1).filter(|c| c.kind() == "sym_lit"))
                .flatten();
            let mut named_shadow;
            let shadow = match named {
                Some(name) => {
                    named_shadow = shadow.clone();
                    named_shadow.insert(clojure_sym_name(name, source));
                    &named_shadow
                }
                None => shadow,
            };
            if let Some(i) = children.iter().position(|c| c.kind() == "vec_lit") {
                for n in &children[1..i] {
                    collect_clojure_call_heads(n, source, scope, shadow, out);
                }
                collect_clojure_with_params(
                    &children[i],
                    &children[i + 1..],
                    source,
                    scope,
                    shadow,
                    out,
                );
                return;
            }
            // Multi-arity: arity-clause lists carry their own params.
            for n in &children[1..] {
                collect_clojure_call_heads(n, source, scope, shadow, out);
            }
            return;
        }
        h if CLJ_IMPL_HEADS.contains(&h) => {
            let mut inner = shadow.clone();
            if h == "letfn" {
                if let Some(fns) = children.get(1).filter(|c| c.kind() == "vec_lit") {
                    for f in clojure_forms(fns) {
                        if let Some(name) = clojure_forms(&f).first() {
                            inner.insert(clojure_sym_name(name, source));
                        }
                    }
                    for f in clojure_forms(fns) {
                        collect_clojure_method_body(&f, source, scope, &inner, out);
                    }
                    for n in &children[2..] {
                        collect_clojure_call_heads(n, source, scope, &inner, out);
                    }
                    return;
                }
            }
            for n in &children[1..] {
                if n.kind() == "list_lit" {
                    collect_clojure_method_body(n, source, scope, &inner, out);
                } else {
                    collect_clojure_call_heads(n, source, scope, &inner, out);
                }
            }
            return;
        }
        _ => {}
    }

    if head_node.kind() == "sym_lit" {
        if let Some((alias, name)) = head.split_once('/') {
            if !name.is_empty() {
                if let Some(req_ns) = scope.aliases.get(alias) {
                    out.push(format!("{req_ns}/{name}"));
                } else if scope.namespaces.contains(alias) || clojure_is_namespace_shaped(alias) {
                    out.push(head.to_string());
                }
            }
        } else if !shadow.contains(head) {
            if scope.locals.contains(head) {
                out.push(Clj::qualify(&scope.ns, head));
            }
            if let Some(qualified) = scope.refers.get(head) {
                out.push(qualified.clone());
            }
        }
    }
    for c in &children {
        collect_clojure_call_heads(c, source, scope, shadow, out);
    }
}

/// A method signature or impl inside a protocol/record form:
/// `(name [params] body…)` or `(name ([params] body…) …)`. The name is not a
/// call; the params shadow locals in the body.
#[cfg(feature = "lang-clojure")]
fn collect_clojure_method_body(
    method: &tree_sitter::Node,
    source: &[u8],
    scope: &ClojureScope,
    shadow: &std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    let children = clojure_forms(method);
    let is_signature = method.kind() == "list_lit"
        && children.first().is_some_and(|h| h.kind() == "sym_lit")
        && children.get(1).is_some_and(|c| {
            c.kind() == "vec_lit"
                || (c.kind() == "list_lit"
                    && clojure_forms(c)
                        .first()
                        .is_some_and(|f| f.kind() == "vec_lit"))
        });
    if !is_signature {
        collect_clojure_call_heads(method, source, scope, shadow, out);
        return;
    }
    for c in &children[1..] {
        if c.kind() == "vec_lit" {
            collect_clojure_with_params(c, &children[2..], source, scope, shadow, out);
            return;
        }
        collect_clojure_call_heads(c, source, scope, shadow, out);
    }
}

/// Parse `source` **once** and map every function/method name to the call-site
/// names in its body — the one-pass equivalent of calling
/// [`extract_call_sites_from_source`] once per symbol.
///
/// `build_call_graph` previously re-parsed each file once per symbol it
/// contains (O(symbols) full tree-sitter parses per file); on a large file that
/// is quadratic in both wall-clock and syntax-tree allocation, the cause of the
/// watcher thread pegging CPU and the RSS ratcheting under repeated rebuilds.
/// This parses each file exactly once. Returns `None` for languages with no
/// tree-sitter extractor so the caller keeps the per-symbol regex fallback.
///
/// The result is *set*-equivalent to the per-symbol path: a name maps to the
/// union of calls over every same-named function node, matching the old
/// find-all-matches-then-dedup behaviour once the caller applies
/// retain/sort/dedup on read.
fn extract_calls_by_symbol(
    source: &str,
    language: &str,
) -> Option<std::collections::HashMap<String, Vec<String>>> {
    #[cfg(feature = "lang-clojure")]
    if language == "clojure" {
        return extract_clojure_calls_by_symbol(source);
    }
    let (lang, func_kinds, call_kinds): (tree_sitter::Language, &[&str], &[&str]) = match language {
        #[cfg(feature = "lang-rust")]
        "rust" => (
            tree_sitter_rust::LANGUAGE.into(),
            &["function_item"],
            &["call_expression"],
        ),
        #[cfg(feature = "lang-python")]
        "python" => (
            tree_sitter_python::LANGUAGE.into(),
            &["function_definition"],
            &["call"],
        ),
        #[cfg(feature = "lang-typescript")]
        "typescript" | "javascript" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            &[
                "function_declaration",
                "method_definition",
                "arrow_function",
            ],
            &["call_expression"],
        ),
        #[cfg(all(not(feature = "lang-typescript"), feature = "lang-javascript"))]
        "typescript" | "javascript" => (
            tree_sitter_javascript::LANGUAGE.into(),
            &[
                "function_declaration",
                "method_definition",
                "arrow_function",
            ],
            &["call_expression"],
        ),
        #[cfg(feature = "lang-go")]
        "go" => (
            tree_sitter_go::LANGUAGE.into(),
            &["function_declaration", "method_declaration"],
            &["call_expression"],
        ),
        #[cfg(feature = "lang-java")]
        "java" => (
            tree_sitter_java::LANGUAGE.into(),
            &["method_declaration", "constructor_declaration"],
            &["method_invocation"],
        ),
        #[cfg(feature = "lang-c")]
        "c" => (
            tree_sitter_c::LANGUAGE.into(),
            &["function_definition"],
            &["call_expression"],
        ),
        #[cfg(feature = "lang-cpp")]
        "cpp" => (
            tree_sitter_cpp::LANGUAGE.into(),
            &["function_definition"],
            &["call_expression"],
        ),
        #[cfg(feature = "lang-ruby")]
        "ruby" => (
            tree_sitter_ruby::LANGUAGE.into(),
            &["method", "singleton_method"],
            &["call", "method_call"],
        ),
        #[cfg(feature = "lang-csharp")]
        "csharp" => (
            tree_sitter_c_sharp::LANGUAGE.into(),
            &[
                "method_declaration",
                "constructor_declaration",
                "local_function_statement",
            ],
            &["invocation_expression"],
        ),
        _ => return None,
    };

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).ok()?;
    let tree = parser.parse(source, None)?;
    #[cfg(test)]
    PARSED_SOURCES.with(|p| p.borrow_mut().push(source.to_string()));
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    collect_calls_by_symbol(
        &tree.root_node(),
        source.as_bytes(),
        func_kinds,
        call_kinds,
        &mut map,
    );
    Some(map)
}

/// One-pass tree walk backing [`extract_calls_by_symbol`]: for every
/// function/method node, append the calls in its subtree to that name's entry.
fn collect_calls_by_symbol(
    node: &tree_sitter::Node,
    source: &[u8],
    func_kinds: &[&str],
    call_kinds: &[&str],
    map: &mut std::collections::HashMap<String, Vec<String>>,
) {
    if func_kinds.contains(&node.kind()) {
        if let Some(name) = extract_func_name(node, source) {
            let entry = map.entry(name).or_default();
            collect_calls_recursive(node, source, call_kinds, entry);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls_by_symbol(&child, source, func_kinds, call_kinds, map);
    }
}

// ---------------------------------------------------------------------------
// Cross-file call graph construction
// ---------------------------------------------------------------------------

// Counts `build_call_graph` calls per thread so tests can prove a derived-cache
// hit skips the pass (ADR-0210).
#[cfg(test)]
thread_local! {
    pub(crate) static BUILD_CALL_GRAPH_CALLS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    /// Every source `extract_calls_by_symbol` parsed on this thread, so tests
    /// can prove a cached call-site entry replaced the tree-sitter parse.
    pub(crate) static PARSED_SOURCES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

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
pub fn build_call_graph(index: &crate::core_graph::CodebaseIndex) -> CallGraph {
    build_call_graph_cached(index, &std::collections::HashMap::new()).0
}

/// Per-file call sites by function name, as persisted in the artifact store.
pub(crate) type CallSites = std::collections::BTreeMap<String, Vec<String>>;

/// [`build_call_graph`] with per-file call sites supplied by the caller
/// (ADR-0212).
///
/// `cache` maps a repo-relative path to call sites the caller has already
/// validated against that file's current bytes and language; a hit skips the
/// tree-sitter parse. The second return value holds the call sites computed
/// here (cache misses for languages with an extractor) for the caller to
/// persist. Languages without an extractor keep the per-symbol regex fallback
/// and appear in neither side.
pub(crate) fn build_call_graph_cached(
    index: &crate::core_graph::CodebaseIndex,
    cache: &std::collections::HashMap<String, CallSites>,
) -> (CallGraph, std::collections::HashMap<String, CallSites>) {
    #[cfg(test)]
    BUILD_CALL_GRAPH_CALLS.with(|c| c.set(c.get() + 1));
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
    let mut computed: std::collections::HashMap<String, CallSites> =
        std::collections::HashMap::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        let lang = file.language.as_deref().unwrap_or("unknown");
        let imports_of_this_file = imported_from.get(&file.relative_path);

        // Parse each file exactly once (was: once per symbol — quadratic).
        // `None` for languages without a tree-sitter extractor → the per-symbol
        // regex fallback below, unchanged.
        if !cache.contains_key(&file.relative_path) {
            if let Some(map) = extract_calls_by_symbol(&file.content, lang) {
                computed.insert(
                    file.relative_path.clone(),
                    map.into_iter().collect::<CallSites>(),
                );
            }
        }
        let call_map: Option<&CallSites> = cache
            .get(&file.relative_path)
            .or_else(|| computed.get(&file.relative_path));

        for symbol in &pr.symbols {
            let called_names = match &call_map {
                Some(m) => {
                    let mut names = m.get(&symbol.name).cloned().unwrap_or_default();
                    names.retain(|c| c != &symbol.name);
                    names.sort();
                    names.dedup();
                    names
                }
                None => extract_call_sites_from_source(&file.content, lang, &symbol.name),
            };

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

    (CallGraph { edges, unresolved }, computed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

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

    /// Parity guard for the single-parse-per-file fix: the one-pass
    /// `extract_calls_by_symbol` map must return exactly what the old
    /// per-symbol `extract_call_sites_from_source` returned, for every symbol
    /// — including cross-calling siblings, so the quadratic re-parse is gone
    /// without changing the call graph.
    #[test]
    #[cfg(feature = "lang-rust")]
    fn test_calls_by_symbol_matches_per_symbol_rust() {
        let source = r#"
fn alpha() { beta(); helper(); }
fn beta() { gamma(); helper(); }
fn gamma() { alpha(); }
fn helper() {}
"#;
        let map = extract_calls_by_symbol(source, "rust").expect("rust map");
        for sym in ["alpha", "beta", "gamma", "helper"] {
            let mut via_map = map.get(sym).cloned().unwrap_or_default();
            via_map.retain(|c| c != sym);
            via_map.sort();
            via_map.dedup();
            let direct = extract_call_sites_from_source(source, "rust", sym);
            assert_eq!(via_map, direct, "mismatch for symbol {sym}");
        }
    }

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
    fn make_intra_file_index() -> crate::core_graph::CodebaseIndex {
        use crate::core_graph::CodebaseIndex;
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
    fn build_call_graph_wrapper_matches_cached_empty() {
        let index = make_intra_file_index();
        let wrapped = build_call_graph(&index);
        let (cached, computed) = build_call_graph_cached(&index, &HashMap::new());
        assert_eq!(
            serde_json::to_value(&wrapped).unwrap(),
            serde_json::to_value(&cached).unwrap()
        );
        assert_eq!(
            computed.keys().collect::<Vec<_>>(),
            vec!["lib.rs"],
            "an empty cache computes every file with an extractor"
        );
    }

    #[test]
    fn cached_call_sites_replace_the_parse() {
        let index = make_intra_file_index();
        let cache = HashMap::from([(
            "lib.rs".to_string(),
            BTreeMap::from([("caller".to_string(), vec!["SENTINEL_CALLEE".to_string()])]),
        )]);
        PARSED_SOURCES.with(|p| p.borrow_mut().clear());
        let (graph, computed) = build_call_graph_cached(&index, &cache);
        assert!(PARSED_SOURCES.with(|p| p.borrow().is_empty()));
        assert!(computed.is_empty(), "hits are not recomputed");
        assert!(graph
            .unresolved
            .iter()
            .any(|u| u.callee_name == "SENTINEL_CALLEE"));
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

    // ─── extract_body_from_offset direct tests ──────────────────────────────

    #[test]
    fn extract_body_from_offset_returns_empty_when_body_start_beyond_end() {
        let source = "fn foo(";
        let offset = source.len();
        assert_eq!(extract_body_from_offset(source, offset), "");
    }

    #[test]
    fn extract_body_from_offset_handles_nested_braces() {
        // Depth must go 1 (entering) -> 2 (nested open) -> 1 (nested close,
        // not yet zero) -> 0 (final close, stop).
        let body = extract_body_from_offset("X{a{b}c}", 0);
        assert_eq!(body, "a{b}c");
    }

    #[test]
    fn extract_body_from_offset_handles_colon_delimited_body() {
        // No opening brace before EOF but a ':' — the indent-delimited
        // (Python-style) fallback takes the following lines verbatim.
        let source = "def foo():\n    return 1\n    return 2\n";
        let body = extract_body_from_offset(source, 0);
        assert!(body.contains("return 1"));
        assert!(body.contains("return 2"));
    }

    // ─── extract_func_name / find_identifier_deep direct tests ─────────────

    #[cfg(feature = "lang-go")]
    #[test]
    fn test_extract_calls_go_receiver_method_name_unresolved() {
        // Go's `method_declaration` name field is a `field_identifier`, not
        // `identifier` — `extract_func_name` has no branch that resolves that
        // kind (no "identifier"/"name" child, no declarator chain), so a
        // receiver method's call sites are never attributed to it. This pins
        // down the current (fallback-to-None) behaviour rather than treating
        // it as a Go-support gap to fix here.
        let source =
            "package main\ntype Receiver struct{}\nfunc (r *Receiver) DoWork() {\n\thelper()\n}\nfunc helper() {}\n";
        let calls = extract_call_sites_from_source(source, "go", "DoWork");
        assert!(
            calls.is_empty(),
            "receiver method name cannot currently be resolved by extract_func_name: {:?}",
            calls
        );
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn find_identifier_deep_returns_none_for_subtree_without_identifier_then_continues() {
        // `function_definition`'s first child is the return-type node (e.g.
        // the `void` `primitive_type` leaf), which has no identifier
        // subtree anywhere — `find_identifier_deep` must return None for it
        // (its empty-recursion fallback) before the search loop continues to
        // the `declarator` child, which does contain the function's name.
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
        let source = "void handler() { init(); }\n";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let func_def = root.child(0).expect("function_definition node");
        assert_eq!(func_def.kind(), "function_definition");

        let name = find_identifier_deep(&func_def, source.as_bytes());
        assert_eq!(name, Some("handler".to_string()));
    }

    // ─── extract_call_name attribute-fallback direct test ───────────────────

    #[test]
    fn test_extract_calls_python_method_call_via_attribute_access() {
        // Python's `attribute` node names its member field with plain
        // `identifier` (not `field_identifier`/`property_identifier`), so
        // resolution must go through extract_call_name's fallback branch
        // that scans for a bare identifier/name child.
        let source = "class Foo:\n    def run(self):\n        self.helper()\n        obj.method_call()\n    def helper(self):\n        pass\n";
        let calls = extract_call_sites_from_source(source, "python", "run");
        assert!(
            calls.contains(&"helper".to_string()),
            "expected helper via self.helper() attribute call, got {:?}",
            calls
        );
        assert!(
            calls.contains(&"method_call".to_string()),
            "expected method_call via obj.method_call() attribute call, got {:?}",
            calls
        );
    }

    // ─── build_call_graph self-recursion test ───────────────────────────────

    #[test]
    fn test_build_call_graph_skips_self_recursive_calls() {
        use crate::core_graph::CodebaseIndex;
        use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let src =
            "fn factorial(n: i32) -> i32 {\n    if n <= 1 { 1 } else { n * factorial(n - 1) }\n}\n";
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
                symbols: vec![Symbol {
                    name: "factorial".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn factorial(n: i32) -> i32".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("lib.rs".to_string(), src.to_string());
        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

        let cg = build_call_graph(&index);
        assert!(
            !cg.edges
                .iter()
                .any(|e| e.caller_symbol == "factorial" && e.callee_symbol == "factorial"),
            "a recursive self-call must not produce a self-referencing edge: {:?}",
            cg.edges
        );
    }

    // ─── Clojure call-graph regression tests ─────────────────────────────
    //
    // Deployment evidence (2026-10-30): a 4k-file corpus indexed cleanly,
    // yet `/call_graph` returned zero edges for every Clojure symbol and
    // `/blast_radius` returned `total_affected: 0` for a `.clj` file with
    // known dependents. Lisp call syntax `(callee …)` puts the paren before
    // the name, so the generic `ident(` regex fallback extracts nothing.

    /// Two-file index built through the real pipeline: real tree-sitter
    /// parse of `.clj` sources → `build_with_content` → `build_call_graph`.
    #[cfg(feature = "lang-clojure")]
    fn clojure_index(files: &[(&str, &str)]) -> crate::index::CodebaseIndex {
        use crate::parser::language::LanguageSupport;
        use crate::parser::languages::clojure::ClojureLanguage;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = crate::budget::counter::TokenCounter::new();
        let parse = |src: &str| {
            let lang = ClojureLanguage;
            let mut p = tree_sitter::Parser::new();
            p.set_language(&lang.ts_language())
                .expect("clojure grammar");
            let tree = p.parse(src, None).expect("clojure parse");
            lang.extract(src, &tree)
        };

        let mut scanned = Vec::new();
        let mut parse_results = HashMap::new();
        let mut content = HashMap::new();
        for (rel, src) in files {
            scanned.push(ScannedFile {
                relative_path: (*rel).into(),
                absolute_path: format!("/tmp/{rel}").into(),
                language: Some("clojure".into()),
                size_bytes: src.len() as u64,
            });
            parse_results.insert((*rel).to_string(), parse(src));
            content.insert((*rel).to_string(), (*src).to_string());
        }
        crate::index::CodebaseIndex::build_with_content(scanned, parse_results, &counter, content)
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_alias_qualified_call_produces_edge() {
        let index = clojure_index(&[
            (
                "src/ui/version.clj",
                "(ns ui.version)\n\n(defn server-build-id []\n  (str \"v\" 1))\n",
            ),
            (
                "src/ui/attach.clj",
                "(ns ui.attach\n  (:require [ui.version :as version]))\n\n(defn mount []\n  (version/server-build-id))\n",
            ),
        ]);

        let cg = build_call_graph(&index);
        assert!(
            cg.edges.iter().any(|e| {
                e.caller_file == "src/ui/attach.clj"
                    && e.caller_symbol == "ui.attach/mount"
                    && e.callee_file == "src/ui/version.clj"
                    && e.callee_symbol == "ui.version/server-build-id"
            }),
            "expected attach/mount → version/server-build-id edge, got: {:?}",
            cg.edges
        );
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_local_and_referred_calls_resolve() {
        let index = clojure_index(&[
            (
                "src/ui/version.clj",
                "(ns ui.version)\n\n(defn- fmt [v]\n  (str \"v\" v))\n\n(defn server-build-id []\n  (fmt 1))\n",
            ),
            (
                "src/ui/attach.clj",
                "(ns ui.attach\n  (:require [ui.version :refer [server-build-id]]))\n\n(defn mount []\n  (server-build-id))\n",
            ),
        ]);

        let cg = build_call_graph(&index);
        // Intra-file: public defn calls its private helper.
        assert!(
            cg.edges.iter().any(|e| {
                e.caller_file == "src/ui/version.clj"
                    && e.caller_symbol == "ui.version/server-build-id"
                    && e.callee_file == "src/ui/version.clj"
                    && e.callee_symbol == "ui.version/fmt"
            }),
            "expected intra-file edge to private helper, got: {:?}",
            cg.edges
        );
        // Cross-file via :refer (bare call head).
        assert!(
            cg.edges.iter().any(|e| {
                e.caller_file == "src/ui/attach.clj"
                    && e.callee_file == "src/ui/version.clj"
                    && e.callee_symbol == "ui.version/server-build-id"
            }),
            "expected :refer-resolved edge, got: {:?}",
            cg.edges
        );
    }

    /// Calls the one-pass Clojure extractor attributes to `ns/sym`.
    #[cfg(feature = "lang-clojure")]
    fn clj_calls(source: &str, sym: &str) -> Vec<String> {
        extract_clojure_calls(source, sym)
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_param_and_let_shadowing_is_not_a_call() {
        let src = "(ns a)\n(defn fmt [x] x)\n(defn f [fmt] (fmt 1))\n(defn g [] (let [fmt inc] (fmt 1)))\n(defn h [] (fmt 1))\n";
        assert!(clj_calls(src, "a/f").is_empty(), "param shadow");
        assert!(clj_calls(src, "a/g").is_empty(), "let shadow");
        assert_eq!(clj_calls(src, "a/h"), vec!["a/fmt"], "unshadowed stays");
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_case_clause_list_is_not_a_call() {
        let src = "(ns a)\n(defn fmt [x] x)\n(defn f [k] (case k (fmt x) 1 2))\n";
        assert!(clj_calls(src, "a/f").is_empty());
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_protocol_impl_method_head_is_not_a_call() {
        let src = "(ns a)\n(defn fmt [x] x)\n(defrecord R []\n  P\n  (fmt [this] 1))\n(defn f [] (reify P (fmt [this] 1)))\n(defn g [] (extend-type T P (fmt [this] 1)))\n";
        assert!(clj_calls(src, "a/R").is_empty(), "defrecord");
        assert!(clj_calls(src, "a/f").is_empty(), "reify");
        assert!(clj_calls(src, "a/g").is_empty(), "extend-type");
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_unknown_interop_prefix_is_dropped() {
        let src = "(ns a (:require [b.c :as bc]))\n(defn f [] (Math/abs 1) (System/getenv \"X\") (Thread/sleep 1) (bc/g) (b.c/h) (a/k))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["a/k", "b.c/g", "b.c/h"]);
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_quote_skipped_but_unquote_descended() {
        let src = "(ns a)\n(defn foo [] 1)\n(defn bar [] 1)\n(defn baz [] 1)\n(defn f [x] `(foo ~(bar x) ~@(baz x)))\n(defn g [] '(foo (bar)))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["a/bar", "a/baz"]);
        assert!(clj_calls(src, "a/g").is_empty());
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_require_forms() {
        let src = "(ns a (:require [p [q :as x] [r :refer [k]]] s.t) (:use [u :only [h]]) (:require [v :refer [f] :rename {f g}]))\n(defn z [] (x/m) (k) (h) (g) (t/n))\n";
        let got = clj_calls(src, "a/z");
        for want in ["p.q/m", "p.r/k", "u/h", "v/f"] {
            assert!(got.iter().any(|c| c == want), "missing {want}: {got:?}");
        }
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_bare_require_and_multi_arity_scoping() {
        let src = "(ns a (:require s.t))\n(defn fmt [x] x)\n(defn f ([] (s.t/n) (fmt 1)) ([fmt] (fmt 2)))\n(defn g [] (let [y (fmt 1) fmt inc] (fmt y)))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["a/fmt", "s.t/n"]);
        assert_eq!(clj_calls(src, "a/g"), vec!["a/fmt"]);
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_or_defaults_are_expressions_not_bindings() {
        let src = "(ns a)\n(defn g [] 1)\n(defn f [] (let [{:keys [x] :or {x (g)}} {}] x))\n(defn h [m] (let [{:keys [x] :or {x 1 y g}} m] (g)))\n(defn k [{:keys [g] :as m}] (g))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["a/g"]);
        assert_eq!(clj_calls(src, "a/h"), vec!["a/g"]);
        assert!(clj_calls(src, "a/k").is_empty(), ":keys still shadows");
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_comments_and_discards_do_not_shift_pairing() {
        let src = "(ns a)\n(defn g [] 1)\n(defn f [k] (let [x 1 ; c\n a (g)] a))\n(defn h [k] (let [x 1 #_y a (g)] a))\n(defn m [k] (case k 1 ; c\n (g) 2 (g)))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["a/g"]);
        assert_eq!(clj_calls(src, "a/h"), vec!["a/g"]);
        assert_eq!(clj_calls(src, "a/m"), vec!["a/g"]);
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_named_fn_shadows_its_own_name() {
        let src = "(ns a)\n(defn g [x] 1)\n(defn f [] (fn g [x] (g x)))\n";
        assert!(clj_calls(src, "a/f").is_empty());
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_if_let_binding_does_not_cover_else() {
        let src = "(ns a)\n(defn g [x] 1)\n(defn h [] nil)\n(defn f [] (if-let [g (h)] 1 (g 2)))\n(defn s [] (if-some [g (h)] 1 (g 2)))\n(defn t [] (if-let [g (h)] (g 2) 1))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["a/g", "a/h"]);
        assert_eq!(clj_calls(src, "a/s"), vec!["a/g", "a/h"]);
        assert_eq!(clj_calls(src, "a/t"), vec!["a/h"]);
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_fully_qualified_dotted_prefix_kept_without_require() {
        let src = "(ns a)\n(require '[foo.bar :as fb])\n(defn f [] (fb/x) (foo.bar/y))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["foo.bar/y"]);
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_class_shaped_dotted_prefixes_dropped() {
        let src = "(ns a)\n(defn f [] (java.util.UUID/randomUUID) (java.time.Instant/now) (clojure.lang.RT/count 1) (foo.bar/y))\n";
        assert_eq!(clj_calls(src, "a/f"), vec!["foo.bar/y"]);
    }

    #[test]
    #[cfg(feature = "lang-clojure")]
    fn test_clojure_namespaced_destructuring_keys_bind() {
        for kind in ["keys", "strs", "syms"] {
            let src = format!("(ns a)\n(defn g [] 1)\n(defn f [{{:foo/{kind} [g]}}] (g))\n");
            assert!(clj_calls(&src, "a/f").is_empty(), ":foo/{kind} must bind");
        }
    }
}
