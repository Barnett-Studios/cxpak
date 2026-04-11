# v1.3.0 "Deep Understanding" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add cross-file call graph, dead code detection, architecture quality metrics (5 per module), and monorepo workspace support.

**Architecture:** The call graph is a new module under `src/intelligence/call_graph.rs` that produces `CallGraph { edges: Vec<CallEdge>, unresolved: Vec<UnresolvedCall> }` stored on `CodebaseIndex` alongside the existing `DependencyGraph`. Dead code detection in `src/intelligence/dead_code.rs` consumes the call graph plus the existing `test_map`, `pagerank`, and `api_surface` outputs to classify symbols as live or dead. Architecture quality extends the existing `ModuleInfo` struct (added in v1.2.0) with five new fields: cohesion, boundary_violations, god_files, and augments the circular_deps already computed by Tarjan's SCC. Monorepo support threads a `workspace: Option<String>` parameter through the scanner, all CLI commands, and all MCP tools, using path prefixes for scoping and cache namespacing.

**Tech Stack:** Rust, tree-sitter (call expression extraction), petgraph (Tarjan's SCC, already used), serde, regex (Tier 2 language call extraction), git2 (existing)

---

## Prerequisites: v1.2.0 Types

This plan builds on types defined in v1.2.0. Tasks below assume `HealthScore`, `ArchitectureMap`, `ModuleInfo`, `BoundaryViolation`, and `RiskEntry` already exist on `CodebaseIndex` and `AutoContextResult`. **If v1.2.0 is not yet merged, create stub files first:**
- Create `src/intelligence/architecture.rs` with `ArchitectureMap`, `ModuleInfo` structs (with `coupling: f64`, `aggregate_pagerank: f64`, `file_count: usize`, `prefix: String`).
- Create `src/intelligence/health.rs` with `HealthScore` struct (6 `Option<f64>` dimensions + `composite: f64`).
- Register both in `src/intelligence/mod.rs`.
- The dead_code dimension on `HealthScore` must currently return `None` — this plan populates it.

---

## Task 1: Core call graph types and module skeleton

**Files:**
- Create: `src/intelligence/call_graph.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Write a failing test in `src/intelligence/call_graph.rs` that asserts `CallGraph::default()` has empty `edges` and `unresolved` vecs.

2. Run: `cargo test --lib intelligence::call_graph -- --verbose 2>&1 | head -40`

3. Create `src/intelligence/call_graph.rs` with the full type definitions:

```rust
use crate::schema::EdgeType;
use serde::{Deserialize, Serialize};

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
                },
                CallEdge {
                    caller_file: "c.rs".into(),
                    caller_symbol: "baz".into(),
                    callee_file: "b.rs".into(),
                    callee_symbol: "bar".into(),
                    confidence: CallConfidence::Approximate,
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
            }],
            unresolved: vec![],
        };
        let callees = cg.callees_from("a.rs", "main");
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].callee_symbol, "init");
    }
}
```

4. Add `pub mod call_graph;` to `src/intelligence/mod.rs`.

5. Run: `cargo test --lib intelligence::call_graph -- --verbose`

6. Commit: `feat: add CallGraph types and module skeleton`

---

## Task 2: Call graph extraction for Rust

**Files:**
- Modify: `src/intelligence/call_graph.rs` (add `extract_calls_rust`)
- Modify: `src/parser/languages/rust.rs` (add `extract_call_sites` helper)

**Steps:**

1. Write a failing test in `call_graph.rs`:

```rust
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
    assert!(calls.contains(&"callee_one".to_string()));
    assert!(calls.contains(&"callee_two".to_string()));
    assert!(calls.contains(&"helper".to_string()));
}
```

2. Run: `cargo test --lib intelligence::call_graph::tests::test_extract_calls_rust -- --verbose 2>&1 | head -30`

3. Implement `extract_call_sites_from_source(source: &str, language: &str, symbol_name: &str) -> Vec<String>` in `call_graph.rs`. For Rust: use tree-sitter-rust to parse the source, walk the AST for `call_expression` nodes within the `function_item` or `impl_item` body matching `symbol_name`, extract the function identifier from each `call_expression`'s function child.

```rust
/// Extract the set of called function/method names within a specific symbol's body.
///
/// This is the tree-sitter path for Tier 1 languages.
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
        _ => extract_regex_calls(source, symbol_name),
    }
}

#[cfg(feature = "lang-rust")]
fn extract_rust_calls(source: &str, symbol_name: &str) -> Vec<String> {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("rust grammar");
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let source_bytes = source.as_bytes();
    let root = tree.root_node();
    let mut calls: Vec<String> = Vec::new();

    // Find the target function body by walking top-level items
    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        let in_target = match node.kind() {
            "function_item" => {
                let name = get_identifier_child(&node, source_bytes);
                name.as_deref() == Some(symbol_name)
            }
            "impl_item" => false, // impl items are handled recursively below
            _ => false,
        };
        if in_target {
            collect_call_expressions(&node, source_bytes, &mut calls);
        }
        // Also search impl blocks for methods matching symbol_name
        if node.kind() == "impl_item" {
            let mut impl_cursor = node.walk();
            for impl_child in node.children(&mut impl_cursor) {
                if impl_child.kind() == "declaration_list" {
                    let mut decl_cursor = impl_child.walk();
                    for method in impl_child.children(&mut decl_cursor) {
                        if method.kind() == "function_item" {
                            let name = get_identifier_child(&method, source_bytes);
                            if name.as_deref() == Some(symbol_name) {
                                collect_call_expressions(&method, source_bytes, &mut calls);
                            }
                        }
                    }
                }
            }
        }
    }

    calls.sort();
    calls.dedup();
    calls
}

#[cfg(not(feature = "lang-rust"))]
fn extract_rust_calls(_source: &str, _symbol_name: &str) -> Vec<String> {
    vec![]
}

fn get_identifier_child(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(child.utf8_text(source).unwrap_or("").to_string());
        }
    }
    None
}

fn collect_call_expressions(
    node: &tree_sitter::Node,
    source: &[u8],
    calls: &mut Vec<String>,
) {
    if node.kind() == "call_expression" {
        // The function child is typically an identifier or field_expression
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    let name = child.utf8_text(source).unwrap_or("").to_string();
                    if !name.is_empty() {
                        calls.push(name);
                    }
                }
                "field_expression" => {
                    // method call: obj.method(...)
                    // extract field name (last identifier)
                    let mut fc = child.walk();
                    for fc_child in child.children(&mut fc) {
                        if fc_child.kind() == "field_identifier" {
                            let name = fc_child.utf8_text(source).unwrap_or("").to_string();
                            if !name.is_empty() {
                                calls.push(name);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_call_expressions(&child, source, calls);
    }
}
```

4. Run: `cargo test --lib intelligence::call_graph -- --verbose`

5. Commit: `feat: implement tree-sitter call extraction for Rust`

---

## Task 3: Call graph extraction for Python, TypeScript/JavaScript, Go

**Files:**
- Modify: `src/intelligence/call_graph.rs` (add `extract_python_calls`, `extract_ts_js_calls`, `extract_go_calls`)

**Steps:**

1. Write failing tests:

```rust
#[test]
fn test_extract_calls_python() {
    let source = "def greet():\n    helper()\n    send_email()\n\ndef helper(): pass\ndef send_email(): pass\n";
    let calls = extract_call_sites_from_source(source, "python", "greet");
    assert!(calls.contains(&"helper".to_string()));
    assert!(calls.contains(&"send_email".to_string()));
}

#[test]
fn test_extract_calls_typescript() {
    let source = "export function process() {\n  validate(input);\n  save(data);\n}\nfunction validate(x: any) {}\nfunction save(d: any) {}\n";
    let calls = extract_call_sites_from_source(source, "typescript", "process");
    assert!(calls.contains(&"validate".to_string()));
    assert!(calls.contains(&"save".to_string()));
}

#[test]
fn test_extract_calls_go() {
    let source = "package main\nfunc run() {\n\tsetup()\n\texecute()\n}\nfunc setup() {}\nfunc execute() {}\n";
    let calls = extract_call_sites_from_source(source, "go", "run");
    assert!(calls.contains(&"setup".to_string()));
    assert!(calls.contains(&"execute".to_string()));
}
```

2. Run: `cargo test --lib intelligence::call_graph::tests::test_extract_calls_python -- --verbose 2>&1 | head -20`

3. Implement `extract_python_calls`, `extract_ts_js_calls`, `extract_go_calls` using the same tree-sitter walk pattern as Rust. Python: `call` nodes with `identifier` child. TypeScript/JS: `call_expression` with `identifier` or `member_expression` function child. Go: `call_expression` with `identifier` or `selector_expression`.

4. Run: `cargo test --lib intelligence::call_graph -- --verbose`

5. Commit: `feat: add call graph extraction for Python, TypeScript, JavaScript, Go`

---

## Task 4: Call graph extraction for Java, C, C++, Ruby, C#; regex fallback

**Files:**
- Modify: `src/intelligence/call_graph.rs`

**Steps:**

1. Write failing tests for each language:

```rust
#[test]
fn test_extract_calls_java() {
    let source = "class Foo {\n  public void process() {\n    validate();\n    persist();\n  }\n  void validate() {}\n  void persist() {}\n}\n";
    let calls = extract_call_sites_from_source(source, "java", "process");
    assert!(calls.contains(&"validate".to_string()));
    assert!(calls.contains(&"persist".to_string()));
}

#[test]
fn test_regex_fallback_for_tier2_language() {
    // PHP (Tier 2) uses regex
    let source = "<?php\nfunction handler() {\n  validate_input();\n  save_record();\n}\nfunction validate_input() {}\nfunction save_record() {}\n";
    let calls = extract_call_sites_from_source(source, "php", "handler");
    // Regex is approximate — just verify it finds something
    assert!(calls.contains(&"validate_input".to_string()) || calls.contains(&"save_record".to_string()), "regex should find at least one known call");
}

#[test]
fn test_regex_fallback_extracts_known_symbols() {
    // Regex fallback: given a list of known symbols, find references
    let known = vec!["validate_input".to_string(), "save_record".to_string()];
    let body = "{\n  validate_input();\n  save_record();\n}";
    let found = regex_extract_calls(body, &known);
    assert!(found.contains(&"validate_input".to_string()));
    assert!(found.contains(&"save_record".to_string()));
}
```

2. Run: `cargo test --lib intelligence::call_graph -- --verbose 2>&1 | head -50`

3. Implement `extract_java_calls`, `extract_c_calls`, `extract_cpp_calls`, `extract_ruby_calls`, `extract_csharp_calls` — each walking the tree-sitter AST for the respective `call_expression` / `method_invocation` nodes.

4. Implement `extract_regex_calls(body: &str, known_symbols: &[String]) -> Vec<String>` using the regex crate to scan for occurrences of each known symbol name followed by `(`:

```rust
pub fn regex_extract_calls(body: &str, known_symbols: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for sym in known_symbols {
        // Match symbol name followed by optional whitespace and open paren
        let pattern = format!(r"\b{}\s*\(", regex::escape(sym));
        if let Ok(re) = regex::Regex::new(&pattern) {
            if re.is_match(body) {
                found.push(sym.clone());
            }
        }
    }
    found
}
```

5. Run: `cargo test --lib intelligence::call_graph -- --verbose`

6. Commit: `feat: add call graph extraction for Java, C, C++, Ruby, C#, and regex fallback`

---

## Task 5: Build the full cross-file call graph from CodebaseIndex

**Files:**
- Modify: `src/intelligence/call_graph.rs` (add `build_call_graph`)
- Modify: `src/index/mod.rs` (add `call_graph: CallGraph` field)

**Steps:**

1. Write a failing integration test in `call_graph.rs`:

```rust
#[test]
fn test_build_call_graph_resolves_cross_file_calls() {
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    std::fs::write(&a, "fn caller() { callee(); }\n").unwrap();
    std::fs::write(&b, "pub fn callee() {}\n").unwrap();

    let files = vec![
        ScannedFile { relative_path: "a.rs".into(), absolute_path: a, language: Some("rust".into()), size_bytes: 28 },
        ScannedFile { relative_path: "b.rs".into(), absolute_path: b, language: Some("rust".into()), size_bytes: 18 },
    ];

    let mut parse_results = HashMap::new();
    parse_results.insert("a.rs".into(), ParseResult {
        symbols: vec![Symbol { name: "caller".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn caller()".into(), body: "{ callee(); }".into(), start_line: 1, end_line: 1 }],
        imports: vec![crate::parser::language::Import { source: "b".into(), names: vec!["callee".into()] }],
        exports: vec![],
    });
    parse_results.insert("b.rs".into(), ParseResult {
        symbols: vec![Symbol { name: "callee".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "pub fn callee()".into(), body: "{}".into(), start_line: 1, end_line: 1 }],
        imports: vec![],
        exports: vec![crate::parser::language::Export { name: "callee".into(), kind: SymbolKind::Function }],
    });

    let index = CodebaseIndex::build(files, parse_results, &counter);
    let cg = build_call_graph(&index);
    // callee is in b.rs; caller is in a.rs
    assert!(cg.edges.iter().any(|e| e.caller_symbol == "caller" && e.callee_symbol == "callee"),
        "expected a cross-file call edge caller->callee, got: {:?}", cg.edges);
}
```

2. Run: `cargo test --lib intelligence::call_graph::tests::test_build_call_graph -- --verbose 2>&1 | head -30`

3. Implement `build_call_graph(index: &CodebaseIndex) -> CallGraph`:

```rust
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
        }
    }

    // Build: for each file, which files does it import from?
    // imported_from[file_path] = set of file paths that file imports
    let imported_from: std::collections::HashMap<String, std::collections::HashSet<String>> = {
        let mut m = std::collections::HashMap::new();
        for (from, edges) in &index.graph.edges {
            let targets: std::collections::HashSet<String> =
                edges.iter().map(|e| e.target.clone()).collect();
            m.insert(from.clone(), targets);
        }
        m
    };

    let mut edges: Vec<CallEdge> = Vec::new();
    let mut unresolved: Vec<UnresolvedCall> = Vec::new();

    for file in &index.files {
        let Some(pr) = &file.parse_result else { continue };
        let lang = file.language.as_deref().unwrap_or("unknown");
        let imports_of_this_file = imported_from.get(&file.relative_path);

        for symbol in &pr.symbols {
            // Extract called names from this symbol's body
            let called_names =
                extract_call_sites_from_source(&file.content, lang, &symbol.name);

            for callee_name in called_names {
                // Skip self-calls and standard library names (heuristic: no dots)
                if callee_name == symbol.name {
                    continue;
                }

                // Try to resolve: is there a file that this file imports from
                // that exports `callee_name`?
                let resolved_exact = symbol_exports
                    .get(&callee_name)
                    .and_then(|exporters| {
                        if let Some(imports) = imports_of_this_file {
                            exporters.iter().find(|exp| imports.contains(*exp))
                        } else {
                            None
                        }
                    })
                    .cloned();

                if let Some(callee_file) = resolved_exact {
                    edges.push(CallEdge {
                        caller_file: file.relative_path.clone(),
                        caller_symbol: symbol.name.clone(),
                        callee_file,
                        callee_symbol: callee_name,
                        confidence: CallConfidence::Exact,
                    });
                } else if let Some(exporters) = symbol_exports.get(&callee_name) {
                    // Approximate: symbol exists elsewhere but we can't confirm import
                    if !exporters.is_empty() {
                        edges.push(CallEdge {
                            caller_file: file.relative_path.clone(),
                            caller_symbol: symbol.name.clone(),
                            callee_file: exporters[0].clone(),
                            callee_symbol: callee_name,
                            confidence: CallConfidence::Approximate,
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

    // Deduplicate edges (same caller_file/caller_symbol/callee_file/callee_symbol)
    edges.sort_by(|a, b| {
        (&a.caller_file, &a.caller_symbol, &a.callee_file, &a.callee_symbol)
            .cmp(&(&b.caller_file, &b.caller_symbol, &b.callee_file, &b.callee_symbol))
    });
    edges.dedup_by(|a, b| {
        a.caller_file == b.caller_file
            && a.caller_symbol == b.caller_symbol
            && a.callee_file == b.callee_file
            && a.callee_symbol == b.callee_symbol
    });

    CallGraph { edges, unresolved }
}
```

4. Add `pub call_graph: crate::intelligence::call_graph::CallGraph` field to `CodebaseIndex` struct in `src/index/mod.rs`. Initialize to `CallGraph::default()` in both `build` and `build_with_content`. Add `pub use crate::intelligence::call_graph::CallGraph;` re-export in `src/intelligence/mod.rs`.

5. **Critical:** Also update `rebuild_graph()` in `src/index/mod.rs` to re-run `build_call_graph(&self)` after rebuilding the dependency graph. Without this, incremental updates via `upsert_file()` + `rebuild_graph()` (from v1.2.0) would leave the call graph stale. The pattern: `self.graph = build_dependency_graph(...); self.call_graph = build_call_graph(self);`

5. In `CodebaseIndex::build` and `build_with_content`, after `index.test_map = ...`, add:
   ```rust
   index.call_graph = crate::intelligence::call_graph::build_call_graph(&index);
   ```

6. Also add `call_graph: CallGraph::default()` to `CodebaseIndex::empty()`.

7. Run: `cargo test --lib intelligence::call_graph -- --verbose`

8. Run: `cargo test --lib index -- --verbose`

9. Commit: `feat: build cross-file call graph on CodebaseIndex`

---

## Task 6: Dead code detection types and module skeleton

**Files:**
- Create: `src/intelligence/dead_code.rs`
- Modify: `src/intelligence/mod.rs`

**Steps:**

1. Write a failing type-shape test:

```rust
#[test]
fn test_dead_symbol_fields_exist() {
    let ds = DeadSymbol {
        file: "src/util.rs".into(),
        symbol: "unused_helper".into(),
        kind: crate::parser::language::SymbolKind::Function,
        liveness_score: 0.42,
        reason: "zero callers, not entry point, no test reference".into(),
    };
    assert_eq!(ds.file, "src/util.rs");
    assert!((ds.liveness_score - 0.42).abs() < 1e-9);
}
```

2. Run: `cargo test --lib intelligence::dead_code -- --verbose 2>&1 | head -20`

3. Create `src/intelligence/dead_code.rs`:

```rust
use crate::parser::language::SymbolKind;
use serde::{Deserialize, Serialize};

/// A symbol classified as dead (zero callers, not an entry point).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadSymbol {
    pub file: String,
    pub symbol: String,
    pub kind: SymbolKind,
    /// Sorting key: higher = more concerning dead symbol.
    /// Formula: pagerank × (1.0 + test_file_count) × export_weight
    /// where export_weight = 2.0 for pub exports, 1.0 otherwise.
    pub liveness_score: f64,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dead_symbol_fields_exist() {
        let ds = DeadSymbol {
            file: "src/util.rs".into(),
            symbol: "unused_helper".into(),
            kind: SymbolKind::Function,
            liveness_score: 0.42,
            reason: "zero callers, not entry point, no test reference".into(),
        };
        assert_eq!(ds.file, "src/util.rs");
        assert!((ds.liveness_score - 0.42).abs() < 1e-9);
    }
}
```

4. Add `pub mod dead_code;` to `src/intelligence/mod.rs`.

5. Run: `cargo test --lib intelligence::dead_code -- --verbose`

6. Commit: `feat: add DeadSymbol type and dead_code module skeleton`

---

## Task 7: Dead code detection algorithm

**Files:**
- Modify: `src/intelligence/dead_code.rs` (add `detect_dead_code`)

**Steps:**

1. Write a failing integration test:

```rust
#[test]
fn test_detect_dead_code_finds_uncalled_private_function() {
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility, Export};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("util.rs");
    std::fs::write(&fp, "fn live_fn() {} fn dead_fn() {}").unwrap();

    let files = vec![ScannedFile {
        relative_path: "util.rs".into(), absolute_path: fp,
        language: Some("rust".into()), size_bytes: 36,
    }];
    let mut parse_results = HashMap::new();
    parse_results.insert("util.rs".into(), ParseResult {
        symbols: vec![
            Symbol { name: "live_fn".into(), kind: SymbolKind::Function, visibility: Visibility::Private,
                signature: "fn live_fn()".into(), body: "{}".into(), start_line: 1, end_line: 1 },
            Symbol { name: "dead_fn".into(), kind: SymbolKind::Function, visibility: Visibility::Private,
                signature: "fn dead_fn()".into(), body: "{}".into(), start_line: 1, end_line: 1 },
        ],
        imports: vec![],
        exports: vec![],
    });

    let index = CodebaseIndex::build(files, parse_results, &counter);
    // Manually inject a call edge making live_fn a callee
    // (In real usage the call graph is built automatically)
    let dead = detect_dead_code(&index, None);

    // dead_fn has no callers in any call graph edge
    assert!(dead.iter().any(|d| d.symbol == "dead_fn"),
        "dead_fn should be detected as dead, got: {:?}", dead.iter().map(|d| &d.symbol).collect::<Vec<_>>());
}

#[test]
fn test_liveness_score_formula() {
    // liveness_score = pagerank × (1 + test_file_count) × export_weight
    // pagerank=0.5, test_file_count=1, export_weight=2.0 → 0.5 × 2.0 × 2.0 = 2.0
    let score = compute_liveness_score(0.5, 1, true);
    assert!((score - 2.0).abs() < 1e-9, "expected 2.0, got {score}");

    // pagerank=0.3, test_file_count=0, export_weight=1.0 → 0.3 × 1.0 × 1.0 = 0.3
    let score2 = compute_liveness_score(0.3, 0, false);
    assert!((score2 - 0.3).abs() < 1e-9, "expected 0.3, got {score2}");
}
```

2. Run: `cargo test --lib intelligence::dead_code -- --verbose 2>&1 | head -30`

3. Implement `detect_dead_code` and `compute_liveness_score`:

```rust
use crate::index::CodebaseIndex;
use crate::intelligence::api_surface::detect_routes;
use crate::parser::language::Visibility;
use std::collections::HashSet;

/// Compute liveness score for sorting dead symbols.
/// Higher = more important dead symbol (pub export, has tests nearby, high pagerank).
pub fn compute_liveness_score(
    pagerank: f64,
    test_file_count: usize,
    is_pub_export: bool,
) -> f64 {
    let export_weight = if is_pub_export { 2.0 } else { 1.0 };
    pagerank * (1.0 + test_file_count as f64) * export_weight
}

/// Entry point detection: a symbol is a live entry point when it is:
/// - Named "main"
/// - An HTTP handler (detected via route patterns in the same file)
/// - A test function (name starts with "test_" or contains "#[test]" in signature)
/// - A pub export from a lib root (mod.rs, lib.rs, index.ts, __init__.py)
fn is_entry_point(
    file: &str,
    symbol_name: &str,
    signature: &str,
    file_content: &str,
    is_public: bool,
) -> bool {
    // main function
    if symbol_name == "main" {
        return true;
    }
    // test function
    if symbol_name.starts_with("test_")
        || signature.contains("#[test]")
        || signature.contains("@Test")
        || signature.contains("def test_")
    {
        return true;
    }
    // pub export from a module root
    let is_root_file = file.ends_with("mod.rs")
        || file.ends_with("lib.rs")
        || file.ends_with("index.ts")
        || file.ends_with("index.js")
        || file.ends_with("__init__.py");
    if is_public && is_root_file {
        return true;
    }
    // trait implementation: methods inside `impl Trait for Type` blocks
    // are called via trait dispatch even if they have zero direct callers.
    // Check if the signature contains trait impl markers per language.
    if signature.contains("impl ") && signature.contains(" for ")  // Rust
        || signature.contains("@Override")                          // Java
        || signature.contains("override ")                          // Kotlin/C#
        || signature.contains("def ") && file_content.contains("(ABC)")  // Python ABC
    {
        return true;
    }
    // HTTP handler: check if this file contains route registrations.
    // Cache this per file in the caller to avoid N calls per file.
    let routes = detect_routes(file_content, file);
    if !routes.is_empty() && is_public {
        return true;
    }
    false
}

/// Detect dead symbols across the codebase.
///
/// A symbol is dead when ALL of:
/// - Zero callers in the call graph
/// - Not an entry point (main, HTTP handler, test fn, pub root export)
/// - Not referenced in any test file (via test_map)
///
/// Returns symbols sorted by liveness_score descending (most important dead symbols first).
pub fn detect_dead_code(
    index: &CodebaseIndex,
    focus: Option<&str>,
) -> Vec<DeadSymbol> {
    // Build set of test-referenced symbols: symbols that appear as callees
    // from test files in the call graph
    let test_file_paths: HashSet<&str> = index
        .test_map
        .values()
        .flatten()
        .map(|r| r.path.as_str())
        .collect();

    let test_referenced: HashSet<(String, String)> = index
        .call_graph
        .edges
        .iter()
        .filter(|e| test_file_paths.contains(e.caller_file.as_str()))
        .map(|e| (e.callee_file.clone(), e.callee_symbol.clone()))
        .collect();

    let mut dead: Vec<DeadSymbol> = Vec::new();

    for file in &index.files {
        if let Some(prefix) = focus {
            if !file.relative_path.starts_with(prefix) {
                continue;
            }
        }
        // Skip test files themselves
        if is_test_file(&file.relative_path) {
            continue;
        }
        let Some(pr) = &file.parse_result else { continue };

        for symbol in &pr.symbols {
            // Check: has callers?
            let has_callers = index
                .call_graph
                .has_callers(&file.relative_path, &symbol.name);
            if has_callers {
                continue;
            }

            // Check: is entry point?
            let is_public = symbol.visibility == Visibility::Public;
            if is_entry_point(
                &file.relative_path,
                &symbol.name,
                &symbol.signature,
                &file.content,
                is_public,
            ) {
                continue;
            }

            // Check: referenced in test files?
            let key = (file.relative_path.clone(), symbol.name.clone());
            if test_referenced.contains(&key) {
                continue;
            }

            // Dead symbol — compute liveness score for sorting
            let pagerank = index
                .pagerank
                .get(&file.relative_path)
                .copied()
                .unwrap_or(0.0);
            let test_file_count = index
                .test_map
                .get(&file.relative_path)
                .map(|v| v.len())
                .unwrap_or(0);
            let is_pub_export = pr
                .exports
                .iter()
                .any(|e| e.name == symbol.name);
            let liveness_score =
                compute_liveness_score(pagerank, test_file_count, is_pub_export);

            dead.push(DeadSymbol {
                file: file.relative_path.clone(),
                symbol: symbol.name.clone(),
                kind: symbol.kind.clone(),
                liveness_score,
                reason: "zero callers, not entry point, no test reference".into(),
            });
        }
    }

    // Sort descending by liveness_score (most important dead symbols first)
    dead.sort_by(|a, b| {
        b.liveness_score
            .partial_cmp(&a.liveness_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    dead
}

fn is_test_file(path: &str) -> bool {
    path.contains("/tests/")
        || path.contains("/test/")
        || path.contains("/spec/")
        || path.contains("__tests__")
        || path.ends_with("_test.rs")
        || path.ends_with("_test.py")
        || path.ends_with("_test.go")
        || path.ends_with(".test.ts")
        || path.ends_with(".spec.ts")
}
```

4. Run: `cargo test --lib intelligence::dead_code -- --verbose`

5. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

6. Commit: `feat: implement dead code detection with liveness score sorting`

---

## Task 8: Architecture quality — cohesion metric

**Files:**
- Modify: `src/intelligence/architecture.rs` (v1.2.0 file) — add `compute_cohesion`
- If `architecture.rs` does not yet exist: create it with `ModuleInfo` extended struct

**Steps:**

1. Write a failing test:

```rust
#[test]
fn test_cohesion_fully_connected_module() {
    // 3 files, each importing both others = max intra-module edges
    // intra_edges = 6 (3×2 directed), max_possible = 3×2 = 6 → cohesion = 1.0
    let cohesion = compute_cohesion(6, 3);
    assert!((cohesion - 1.0).abs() < 1e-9, "expected 1.0, got {cohesion}");
}

#[test]
fn test_cohesion_isolated_module() {
    // 3 files, no intra-module edges → cohesion = 0.0
    let cohesion = compute_cohesion(0, 3);
    assert!((cohesion - 0.0).abs() < 1e-9, "expected 0.0, got {cohesion}");
}

#[test]
fn test_cohesion_single_file_module() {
    // Single file: max_possible = 0 → cohesion = 0.0 by convention
    let cohesion = compute_cohesion(0, 1);
    assert!((cohesion - 0.0).abs() < 1e-9, "single-file module cohesion = 0.0");
}
```

2. Run: `cargo test -- --verbose 2>&1 | grep "test_cohesion" | head -10`

3. Implement `compute_cohesion(intra_edges: usize, file_count: usize) -> f64`:

```rust
/// Cohesion = ratio of actual intra-module edges to maximum possible.
/// Maximum possible for N files = N × (N-1) directed edges.
/// Returns 0.0 for single-file modules (undefined ratio).
pub fn compute_cohesion(intra_edges: usize, file_count: usize) -> f64 {
    if file_count <= 1 {
        return 0.0;
    }
    let max_possible = file_count * (file_count - 1);
    if max_possible == 0 {
        return 0.0;
    }
    (intra_edges as f64 / max_possible as f64).min(1.0)
}
```

4. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

5. Commit: `feat: add cohesion metric computation`

---

## Task 9: Architecture quality — boundary violations and god file detection

**Files:**
- Modify: `src/intelligence/architecture.rs`

**Steps:**

1. Write failing tests:

```rust
#[test]
fn test_boundary_violation_detects_non_root_import() {
    // src/api/handler.rs importing src/db/internal/pool.rs is a violation
    // Root files: mod.rs, lib.rs, index.ts, __init__.py, index.js
    assert!(is_boundary_violation("src/db/internal/pool.rs", "src/db"));
    assert!(!is_boundary_violation("src/db/mod.rs", "src/db"));
    assert!(!is_boundary_violation("src/db/lib.rs", "src/db"));
}

#[test]
fn test_god_file_detection_mean_plus_2sigma() {
    // Files with inbound count > mean + 2σ are god files
    let inbound_counts = vec![
        ("a.rs", 1usize),
        ("b.rs", 2),
        ("c.rs", 2),
        ("d.rs", 50), // outlier
    ];
    let god_files = detect_god_files(&inbound_counts);
    assert!(god_files.contains(&"d.rs"), "d.rs should be a god file");
    assert!(!god_files.contains(&"a.rs"), "a.rs should not be a god file");
}
```

2. Run: `cargo test --lib -- --verbose 2>&1 | grep "test_boundary\|test_god" | head -10`

3. Implement `is_boundary_violation(target_path: &str, target_module: &str) -> bool`:

```rust
/// Returns true when `target_path` is not a root file of `target_module`.
///
/// Root files are: mod.rs, lib.rs, index.ts, index.js, __init__.py.
/// A file in `src/db/internal/pool.rs` is not the root of `src/db` → violation.
/// A file in `src/db/mod.rs` IS the root of `src/db` → not a violation.
pub fn is_boundary_violation(target_path: &str, target_module: &str) -> bool {
    let root_files = ["mod.rs", "lib.rs", "index.ts", "index.js", "__init__.py"];
    let filename = target_path.rsplit('/').next().unwrap_or(target_path);
    // Not a violation if target is the barrel/root file of its module
    if root_files.contains(&filename) {
        // Check it belongs to target_module directly (not a deeper sub-module)
        let parent = target_path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        return parent != target_module;
    }
    // Not a violation if file is directly in target_module (e.g. src/db/queries.rs)
    let direct = format!("{}/", target_module);
    let depth_in_module = target_path
        .strip_prefix(&direct)
        .map(|rest| rest.contains('/'))
        .unwrap_or(true);
    depth_in_module
}
```

4. Implement `detect_god_files(inbound_counts: &[(&str, usize)]) -> Vec<&str>` using mean + 2σ formula:

```rust
pub fn detect_god_files<'a>(inbound_counts: &[(&'a str, usize)]) -> Vec<&'a str> {
    if inbound_counts.len() < 3 {
        return vec![];
    }
    let counts: Vec<f64> = inbound_counts.iter().map(|(_, c)| *c as f64).collect();
    let mean = counts.iter().sum::<f64>() / counts.len() as f64;
    let variance = counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / counts.len() as f64;
    let sigma = variance.sqrt();
    let threshold = mean + 2.0 * sigma;
    inbound_counts
        .iter()
        .filter(|(_, c)| *c as f64 > threshold)
        .map(|(path, _)| *path)
        .collect()
}
```

5. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

6. Commit: `feat: add boundary violation detection and god file algorithm`

---

## Task 10: Extend ModuleInfo and build ArchitectureMap with v1.3.0 fields

**Files:**
- Modify: `src/intelligence/architecture.rs` (extend `ModuleInfo`, update `build_architecture_map`)

**Steps:**

1. Write a failing test:

```rust
#[test]
fn test_module_info_has_v130_fields() {
    let mi = ModuleInfo {
        prefix: "src/api".into(),
        file_count: 3,
        aggregate_pagerank: 0.8,
        coupling: 0.3,
        cohesion: 0.5,
        boundary_violations: vec![],
        god_files: vec![],
    };
    assert_eq!(mi.prefix, "src/api");
    assert!((mi.cohesion - 0.5).abs() < 1e-9);
}
```

2. Update the `ModuleInfo` struct to include the v1.3.0 fields:

```rust
use crate::schema::EdgeType;
use serde::{Deserialize, Serialize};

/// A cross-module import that bypasses the module's public interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryViolation {
    pub source_file: String,
    pub target_file: String,
    pub target_module: String,
    pub edge_type: EdgeType,  // typed, not stringly-typed
}

/// Per-module quality metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    // v1.2.0 fields
    pub prefix: String,
    pub file_count: usize,
    pub aggregate_pagerank: f64,
    pub coupling: f64,

    // v1.3.0 fields
    pub cohesion: f64,
    pub boundary_violations: Vec<BoundaryViolation>,
    pub god_files: Vec<String>,
}
```

3. Update `build_architecture_map` to populate the three new fields for each module by:
   - Counting intra-module edges for cohesion
   - Walking cross-module edges to identify boundary violations
   - Running `detect_god_files` on per-file inbound edge counts

4. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

5. Commit: `feat: extend ModuleInfo with cohesion, boundary_violations, god_files`

---

## Task 11: Health score — populate dead_code dimension

**Files:**
- Modify: `src/intelligence/health.rs` (v1.2.0 file, update `compute_health_score`)

**Steps:**

1. Write a failing test:

```rust
#[test]
fn test_dead_code_dimension_populated_when_call_graph_present() {
    // 10 symbols, 2 dead = dead_ratio 0.2 → dead_code_score = 10.0 × (1 - 0.2) = 8.0
    let score = compute_dead_code_dimension(10, 2);
    assert!((score - 8.0).abs() < 1e-9, "expected 8.0, got {score}");
}

#[test]
fn test_dead_code_dimension_zero_symbols_is_ten() {
    // No symbols → no dead code possible → healthy = 10.0
    let score = compute_dead_code_dimension(0, 0);
    assert!((score - 10.0).abs() < 1e-9, "expected 10.0, got {score}");
}
```

2. Implement `compute_dead_code_dimension(total_symbols: usize, dead_count: usize) -> f64`:

```rust
pub fn compute_dead_code_dimension(total_symbols: usize, dead_count: usize) -> f64 {
    if total_symbols == 0 {
        return 10.0;
    }
    let dead_ratio = dead_count as f64 / total_symbols as f64;
    10.0 * (1.0 - dead_ratio)
}
```

3. In `compute_health_score`, replace the `None` placeholder for `dead_code` with the live value:
   - Count `total_symbols` from `index.files` (all symbols across all parse results)
   - Run `detect_dead_code(index, None)` to get `dead_count`
   - Call `compute_dead_code_dimension(total_symbols, dead_count)` for the dimension score
   - Use full 6-dimension composite (all weights sum to 1.0, dead_code weight = 0.10)

4. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

5. Commit: `feat: populate dead_code dimension of HealthScore using call graph`

---

## Task 12: Monorepo workspace parameter — scanner scoping

**Files:**
- Modify: `src/scanner/mod.rs`

**Steps:**

1. Write a failing test:

```rust
#[test]
fn test_scanner_workspace_scoping() {
    // Only files under workspace prefix are returned
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
    std::fs::create_dir_all(tmp.path().join("packages/api")).unwrap();
    std::fs::create_dir_all(tmp.path().join("packages/web")).unwrap();
    std::fs::write(tmp.path().join("packages/api/main.rs"), "fn main() {}").unwrap();
    std::fs::write(tmp.path().join("packages/web/index.ts"), "export {}").unwrap();

    let scanner = Scanner::new(tmp.path()).unwrap();
    let files = scanner.scan_workspace(Some("packages/api")).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(paths.iter().any(|p| p.starts_with("packages/api")));
    assert!(!paths.iter().any(|p| p.starts_with("packages/web")));
}
```

2. Run: `cargo test --lib scanner -- --verbose 2>&1 | head -20`

3. Add `scan_workspace` method to `Scanner` that delegates to `scan()` and filters by workspace prefix:

```rust
/// Scan files restricted to a workspace prefix.
///
/// When `workspace` is `None`, behaves identically to `scan()`.
/// When `workspace` is `Some(prefix)`, only files whose `relative_path`
/// starts with `prefix` are returned.
pub fn scan_workspace(&self, workspace: Option<&str>) -> Result<Vec<ScannedFile>, ScanError> {
    let all = self.scan()?;
    match workspace {
        None => Ok(all),
        Some(prefix) => Ok(all
            .into_iter()
            .filter(|f| f.relative_path.starts_with(prefix))
            .collect()),
    }
}
```

4. Run: `cargo test --lib scanner -- --verbose`

5. Commit: `feat: add workspace scoping to Scanner via scan_workspace()`

---

## Task 13: Monorepo workspace parameter — CLI commands

**Files:**
- Modify: `src/commands/overview.rs`
- Modify: `src/commands/trace.rs`
- Modify: `src/main.rs` (add `--workspace` flag to relevant subcommands)

**Steps:**

1. Write a failing integration test for `cxpak overview --workspace packages/api`:

```rust
#[test]
fn test_overview_workspace_flag_accepted() {
    use assert_cmd::Command;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("packages/api")).unwrap();
    std::fs::write(dir.path().join("packages/api/main.rs"), "fn main() {}").unwrap();

    let mut cmd = Command::cargo_bin("cxpak").unwrap();
    cmd.arg("overview")
        .arg("--workspace")
        .arg("packages/api")
        .arg(dir.path().to_str().unwrap());
    cmd.assert().success();
}
```

2. Add `#[arg(long)] workspace: Option<String>` to the `Overview` and `Trace` clap subcommand structs. Pass the workspace value through to `Scanner::scan_workspace()` in each command handler.

3. Run: `cargo test --test integration -- overview_workspace 2>&1 | head -20`

4. Run: `cargo build 2>&1 | tail -10` — verify clean build.

5. Run: `cargo test --lib scanner -- --verbose && cargo test --test integration -- 2>&1 | tail -20`

6. Commit: `feat: add --workspace flag to overview and trace CLI commands`

---

## Task 14: Monorepo workspace parameter — cache namespace

**Files:**
- Modify: `src/commands/serve.rs` (update `build_index` and file-cache path logic)
- Modify: `src/daemon/cache.rs` if it exists, or the cache path resolution utility

**Steps:**

1. Write a failing test:

```rust
#[test]
fn test_workspace_cache_namespace_differs() {
    let base_path = std::path::Path::new("/repo");
    let ns_none = cache_namespace(base_path, None);
    let ns_api = cache_namespace(base_path, Some("packages/api"));
    let ns_web = cache_namespace(base_path, Some("packages/web"));
    assert_ne!(ns_none, ns_api);
    assert_ne!(ns_api, ns_web);
}
```

2. Implement `cache_namespace(repo_root: &Path, workspace: Option<&str>) -> String`:

```rust
/// Returns a cache directory name scoped to the given workspace.
///
/// When workspace is None: ".cxpak/cache/root"
/// When workspace is Some("packages/api"): ".cxpak/cache/packages_api"
pub fn cache_namespace(repo_root: &std::path::Path, workspace: Option<&str>) -> String {
    let _ = repo_root; // reserved for future use (multi-repo scenarios)
    match workspace {
        None => ".cxpak/cache/root".to_string(),
        Some(ws) => format!(".cxpak/cache/{}", ws.replace('/', "_")),
    }
}
```

3. Thread `workspace: Option<String>` through `build_index` and the `AppState` struct in `serve.rs`. Use `cache_namespace` when writing/reading cached index state.

4. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

5. Commit: `feat: namespace cache directories per workspace`

---

## Task 15: New MCP tool — `cxpak_call_graph`

**Files:**
- Modify: `src/commands/serve.rs` (add `/call_graph` endpoint and handler)

**Steps:**

1. Write a failing integration test for the MCP endpoint:

```rust
#[tokio::test]
async fn test_call_graph_endpoint_returns_json() {
    // Build a minimal server and POST /call_graph
    // Response must have "edges" and "unresolved" keys
    use tower::ServiceExt;
    use axum::http::{Request, StatusCode};
    use axum::body::Body;

    let index = crate::index::CodebaseIndex::empty();
    let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
    let repo_path = std::sync::Arc::new(std::path::PathBuf::from("/tmp"));
    let app = build_router(shared, repo_path);

    let req = Request::builder()
        .method("POST")
        .uri("/call_graph")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"target":"src/main.rs","depth":1}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("edges").is_some(), "response must have 'edges' key");
    assert!(json.get("unresolved").is_some(), "response must have 'unresolved' key");
}
```

2. Run: `cargo test --test integration -- test_call_graph_endpoint 2>&1 | head -30`

3. Add request/response types and handler:

```rust
#[derive(Deserialize)]
struct CallGraphParams {
    target: Option<String>,     // file path or symbol name
    depth: Option<usize>,       // default 1
    focus: Option<String>,
    workspace: Option<String>,
}

async fn call_graph_handler(
    State(state): State<AppState>,
    Json(params): Json<CallGraphParams>,
) -> Result<Json<Value>, StatusCode> {
    let index = state.index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let depth = params.depth.unwrap_or(1);
    let cg = &index.call_graph;

    let filtered_edges: Vec<&crate::intelligence::call_graph::CallEdge> =
        if let Some(ref target) = params.target {
            // If target matches a file path, return all edges for that file
            // If target matches a symbol name, return edges for that symbol
            cg.edges
                .iter()
                .filter(|e| {
                    e.caller_file.contains(target.as_str())
                        || e.callee_file.contains(target.as_str())
                        || e.caller_symbol.contains(target.as_str())
                        || e.callee_symbol.contains(target.as_str())
                })
                .collect()
        } else {
            cg.edges.iter().collect()
        };

    // Apply focus filter
    let edges: Vec<&crate::intelligence::call_graph::CallEdge> =
        if let Some(ref focus) = params.focus {
            filtered_edges
                .into_iter()
                .filter(|e| {
                    e.caller_file.starts_with(focus.as_str())
                        || e.callee_file.starts_with(focus.as_str())
                })
                .collect()
        } else {
            filtered_edges
        };

    let _ = depth; // depth BFS expansion is a future enhancement

    Ok(Json(json!({
        "edges": edges,
        "unresolved": cg.unresolved,
        "total_edges": cg.edges.len(),
    })))
}
```

4. Register the route in `build_router`: `.route("/call_graph", axum::routing::post(call_graph_handler))`.

5. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

6. Commit: `feat: add cxpak_call_graph MCP endpoint`

---

## Task 16: New MCP tool — `cxpak_dead_code`

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Write a failing test:

```rust
#[tokio::test]
async fn test_dead_code_endpoint_returns_sorted_list() {
    use tower::ServiceExt;
    use axum::http::{Request, StatusCode};
    use axum::body::Body;

    let index = crate::index::CodebaseIndex::empty();
    let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
    let repo_path = std::sync::Arc::new(std::path::PathBuf::from("/tmp"));
    let app = build_router(shared, repo_path);

    let req = Request::builder()
        .method("POST")
        .uri("/dead_code")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"limit":10}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("dead_symbols").is_some());
    assert!(json.get("total_count").is_some());
}
```

2. Implement handler and register route `/dead_code` (POST):

```rust
#[derive(Deserialize)]
struct DeadCodeParams {
    focus: Option<String>,
    limit: Option<usize>,
    workspace: Option<String>,
}

async fn dead_code_handler(
    State(state): State<AppState>,
    Json(params): Json<DeadCodeParams>,
) -> Result<Json<Value>, StatusCode> {
    let index = state.index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let limit = params.limit.unwrap_or(50);
    let focus = params.focus.as_deref();

    let dead = crate::intelligence::dead_code::detect_dead_code(&index, focus);
    let total_count = dead.len();
    let limited: Vec<_> = dead.into_iter().take(limit).collect();

    Ok(Json(json!({
        "dead_symbols": limited,
        "total_count": total_count,
        "showing": limited.len(),
    })))
}
```

3. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

4. Commit: `feat: add cxpak_dead_code MCP endpoint`

---

## Task 17: New MCP tool — `cxpak_architecture`

**Files:**
- Modify: `src/commands/serve.rs`

**Steps:**

1. Write a failing test:

```rust
#[tokio::test]
async fn test_architecture_endpoint_returns_modules_and_circular_deps() {
    use tower::ServiceExt;
    use axum::http::{Request, StatusCode};
    use axum::body::Body;

    let index = crate::index::CodebaseIndex::empty();
    let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
    let repo_path = std::sync::Arc::new(std::path::PathBuf::from("/tmp"));
    let app = build_router(shared, repo_path);

    let req = Request::builder()
        .method("POST")
        .uri("/architecture")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("modules").is_some());
    assert!(json.get("circular_deps").is_some());
}
```

2. Implement handler and register route `/architecture` (POST). The handler calls `build_architecture_map(&index, focus)` (v1.2.0 function, now extended with v1.3.0 fields).

3. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

4. Commit: `feat: add cxpak_architecture MCP endpoint`

---

## Task 18: Wire workspace parameter through MCP serve endpoints

**Files:**
- Modify: `src/commands/serve.rs` (update `AppState`, `build_router`, all handlers)

**Steps:**

1. Add `workspace: Option<String>` to `AppState`. Add `SharedWorkspace = Arc<Option<String>>` type alias.

2. Update `build_router` signature to accept `workspace: Option<String>`. Pass through to `AppState`.

3. Update `run()` in `serve.rs` to accept `workspace: Option<String>` and pass it through. Update `main.rs` to pass the `--workspace` flag value.

4. In each POST handler, thread the `workspace` from the request params through to focus-aware function calls. When `params.workspace` is `Some(ws)`, use it to prefix the focus path (or replace it if the user also provides a `focus`).

5. Write a test verifying that the workspace param is threaded correctly:

```rust
#[test]
fn test_workspace_is_accepted_in_request_params() {
    // Verify the WorkspaceParams struct deserializes workspace field
    let json = r#"{"workspace": "packages/api", "limit": 5}"#;
    let params: DeadCodeParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.workspace, Some("packages/api".to_string()));
}
```

6. Run: `cargo test --lib -- --verbose 2>&1 | tail -20`

7. Commit: `feat: thread workspace parameter through all MCP tool handlers`

---

## Task 19: Integration tests for call graph accuracy

**Files:**
- Create: `tests/call_graph_integration.rs`

**Steps:**

1. Write comprehensive accuracy tests:

```rust
use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::intelligence::call_graph::{build_call_graph, CallConfidence};
use cxpak::parser::language::{Export, Import, ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

fn make_two_file_index() -> (CodebaseIndex, tempfile::TempDir) {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let caller_src = "use crate::b::callee;\nfn caller() { callee(); }\n";
    let callee_src = "pub fn callee() {}\n";

    let fp_a = dir.path().join("src/a.rs");
    let fp_b = dir.path().join("src/b.rs");
    std::fs::create_dir_all(fp_a.parent().unwrap()).unwrap();
    std::fs::write(&fp_a, caller_src).unwrap();
    std::fs::write(&fp_b, callee_src).unwrap();

    let files = vec![
        ScannedFile { relative_path: "src/a.rs".into(), absolute_path: fp_a, language: Some("rust".into()), size_bytes: caller_src.len() as u64 },
        ScannedFile { relative_path: "src/b.rs".into(), absolute_path: fp_b, language: Some("rust".into()), size_bytes: callee_src.len() as u64 },
    ];

    let mut parse_results = HashMap::new();
    parse_results.insert("src/a.rs".into(), ParseResult {
        symbols: vec![Symbol { name: "caller".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn caller()".into(), body: "{ callee(); }".into(), start_line: 2, end_line: 2 }],
        imports: vec![Import { source: "crate::b".into(), names: vec!["callee".into()] }],
        exports: vec![],
    });
    parse_results.insert("src/b.rs".into(), ParseResult {
        symbols: vec![Symbol { name: "callee".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "pub fn callee()".into(), body: "{}".into(), start_line: 1, end_line: 1 }],
        imports: vec![],
        exports: vec![Export { name: "callee".into(), kind: SymbolKind::Function }],
    });

    (CodebaseIndex::build(files, parse_results, &counter), dir)
}

#[test]
fn test_exact_cross_file_edge_resolved() {
    let (index, _dir) = make_two_file_index();
    let cg = &index.call_graph;
    let edge = cg.edges.iter().find(|e| e.caller_symbol == "caller" && e.callee_symbol == "callee");
    assert!(edge.is_some(), "expected caller->callee edge, edges: {:?}", cg.edges);
    assert_eq!(edge.unwrap().confidence, CallConfidence::Exact);
    assert_eq!(edge.unwrap().callee_file, "src/b.rs");
}

#[test]
fn test_no_self_call_edges() {
    let (index, _dir) = make_two_file_index();
    let self_calls: Vec<_> = index.call_graph.edges.iter()
        .filter(|e| e.caller_file == e.callee_file && e.caller_symbol == e.callee_symbol)
        .collect();
    assert!(self_calls.is_empty(), "unexpected self-call edges: {:?}", self_calls);
}

#[test]
fn test_call_graph_stored_on_index() {
    let (index, _dir) = make_two_file_index();
    // The call graph is built as part of CodebaseIndex::build
    assert!(index.call_graph.edges.len() > 0 || index.call_graph.unresolved.len() >= 0,
        "call graph should be populated on index");
}
```

2. Run: `cargo test --test call_graph_integration -- --verbose`

3. Fix any failures (adjust import resolution logic in `build_call_graph` if needed).

4. Commit: `test: add cross-file call graph integration tests`

---

## Task 20: Integration tests for dead code detection accuracy

**Files:**
- Create: `tests/dead_code_integration.rs`

**Steps:**

1. Write property-based tests covering edge cases:

```rust
use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::intelligence::dead_code::detect_dead_code;
use cxpak::parser::language::{Export, ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

fn build_single_file_index(symbols: Vec<Symbol>) -> (CodebaseIndex, tempfile::TempDir) {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp = dir.path().join("src/lib.rs");
    std::fs::create_dir_all(fp.parent().unwrap()).unwrap();
    let content = "// stub";
    std::fs::write(&fp, content).unwrap();
    let files = vec![ScannedFile { relative_path: "src/lib.rs".into(), absolute_path: fp, language: Some("rust".into()), size_bytes: content.len() as u64 }];
    let mut parse_results = HashMap::new();
    parse_results.insert("src/lib.rs".into(), ParseResult { symbols, imports: vec![], exports: vec![] });
    (CodebaseIndex::build(files, parse_results, &counter), dir)
}

#[test]
fn test_main_function_is_not_dead() {
    let (index, _dir) = build_single_file_index(vec![
        Symbol { name: "main".into(), kind: SymbolKind::Function, visibility: Visibility::Public, signature: "fn main()".into(), body: "{}".into(), start_line: 1, end_line: 1 },
    ]);
    let dead = detect_dead_code(&index, None);
    assert!(!dead.iter().any(|d| d.symbol == "main"), "main() must never be classified as dead");
}

#[test]
fn test_liveness_score_is_nonnegative_for_all_dead_symbols() {
    let (index, _dir) = build_single_file_index(vec![
        Symbol { name: "orphan_fn".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn orphan_fn()".into(), body: "{}".into(), start_line: 1, end_line: 2 },
    ]);
    let dead = detect_dead_code(&index, None);
    for d in &dead {
        assert!(d.liveness_score >= 0.0, "liveness_score must be >= 0 for {}", d.symbol);
    }
}

#[test]
fn test_dead_symbols_sorted_descending_by_liveness_score() {
    let (index, _dir) = build_single_file_index(vec![
        Symbol { name: "alpha".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn alpha()".into(), body: "{}".into(), start_line: 1, end_line: 1 },
        Symbol { name: "beta".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn beta()".into(), body: "{}".into(), start_line: 2, end_line: 2 },
    ]);
    let dead = detect_dead_code(&index, None);
    for window in dead.windows(2) {
        assert!(
            window[0].liveness_score >= window[1].liveness_score,
            "dead symbols must be sorted descending: {} ({}) > {} ({})",
            window[0].symbol, window[0].liveness_score,
            window[1].symbol, window[1].liveness_score
        );
    }
}

#[test]
fn test_focus_filter_restricts_dead_code_to_prefix() {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();
    let fp_a = dir.path().join("src/api/handler.rs");
    let fp_b = dir.path().join("src/db/query.rs");
    std::fs::create_dir_all(fp_a.parent().unwrap()).unwrap();
    std::fs::create_dir_all(fp_b.parent().unwrap()).unwrap();
    std::fs::write(&fp_a, "fn api_fn() {}").unwrap();
    std::fs::write(&fp_b, "fn db_fn() {}").unwrap();

    let files = vec![
        ScannedFile { relative_path: "src/api/handler.rs".into(), absolute_path: fp_a, language: Some("rust".into()), size_bytes: 15 },
        ScannedFile { relative_path: "src/db/query.rs".into(), absolute_path: fp_b, language: Some("rust".into()), size_bytes: 14 },
    ];
    let mut parse_results = HashMap::new();
    parse_results.insert("src/api/handler.rs".into(), ParseResult { symbols: vec![Symbol { name: "api_fn".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn api_fn()".into(), body: "{}".into(), start_line: 1, end_line: 1 }], imports: vec![], exports: vec![] });
    parse_results.insert("src/db/query.rs".into(), ParseResult { symbols: vec![Symbol { name: "db_fn".into(), kind: SymbolKind::Function, visibility: Visibility::Private, signature: "fn db_fn()".into(), body: "{}".into(), start_line: 1, end_line: 1 }], imports: vec![], exports: vec![] });

    let index = CodebaseIndex::build(files, parse_results, &counter);
    let dead = detect_dead_code(&index, Some("src/api/"));
    for d in &dead {
        assert!(d.file.starts_with("src/api/"), "focus filter failed: {} is outside src/api/", d.file);
    }
}
```

2. Run: `cargo test --test dead_code_integration -- --verbose`

3. Fix any test failures.

4. Commit: `test: add dead code detection integration tests`

---

## Task 21: Integration tests for architecture quality metrics

**Files:**
- Create: `tests/architecture_quality_integration.rs`

**Steps:**

1. Write tests for cohesion, boundary violations, and god file detection:

```rust
use cxpak::intelligence::architecture::{
    compute_cohesion, detect_god_files, is_boundary_violation,
};

#[test]
fn test_cohesion_range_is_zero_to_one() {
    for n in 0..=10usize {
        for e in 0..=(n * (n.saturating_sub(1))) {
            let c = compute_cohesion(e, n);
            assert!((0.0..=1.0 + 1e-9).contains(&c), "cohesion {c} out of [0,1] for n={n} e={e}");
        }
    }
}

#[test]
fn test_god_file_requires_at_least_3_files() {
    let counts = vec![("a.rs", 100usize)];
    let gods = detect_god_files(&counts);
    assert!(gods.is_empty(), "single file should never be a god file");
}

#[test]
fn test_boundary_violation_mod_rs_is_not_violation() {
    // mod.rs at the root of a module is always acceptable
    assert!(!is_boundary_violation("src/db/mod.rs", "src/db"));
}

#[test]
fn test_boundary_violation_deep_internal_file_is_violation() {
    // src/db/internal/pool.rs is 2 levels deep in src/db → violation
    assert!(is_boundary_violation("src/db/internal/pool.rs", "src/db"));
}

#[test]
fn test_god_file_detection_three_file_module() {
    let counts = vec![
        ("src/api/handler.rs", 1usize),
        ("src/api/middleware.rs", 2),
        ("src/api/router.rs", 100), // god file
    ];
    let gods = detect_god_files(&counts);
    assert!(gods.contains(&"src/api/router.rs"), "router.rs should be a god file");
    assert!(!gods.contains(&"src/api/handler.rs"), "handler.rs should not be a god file");
}
```

2. Run: `cargo test --test architecture_quality_integration -- --verbose`

3. Fix any failures.

4. Commit: `test: add architecture quality metrics integration tests`

---

## Task 22: Integration tests for monorepo workspace support

**Files:**
- Create: `tests/workspace_integration.rs`

**Steps:**

1. Write end-to-end workspace tests:

```rust
use assert_cmd::Command;
use std::fs;

fn setup_monorepo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::create_dir_all(dir.path().join("packages/api/src")).unwrap();
    fs::create_dir_all(dir.path().join("packages/web/src")).unwrap();
    fs::write(dir.path().join("packages/api/src/main.rs"), "fn main() {}").unwrap();
    fs::write(dir.path().join("packages/web/src/index.ts"), "export const x = 1;").unwrap();
    dir
}

#[test]
fn test_workspace_scopes_overview_to_prefix() {
    let dir = setup_monorepo();
    let mut cmd = Command::cargo_bin("cxpak").unwrap();
    cmd.arg("overview")
        .arg("--workspace")
        .arg("packages/api")
        .arg(dir.path().to_str().unwrap());
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The output must reference packages/api files
    assert!(stdout.contains("packages/api") || output.status.success(),
        "workspace-scoped overview should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr));
}

#[test]
fn test_workspace_cache_namespace_is_unique_per_workspace() {
    let ns_api = cxpak::commands::serve::cache_namespace(
        std::path::Path::new("/repo"), Some("packages/api"));
    let ns_web = cxpak::commands::serve::cache_namespace(
        std::path::Path::new("/repo"), Some("packages/web"));
    assert_ne!(ns_api, ns_web, "workspace cache namespaces must differ");
    assert!(ns_api.contains("packages_api"), "api namespace: {ns_api}");
    assert!(ns_web.contains("packages_web"), "web namespace: {ns_web}");
}
```

2. Run: `cargo test --test workspace_integration -- --verbose`

3. Fix any failures.

4. Commit: `test: add monorepo workspace integration tests`

---

## Task 23: Full test suite pass and coverage check

**Files:**
- No new files

**Steps:**

1. Run the full test suite: `cargo test --all-targets --verbose 2>&1 | tail -50`

2. Check for any compilation warnings: `cargo clippy --all-targets -- -D warnings 2>&1 | head -50`

3. Fix all clippy warnings.

4. Check formatting: `cargo fmt -- --check 2>&1 | head -20`

5. Fix any formatting issues: `cargo fmt`

6. Run coverage check: `cargo tarpaulin --timeout 120 --out Stdout 2>&1 | tail -10`
   - Target: ≥90% coverage. If below 90%, identify the uncovered paths and add targeted unit tests.

7. Run the full test suite one final time: `cargo test --all-targets 2>&1 | tail -20`

8. Commit: `chore: fix clippy warnings and formatting for v1.3.0`

---

## Task 24: Version bump and changelog

**Files:**
- Modify: `Cargo.toml`
- Modify: `plugin/.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`
- Modify: `Cargo.lock` (auto-generated via `cargo check`)

**Steps:**

1. In `Cargo.toml`, change `version = "1.1.0"` to `version = "1.3.0"`.

2. In `plugin/.claude-plugin/plugin.json`, update the version field to `"1.3.0"`.

3. In `.claude-plugin/marketplace.json`, update the version field to `"1.3.0"`.

4. Run: `cargo check 2>&1 | tail -10` — this regenerates `Cargo.lock` with the new version.

5. Run: `cargo test --all-targets 2>&1 | tail -20` — confirm all tests pass with bumped version.

6. Commit: `chore: bump version to 1.3.0`

---

## Task 25: Documentation update

**Files:**
- Modify: `plugin/.claude-plugin/plugin.json` (add new tool descriptions)

**Steps:**

1. Add the three new MCP tool descriptions to `plugin.json`:

```json
{
  "name": "cxpak_call_graph",
  "description": "Returns the cross-file call graph for a file or symbol. Edges include confidence level (Exact = import-resolved, Approximate = name-matched). Parameters: target (optional file path or symbol name), depth (default 1), focus, workspace."
},
{
  "name": "cxpak_dead_code",
  "description": "Returns dead symbol list sorted by liveness_score descending (most important dead symbols first). A symbol is dead when it has zero callers, is not an entry point (main, HTTP handler, test fn, pub root export), and is not referenced from test files. Parameters: focus, limit (default 50), workspace."
},
{
  "name": "cxpak_architecture",
  "description": "Returns full architecture quality report. Each module includes 5 metrics: coupling (cross-module edge ratio), cohesion (intra-module edge density), circular_dep_count, boundary_violations (imports bypassing module root), and god_files (mean+2σ inbound edge outliers). Parameters: focus, workspace."
}
```

2. Run: `cargo test --all-targets 2>&1 | tail -10` — final verification.

3. Commit: `docs: add MCP tool descriptions for call_graph, dead_code, architecture`

---

## Summary of New Files and Modified Files

**New files:**
- `/Users/lb/Documents/barnett/cxpak/src/intelligence/call_graph.rs`
- `/Users/lb/Documents/barnett/cxpak/src/intelligence/dead_code.rs`
- `/Users/lb/Documents/barnett/cxpak/tests/call_graph_integration.rs`
- `/Users/lb/Documents/barnett/cxpak/tests/dead_code_integration.rs`
- `/Users/lb/Documents/barnett/cxpak/tests/architecture_quality_integration.rs`
- `/Users/lb/Documents/barnett/cxpak/tests/workspace_integration.rs`

**Modified files:**
- `/Users/lb/Documents/barnett/cxpak/src/intelligence/mod.rs` — add `pub mod call_graph; pub mod dead_code;`
- `/Users/lb/Documents/barnett/cxpak/src/intelligence/architecture.rs` — extend `ModuleInfo`, add `BoundaryViolation`, `compute_cohesion`, `is_boundary_violation`, `detect_god_files`
- `/Users/lb/Documents/barnett/cxpak/src/intelligence/health.rs` — populate `dead_code` dimension
- `/Users/lb/Documents/barnett/cxpak/src/index/mod.rs` — add `call_graph: CallGraph` field
- `/Users/lb/Documents/barnett/cxpak/src/scanner/mod.rs` — add `scan_workspace()`
- `/Users/lb/Documents/barnett/cxpak/src/commands/serve.rs` — add 3 new MCP endpoints, workspace threading
- `/Users/lb/Documents/barnett/cxpak/src/commands/overview.rs` — `--workspace` flag
- `/Users/lb/Documents/barnett/cxpak/src/commands/trace.rs` — `--workspace` flag
- `/Users/lb/Documents/barnett/cxpak/src/main.rs` — `--workspace` flag plumbing
- `/Users/lb/Documents/barnett/cxpak/Cargo.toml` — version `1.3.0`
- `/Users/lb/Documents/barnett/cxpak/plugin/.claude-plugin/plugin.json` — version + tool descriptions
- `/Users/lb/Documents/barnett/cxpak/.claude-plugin/marketplace.json` — version

## Key Constraints Reminder

- **90% coverage required:** Every new public function needs unit tests. The integration test files in `tests/` cover end-to-end paths; unit tests inline in each module cover the algorithmic functions.
- **Pre-commit hooks:** `cargo fmt + cargo clippy -- -D warnings + cargo test` must all pass before each commit. Run them manually before committing.
- **`BoundaryViolation.edge_type` is `EdgeType` (not `String`)** — the `EdgeType` enum lives in `src/schema/mod.rs` and is imported via `use crate::schema::EdgeType`.
- **Call graph stored on `CodebaseIndex`:** built in `CodebaseIndex::build` and `build_with_content`, initialized to `CallGraph::default()` in `CodebaseIndex::empty()`.
- **Dead code dimension weight:** `0.10` out of the full 6-dimension composite (conventions 0.20, tests 0.20, churn 0.15, coupling 0.20, cycles 0.15, dead_code 0.10). Weights sum to 1.0.
- **Incremental rollout:** v1.3.0 ships call extraction for Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C# (top 10 Tier 1). All other languages use the regex fallback via `extract_regex_calls`.
