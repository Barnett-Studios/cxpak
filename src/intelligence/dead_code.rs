use crate::index::{CodebaseIndex, IndexedFile};
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

/// Returns true when the symbol kind represents a callable (function/method).
/// These are checked against the call graph for callers.
fn is_callable_kind(kind: &SymbolKind) -> bool {
    matches!(kind, SymbolKind::Function | SymbolKind::Method)
}

/// Returns true when the symbol kind represents a type definition.
/// Types don't appear in call graphs; we use string-reference scanning instead.
fn is_type_kind(kind: &SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::Interface
            | SymbolKind::Class
            | SymbolKind::TypeAlias
    )
}

/// Returns true when the symbol name appears as a substring in any file other than
/// `defining_file`. Short names (<3 chars) are assumed alive to avoid false positives.
fn has_string_references(
    symbol_name: &str,
    defining_file: &str,
    all_files: &[IndexedFile],
) -> bool {
    if symbol_name.len() < 3 {
        return true; // too short to search reliably — assume alive
    }
    for file in all_files {
        if file.relative_path == defining_file {
            continue;
        }
        if file.content.contains(symbol_name) {
            return true;
        }
    }
    false
}

/// Returns true when a module-qualified reference to the symbol appears in any
/// file other than its defining file. For `src/commands/overview.rs::run`, this
/// searches for `overview::run` and `commands::overview::run` across all files.
///
/// Qualification disambiguates common function names (like `run`, `new`,
/// `build`) which would produce too many false negatives under a plain name
/// search. The call graph should ideally catch these references via path
/// resolution, but it misses some macro-expanded / dispatched calls; this
/// check is the last-resort fallback before declaring a callable dead.
fn has_qualified_reference(file_path: &str, symbol_name: &str, all_files: &[IndexedFile]) -> bool {
    let path = std::path::Path::new(file_path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // `mod.rs` files qualify under their parent directory name, not "mod".
    let effective_stem = if stem == "mod" {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or(stem)
    } else {
        stem
    };
    if effective_stem.is_empty() {
        return false;
    }
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let one = format!("{effective_stem}::{symbol_name}");
    let two = if !parent.is_empty() && parent != effective_stem {
        format!("{parent}::{effective_stem}::{symbol_name}")
    } else {
        String::new()
    };
    let use_pattern = "use ".to_string();
    let re_export = format!("::{symbol_name};");
    let bare_qualified = format!("::{symbol_name}");
    for file in all_files {
        if file.relative_path == file_path {
            continue;
        }
        let content = &file.content;
        if content.contains(&one) {
            return true;
        }
        if !two.is_empty() && content.contains(&two) {
            return true;
        }
        // Handle re-exports / direct imports: look for `use ...::{name};` or
        // `pub use ...::{name};` patterns. We require both `use ` AND `::{name};`
        // to appear, on the assumption a re-export uses one of those forms.
        if content.contains(&use_pattern)
            && (content.contains(&re_export) || content.contains(&bare_qualified))
        {
            return true;
        }
    }
    false
}

/// Returns true when the symbol name is referenced inside `content` (the file that
/// defines it) beyond its own definition. Uses word-boundary matching to avoid
/// prefix false-positives (e.g., `"foo"` must not match `"foobar"`).
///
/// Short names (<3 chars) are assumed alive to avoid false positives from ubiquitous
/// identifiers like `id`, `ok`, etc.
fn same_file_string_reference(name: &str, content: &str) -> bool {
    if name.len() < 3 {
        return true; // too short — assume alive
    }
    // Use word-boundary regex for precision. If regex compilation fails (should
    // not happen for valid identifiers), fall back to simple contains-count.
    if let Ok(re) = regex::Regex::new(&format!(r"\b{}\b", regex::escape(name))) {
        // More than 1 occurrence means the name appears outside its definition.
        re.find_iter(content).count() > 1
    } else {
        content.matches(name).count() > 1
    }
}

fn is_test_file(path: &str) -> bool {
    // Top-level conventions (most-common miss before v2.1.0):
    path.starts_with("tests/")
        || path.starts_with("test/")
        || path.starts_with("spec/")
        // Nested test directories anywhere in the tree:
        || path.contains("/tests/")
        || path.contains("/test/")
        || path.contains("/spec/")
        || path.contains("__tests__")
        // Language-specific naming conventions:
        || path.ends_with("_test.rs")
        || path.ends_with("_test.py")
        || path.ends_with("_test.go")
        || path.ends_with(".test.ts")
        || path.ends_with(".test.js")
        || path.ends_with(".spec.ts")
        || path.ends_with(".spec.js")
}

/// Returns true if the lines immediately preceding the symbol declaration
/// contain a `#[derive(...)]` attribute listing any of the serde traits. Such
/// structs/enums are reachable via runtime deserialization (e.g. axum's
/// `Json<T>` extractors) even when the call graph shows zero callers.
///
/// Scans at most 10 lines before `start_line` to handle multi-line attributes
/// and doc comments interleaved with derives.
fn has_serde_derive_above(content: &str, start_line: usize) -> bool {
    if start_line == 0 {
        return false;
    }
    // Lines are 1-indexed in Symbol.start_line; indexable lines starts_line-1 backwards.
    let lines: Vec<&str> = content.lines().collect();
    let end = start_line.saturating_sub(1);
    let start = end.saturating_sub(10);
    // Walk forward through the window, accumulating lines that belong to a
    // derive block (single-line or multi-line). Multi-line derives are common
    // after rustfmt formats long attribute lists across multiple lines.
    let mut in_derive = false;
    let mut accumulator = String::new();
    for i in start..end {
        let Some(line) = lines.get(i) else { continue };
        let l = line.trim();
        if l.starts_with("#[derive") {
            in_derive = true;
            accumulator.push_str(l);
            accumulator.push(' ');
            // Single-line derive that closes on same line:
            if l.contains(")]") {
                in_derive = false;
            }
        } else if in_derive {
            accumulator.push_str(l);
            accumulator.push(' ');
            if l.contains(")]") {
                in_derive = false;
            }
        }
    }
    accumulator.contains("Deserialize") || accumulator.contains("Serialize")
}

/// Removes content inside double-quoted strings and after `//` line comments.
/// Preserves brace characters that appear in code so that brace-depth counting
/// is not corrupted by literals like `"{ {{ "` or comments like `// {`.
///
/// Single-pass: tracks whether we are inside a string and whether we have
/// already seen `//` on this line. Does not handle raw strings (`r#"..."#`)
/// perfectly — those are rare in impl headers and the worst case is a
/// false negative (returning None instead of the correct impl context).
fn strip_strings_and_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_string = false;
    let mut prev = '\0';
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek() == Some(&'/') {
            // Rest of line is a comment.
            break;
        }
        if ch == '"' && prev != '\\' {
            in_string = !in_string;
            // Don't push the quote either way; it's not a brace.
            prev = ch;
            continue;
        }
        if !in_string {
            out.push(ch);
        }
        prev = ch;
    }
    out
}

/// Inspects the immediate-enclosing block opener for a symbol and classifies
/// the impl context. Returns:
///   - `Some((true, _))`  if the symbol is inside `impl Trait for Type {`
///   - `Some((false, Some(type)))` if inside `impl Type {` (inherent impl)
///   - `None` otherwise (not in an impl block, or detection failed)
///
/// Walks backward from `start_line - 1` counting brace depth; the first
/// `{` that drops depth below zero opens the parent scope, so we inspect that
/// line. Multi-line impl headers (e.g. with `where` clauses) are handled by
/// only looking at the line that contains the unmatched `{`.
///
/// Braces inside string literals and `//` line comments are ignored via
/// `strip_strings_and_comments` to avoid false depth counts.
fn enclosing_impl(content: &str, start_line: usize) -> Option<(bool, Option<String>)> {
    if start_line == 0 {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let mut depth: i32 = 0;
    for i in (0..start_line.saturating_sub(1)).rev() {
        let Some(line) = lines.get(i) else { continue };
        let cleaned = strip_strings_and_comments(line);
        for ch in cleaned.chars().rev() {
            if ch == '}' {
                depth += 1;
            } else if ch == '{' {
                depth -= 1;
                if depth < 0 {
                    let trimmed = line.trim_start();
                    if !trimmed.starts_with("impl") {
                        return None;
                    }
                    // Trait impl: `impl ... for Type {` (or impl Trait<..> for Type<..>)
                    if let Some(idx) = trimmed.find(" for ") {
                        return Some((true, parse_impl_type(&trimmed[idx + 5..])));
                    }
                    // Inherent impl: `impl Type<..> {`
                    let after_impl = trimmed.trim_start_matches("impl").trim_start();
                    return Some((false, parse_impl_type(after_impl)));
                }
            }
        }
    }
    None
}

/// Extracts the bare type name from an impl-block fragment.
///
/// Handles all common forms:
///   - `Foo {`
///   - `Foo<T> {`
///   - `Foo where T: Clone {`
///   - `<T: Clone> Foo<T> {` (generic params attached to `impl`)
///
/// Returns None when no plausible identifier is found.
fn parse_impl_type(fragment: &str) -> Option<String> {
    let mut s = fragment.trim_start();
    // Skip a leading generic parameter block, e.g. `<T: Clone>`. Count `<>`
    // depth so nested generics like `<T: Iterator<Item = U>>` are handled.
    if s.starts_with('<') {
        let mut depth = 0i32;
        let mut end = 0usize;
        for (i, ch) in s.char_indices() {
            if ch == '<' {
                depth += 1;
            } else if ch == '>' {
                depth -= 1;
                if depth == 0 {
                    end = i + ch.len_utf8();
                    break;
                }
            }
        }
        if end == 0 {
            return None;
        }
        s = s[end..].trim_start();
    }
    let mut name = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Returns true if the lines immediately preceding the symbol declaration
/// contain a `#[test]`, `#[tokio::test]`, or any `_test`-suffixed test
/// attribute. These functions are entry points invoked by the test harness
/// and have no callers in the call graph.
fn has_test_attribute_above(content: &str, start_line: usize) -> bool {
    if start_line == 0 {
        return false;
    }
    let lines: Vec<&str> = content.lines().collect();
    let end = start_line.saturating_sub(1);
    let start = end.saturating_sub(6);
    for i in start..end {
        let Some(line) = lines.get(i) else { continue };
        let l = line.trim();
        if l.starts_with("#[test]")
            || l.starts_with("#[tokio::test")
            || l.starts_with("#[async_std::test")
            || l.starts_with("#[rstest")
            || l.starts_with("#[cfg(test)]")
        {
            return true;
        }
    }
    false
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
            // Structural-only kinds (Heading, Selector, Key, etc.) are not semantic
            // entities and must be skipped entirely from dead code detection.
            if !is_callable_kind(&symbol.kind)
                && !is_type_kind(&symbol.kind)
                && symbol.kind != SymbolKind::Constant
            {
                continue;
            }

            let is_alive = if is_callable_kind(&symbol.kind) {
                // Harness-invoked test functions (`#[test]`, `#[tokio::test]`, etc.)
                // have no callers in the call graph but are run by cargo test. Mark
                // them alive before consulting the graph.
                if has_test_attribute_above(&file.content, symbol.start_line) {
                    true
                } else {
                    // Compute enclosing impl context once for Method symbols.
                    let impl_ctx = if symbol.kind == SymbolKind::Method {
                        enclosing_impl(&file.content, symbol.start_line)
                    } else {
                        None
                    };
                    let is_trait_method = impl_ctx
                        .as_ref()
                        .map(|(is_trait, _)| *is_trait)
                        .unwrap_or(false);
                    let inherent_type =
                        impl_ctx
                            .as_ref()
                            .and_then(|(is_trait, ty)| if !is_trait { ty.clone() } else { None });

                    if is_trait_method {
                        // Trait-impl methods are dispatched at runtime; the call graph
                        // does not resolve dynamic / generic dispatch, so without this
                        // check every `impl Trait for X` method would be flagged dead.
                        true
                    } else {
                        // Inherent-impl Type::method cross-file reference is ADDITIVE
                        // evidence of liveness. If absent, fall through to call graph
                        // and same-file/qualified-ref fallbacks (the method may still
                        // be alive via `Self::method` calls in the same file or via a
                        // call-graph-tracked invocation).
                        let inherent_type_ref = inherent_type
                            .as_ref()
                            .map(|ty| {
                                let pat = format!("{ty}::{}", symbol.name);
                                index.files.iter().any(|f| {
                                    f.relative_path != file.relative_path
                                        && f.content.contains(&pat)
                                })
                            })
                            .unwrap_or(false);
                        if inherent_type_ref {
                            true
                        } else {
                            let has_callers = index
                                .call_graph
                                .has_callers(&file.relative_path, &symbol.name);
                            if has_callers {
                                true
                            } else {
                                let is_public = symbol.visibility == Visibility::Public;
                                let is_ep = is_entry_point(
                                    &file.relative_path,
                                    &symbol.name,
                                    &symbol.signature,
                                    is_public,
                                    &route_cache,
                                );
                                let is_test_ref = {
                                    let key = (file.relative_path.clone(), symbol.name.clone());
                                    test_referenced.contains(&key)
                                };
                                // Fallback 1: the call graph tracks cross-file edges but may
                                // miss intra-file calls (private helpers called from within
                                // the same file, or `Self::method` calls in inherent impls).
                                // Check whether the name appears more than once in the file's
                                // content using word-boundary matching. If so, the symbol is
                                // referenced locally and is alive.
                                let is_same_file_ref =
                                    same_file_string_reference(&symbol.name, &file.content);
                                // Fallback 2: qualified-name match across other files. Catches
                                // calls like `commands::overview::run(args)` that the call
                                // graph didn't resolve to the concrete symbol.
                                let is_qualified_ref = has_qualified_reference(
                                    &file.relative_path,
                                    &symbol.name,
                                    &index.files,
                                );
                                is_ep || is_test_ref || is_same_file_ref || is_qualified_ref
                            }
                        }
                    }
                }
            } else {
                // For types and constants: a serde-derived type is reachable via
                // runtime deserialization (axum extractors, config files, RPC bodies).
                // The call graph doesn't model this, so detect the derive attribute
                // first.
                if (symbol.kind == SymbolKind::Struct
                    || matches!(symbol.kind, SymbolKind::Enum | SymbolKind::TypeAlias))
                    && has_serde_derive_above(&file.content, symbol.start_line)
                {
                    true
                } else {
                    // Same-file usage check (constants like MAX_PATTERN_LEN are often
                    // referenced exclusively from the file that defines them).
                    let is_same_file_ref = same_file_string_reference(&symbol.name, &file.content);
                    is_same_file_ref
                        || has_string_references(&symbol.name, &file.relative_path, &index.files)
                }
            };

            if is_alive {
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
    fn is_test_file_matches_top_level_tests_dir() {
        // Top-level Rust integration tests live under `tests/` with no leading slash.
        // Pre-2.1.0 the heuristic used `path.contains("/tests/")` which missed these.
        assert!(is_test_file("tests/spa_render.rs"));
        assert!(is_test_file("tests/v1_api_wired.rs"));
        assert!(is_test_file("test/spec_a.py"));
        assert!(is_test_file("spec/widget_spec.rb"));
        // Nested test directories still work.
        assert!(is_test_file("crates/foo/tests/integration.rs"));
        // Source files must NOT match.
        assert!(!is_test_file("src/main.rs"));
        assert!(!is_test_file("src/intelligence/dead_code.rs"));
    }

    #[test]
    fn has_test_attribute_above_detects_common_attributes() {
        let content = "use foo;\n#[test]\nfn my_test() {}\n";
        // my_test is on line 3 (1-indexed)
        assert!(has_test_attribute_above(content, 3));
        let tokio_content = "use foo;\n#[tokio::test]\nasync fn my_test() {}\n";
        assert!(has_test_attribute_above(tokio_content, 3));
        let no_attr = "use foo;\n\nfn my_fn() {}\n";
        assert!(!has_test_attribute_above(no_attr, 3));
    }

    #[test]
    fn has_serde_derive_above_detects_deserialize() {
        let content = "use serde::Deserialize;\n\n#[derive(Debug, Deserialize)]\nstruct Foo {}\n";
        // struct Foo is on line 4
        assert!(has_serde_derive_above(content, 4));
        let no_derive = "use serde::Deserialize;\n\n#[derive(Debug)]\nstruct Foo {}\n";
        assert!(!has_serde_derive_above(no_derive, 4));
        let serialize_only = "#[derive(Serialize)]\nstruct Foo {}\n";
        assert!(has_serde_derive_above(serialize_only, 2));
    }

    #[test]
    fn enclosing_impl_distinguishes_trait_vs_inherent() {
        // Trait impl
        let trait_impl = "impl Display for Foo {\n    fn fmt(&self) -> String {\n        String::new()\n    }\n}\n";
        assert_eq!(
            enclosing_impl(trait_impl, 2),
            Some((true, Some("Foo".to_string())))
        );
        // Inherent impl
        let inh = "impl Foo {\n    fn helper() -> i32 {\n        42\n    }\n}\n";
        assert_eq!(
            enclosing_impl(inh, 2),
            Some((false, Some("Foo".to_string())))
        );
        // Generic inherent impl
        let gen_inh = "impl<T: Clone> Foo<T> {\n    fn build() {}\n}\n";
        assert_eq!(
            enclosing_impl(gen_inh, 2),
            Some((false, Some("Foo".to_string())))
        );
        // Not in an impl
        let bare = "fn standalone() {}\n";
        assert_eq!(enclosing_impl(bare, 1), None);
    }

    #[test]
    fn parse_impl_type_extracts_bare_identifier() {
        assert_eq!(parse_impl_type("Foo {"), Some("Foo".to_string()));
        assert_eq!(parse_impl_type("Foo<'a, T> {"), Some("Foo".to_string()));
        assert_eq!(
            parse_impl_type("CxpakLspBackend {"),
            Some("CxpakLspBackend".to_string())
        );
        assert_eq!(
            parse_impl_type("Foo where T: Clone {"),
            Some("Foo".to_string())
        );
        assert_eq!(parse_impl_type(""), None);
    }

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

    // ---- type-kind dead code fixes ----

    fn make_struct_index(
        symbol_name: &str,
        def_content: &str,
        ref_content: Option<&str>,
    ) -> crate::index::CodebaseIndex {
        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let def_path = dir.path().join("a.rs");
        std::fs::write(&def_path, def_content).unwrap();
        let mut files = vec![ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: def_path,
            language: Some("rust".into()),
            size_bytes: def_content.len() as u64,
        }];
        let mut content_map = HashMap::new();
        content_map.insert("a.rs".to_string(), def_content.to_string());
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "a.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: symbol_name.into(),
                    kind: SymbolKind::Struct,
                    visibility: Visibility::Public,
                    signature: format!("pub struct {symbol_name}"),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        if let Some(ref_src) = ref_content {
            let ref_path = dir.path().join("b.rs");
            std::fs::write(&ref_path, ref_src).unwrap();
            files.push(ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: ref_path,
                language: Some("rust".into()),
                size_bytes: ref_src.len() as u64,
            });
            content_map.insert("b.rs".to_string(), ref_src.to_string());
        }
        crate::index::CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    #[test]
    fn test_dead_code_skips_used_struct() {
        // a.rs defines struct Foo; b.rs references "Foo" by name.
        let index = make_struct_index(
            "Foo",
            "pub struct Foo {}",
            Some("fn bar() -> Foo { todo!() }"),
        );
        let dead = detect_dead_code(&index, None);
        assert!(
            !dead.iter().any(|d| d.symbol == "Foo"),
            "Foo is referenced in b.rs and must NOT be dead: {:?}",
            dead.iter().map(|d| &d.symbol).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dead_code_flags_unused_private_struct() {
        // Single file with a struct that has no references in any other file.
        let index = make_struct_index("Orphan", "pub struct Orphan {}", None);
        let dead = detect_dead_code(&index, None);
        assert!(
            dead.iter().any(|d| d.symbol == "Orphan"),
            "Orphan struct with no external references must be dead: {:?}",
            dead.iter().map(|d| &d.symbol).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dead_code_flags_unused_private_function() {
        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("util.rs");
        std::fs::write(&fp, "fn unused() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "util.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 14,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "util.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "unused".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn unused()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("util.rs".to_string(), "fn unused() {}".to_string());
        let index = crate::index::CodebaseIndex::build_with_content(
            files,
            parse_results,
            &counter,
            content_map,
        );
        let dead = detect_dead_code(&index, None);
        assert!(
            dead.iter().any(|d| d.symbol == "unused"),
            "private fn with no callers must be flagged dead"
        );
    }

    // ---- same-file string reference fallback tests (Bug 7) ----

    #[test]
    fn test_same_file_string_reference_finds_call() {
        let content = "fn helper() {} fn public_fn() { helper(); }";
        assert!(
            same_file_string_reference("helper", content),
            "helper appears twice: once in definition, once in call"
        );
    }

    #[test]
    fn test_same_file_string_reference_single_occurrence_not_referenced() {
        let content = "fn unused() { println!(\"hi\"); }";
        assert!(
            !same_file_string_reference("unused", content),
            "unused appears only once (the definition) — not alive"
        );
    }

    #[test]
    fn test_same_file_string_reference_word_boundary() {
        // "foo" must NOT match "foobar"
        let content = "fn foobar() { println!(\"unrelated\"); }";
        assert!(
            !same_file_string_reference("foo", content),
            "word-boundary: 'foo' must not match 'foobar'"
        );
    }

    #[test]
    fn test_same_file_string_reference_short_name_returns_true() {
        // Very short names (<3 chars) are assumed alive to avoid false positives.
        assert!(
            same_file_string_reference("id", "fn id() {}"),
            "names shorter than 3 chars must be assumed alive"
        );
    }

    #[test]
    fn test_dead_code_skips_privately_called_helper() {
        // File content: helper is defined AND called — not dead.
        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "fn helper() {}\nfn public_fn() { helper(); }";
        let fp = dir.path().join("util.rs");
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "util.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "util.rs".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "helper".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn helper()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "public_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn public_fn()".into(),
                        body: "{ helper(); }".into(),
                        start_line: 2,
                        end_line: 2,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("util.rs".to_string(), content.to_string());
        let index = crate::index::CodebaseIndex::build_with_content(
            files,
            parse_results,
            &counter,
            content_map,
        );
        let dead = detect_dead_code(&index, None);
        assert!(
            !dead.iter().any(|d| d.symbol == "helper"),
            "helper is called within util.rs and must NOT be flagged as dead"
        );
    }

    #[test]
    fn test_dead_code_flags_unused_helper_even_with_short_name_false() {
        // A private function with a 5+ char name that genuinely has no references.
        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "fn orphan() {}";
        let fp = dir.path().join("isolate.rs");
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "isolate.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "isolate.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "orphan".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn orphan()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("isolate.rs".to_string(), content.to_string());
        let index = crate::index::CodebaseIndex::build_with_content(
            files,
            parse_results,
            &counter,
            content_map,
        );
        let dead = detect_dead_code(&index, None);
        assert!(
            dead.iter().any(|d| d.symbol == "orphan"),
            "orphan with no callers or references must be flagged as dead"
        );
    }

    #[test]
    fn enclosing_impl_ignores_braces_in_strings() {
        let content =
            "impl Foo {\n    fn with_string() {\n        let s = \"{nested}\";\n    }\n}\n";
        // fn with_string is on line 2
        assert_eq!(
            enclosing_impl(content, 2),
            Some((false, Some("Foo".to_string())))
        );
    }

    #[test]
    fn has_serde_derive_above_handles_multiline() {
        let content =
            "#[derive(\n    Debug,\n    Clone,\n    Deserialize,\n)]\npub struct Req {}\n";
        // pub struct Req is on line 6
        assert!(has_serde_derive_above(content, 6));
    }

    #[test]
    fn test_dead_code_skips_short_names() {
        // A struct named "T" (< 3 chars) must be treated as alive to avoid false positives.
        let counter = crate::budget::counter::TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("short.rs");
        std::fs::write(&fp, "pub struct T {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "short.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 14,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "short.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "T".into(),
                    kind: SymbolKind::Struct,
                    visibility: Visibility::Public,
                    signature: "pub struct T".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("short.rs".to_string(), "pub struct T {}".to_string());
        let index = crate::index::CodebaseIndex::build_with_content(
            files,
            parse_results,
            &counter,
            content_map,
        );
        let dead = detect_dead_code(&index, None);
        assert!(
            !dead.iter().any(|d| d.symbol == "T"),
            "single-char struct name must be assumed alive (too short to search reliably)"
        );
    }
}
