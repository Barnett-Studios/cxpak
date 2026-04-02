use crate::index::CodebaseIndex;
use crate::intelligence::api_surface::detect_routes;
use crate::parser::language::{SymbolKind, Visibility};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A symbol classified as dead (zero callers, not an entry point).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadSymbol {
    pub file: String,
    pub symbol: String,
    pub kind: SymbolKind,
    /// Sorting key: higher = more concerning dead symbol.
    /// Formula: pagerank * (1.0 + test_file_count) * export_weight
    /// where export_weight = 2.0 for pub exports, 1.0 otherwise.
    pub liveness_score: f64,
    pub reason: String,
}

/// Compute liveness score for sorting dead symbols.
/// Higher = more important dead symbol (pub export, has tests nearby, high pagerank).
pub fn compute_liveness_score(pagerank: f64, test_file_count: usize, is_pub_export: bool) -> f64 {
    let export_weight = if is_pub_export { 2.0 } else { 1.0 };
    pagerank * (1.0 + test_file_count as f64) * export_weight
}

/// Entry point detection: a symbol is a live entry point when it is:
/// - Named "main"
/// - An HTTP handler (detected via route patterns in the same file)
/// - A test function (name starts with "test_" or contains test markers in signature)
/// - A pub export from a lib root (mod.rs, lib.rs, index.ts, __init__.py)
/// - A trait implementation method
fn is_entry_point(
    file: &str,
    symbol_name: &str,
    signature: &str,
    is_public: bool,
    route_cache: &HashMap<String, bool>,
) -> bool {
    if symbol_name == "main" {
        return true;
    }
    if symbol_name.starts_with("test_")
        || signature.contains("#[test]")
        || signature.contains("@Test")
        || signature.contains("def test_")
    {
        return true;
    }
    let is_root_file = file.ends_with("mod.rs")
        || file.ends_with("lib.rs")
        || file.ends_with("index.ts")
        || file.ends_with("index.js")
        || file.ends_with("__init__.py");
    if is_public && is_root_file {
        return true;
    }
    // trait implementation: methods inside `impl Trait for Type` blocks
    if (signature.contains("impl ") && signature.contains(" for "))
        || signature.contains("@Override")
        || signature.contains("override ")
    {
        return true;
    }
    // HTTP handler: check if this file has route registrations
    if let Some(&has_routes) = route_cache.get(file) {
        if has_routes && is_public {
            return true;
        }
    }
    false
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
        || path.ends_with(".test.js")
        || path.ends_with(".spec.ts")
        || path.ends_with(".spec.js")
}

/// Detect dead symbols across the codebase.
///
/// A symbol is dead when ALL of:
/// - Zero callers in the call graph
/// - Not an entry point (main, HTTP handler, test fn, pub root export)
/// - Not referenced in any test file (via test_map + call graph)
///
/// Returns symbols sorted by liveness_score descending (most important dead symbols first).
pub fn detect_dead_code(index: &CodebaseIndex, focus: Option<&str>) -> Vec<DeadSymbol> {
    // Build set of test-referenced symbols from call graph
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

    // Pre-cache route detection per file (avoid N calls per symbol)
    let mut route_cache: HashMap<String, bool> = HashMap::new();
    for file in &index.files {
        if !route_cache.contains_key(&file.relative_path) {
            let routes = detect_routes(&file.content, &file.relative_path);
            route_cache.insert(file.relative_path.clone(), !routes.is_empty());
        }
    }

    let mut dead: Vec<DeadSymbol> = Vec::new();

    for file in &index.files {
        if let Some(prefix) = focus {
            if !file.relative_path.starts_with(prefix) {
                continue;
            }
        }
        if is_test_file(&file.relative_path) {
            continue;
        }
        let Some(pr) = &file.parse_result else {
            continue;
        };

        for symbol in &pr.symbols {
            let has_callers = index
                .call_graph
                .has_callers(&file.relative_path, &symbol.name);
            if has_callers {
                continue;
            }

            let is_public = symbol.visibility == Visibility::Public;
            if is_entry_point(
                &file.relative_path,
                &symbol.name,
                &symbol.signature,
                is_public,
                &route_cache,
            ) {
                continue;
            }

            let key = (file.relative_path.clone(), symbol.name.clone());
            if test_referenced.contains(&key) {
                continue;
            }

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
            let is_pub_export = pr.exports.iter().any(|e| e.name == symbol.name);
            let liveness_score = compute_liveness_score(pagerank, test_file_count, is_pub_export);

            dead.push(DeadSymbol {
                file: file.relative_path.clone(),
                symbol: symbol.name.clone(),
                kind: symbol.kind.clone(),
                liveness_score,
                reason: "zero callers, not entry point, no test reference".into(),
            });
        }
    }

    dead.sort_by(|a, b| {
        b.liveness_score
            .partial_cmp(&a.liveness_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    dead
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

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

    #[test]
    fn test_liveness_score_formula() {
        // pagerank=0.5, test_file_count=1, export_weight=2.0 → 0.5 × 2.0 × 2.0 = 2.0
        let score = compute_liveness_score(0.5, 1, true);
        assert!((score - 2.0).abs() < 1e-9, "expected 2.0, got {score}");

        // pagerank=0.3, test_file_count=0, export_weight=1.0 → 0.3 × 1.0 × 1.0 = 0.3
        let score2 = compute_liveness_score(0.3, 0, false);
        assert!((score2 - 0.3).abs() < 1e-9, "expected 0.3, got {score2}");
    }

    #[test]
    fn test_detect_dead_code_finds_uncalled_private_function() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("util.rs");
        std::fs::write(&fp, "fn live_fn() {} fn dead_fn() {}").unwrap();

        let files = vec![ScannedFile {
            relative_path: "util.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 36,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "util.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "live_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn live_fn()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "dead_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn dead_fn()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let dead = detect_dead_code(&index, None);
        assert!(
            dead.iter().any(|d| d.symbol == "dead_fn"),
            "dead_fn should be detected as dead, got: {:?}",
            dead.iter().map(|d| &d.symbol).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_main_function_is_not_dead() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("main.rs");
        std::fs::write(&fp, "fn main() {}").unwrap();

        let files = vec![ScannedFile {
            relative_path: "main.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 12,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "main.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "main".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn main()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let dead = detect_dead_code(&index, None);
        assert!(
            !dead.iter().any(|d| d.symbol == "main"),
            "main() must never be classified as dead"
        );
    }

    #[test]
    fn test_test_function_is_not_dead() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("tests.rs");
        std::fs::write(&fp, "fn test_something() {}").unwrap();

        let files = vec![ScannedFile {
            relative_path: "tests.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 22,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "tests.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "test_something".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn test_something()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let dead = detect_dead_code(&index, None);
        assert!(
            !dead.iter().any(|d| d.symbol == "test_something"),
            "test functions must not be classified as dead"
        );
    }

    #[test]
    fn test_liveness_score_is_nonnegative() {
        assert!(compute_liveness_score(0.0, 0, false) >= 0.0);
        assert!(compute_liveness_score(1.0, 10, true) >= 0.0);
    }

    #[test]
    fn test_dead_symbol_serialize() {
        let ds = DeadSymbol {
            file: "a.rs".into(),
            symbol: "orphan".into(),
            kind: SymbolKind::Function,
            liveness_score: 0.5,
            reason: "dead".into(),
        };
        let json = serde_json::to_string(&ds).unwrap();
        assert!(json.contains("\"orphan\""));
    }
}
