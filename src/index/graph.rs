use super::IndexedFile;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Identifies a cross-language boundary type. Used as the payload of
/// [`EdgeType::CrossLanguage`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BridgeType {
    /// HTTP request from one service to another (fetch / axios / reqwest → route handler).
    HttpCall,
    /// FFI binding between languages (e.g. Rust extern "C" to a C function).
    FfiBinding,
    /// gRPC client call to a service defined in a `.proto` file.
    GrpcCall,
    /// GraphQL query/mutation against a typed schema.
    GraphqlCall,
    /// Two files that read/write the same database schema entity from different languages.
    SharedSchema,
    /// `subprocess.run` / `exec.Command` invocation of another binary tracked in the index.
    CommandExec,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeType {
    Import,
    ForeignKey,
    ViewReference,
    TriggerTarget,
    IndexTarget,
    FunctionReference,
    EmbeddedSql,
    OrmModel,
    MigrationSequence,
    /// Cross-language symbol resolution edge (v1.5.0).
    CrossLanguage(BridgeType),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypedEdge {
    pub target: String,
    pub edge_type: EdgeType,
}

#[derive(Debug, Default)]
pub struct DependencyGraph {
    pub edges: HashMap<String, HashSet<TypedEdge>>,
    pub reverse_edges: HashMap<String, HashSet<TypedEdge>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_edge(&mut self, from: &str, to: &str, edge_type: EdgeType) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .insert(TypedEdge {
                target: to.to_string(),
                edge_type: edge_type.clone(),
            });
        self.reverse_edges
            .entry(to.to_string())
            .or_default()
            .insert(TypedEdge {
                target: from.to_string(),
                edge_type,
            });
    }

    pub fn dependents(&self, path: &str) -> Vec<&TypedEdge> {
        self.reverse_edges
            .get(path)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    pub fn dependencies(&self, path: &str) -> Option<&HashSet<TypedEdge>> {
        self.edges.get(path)
    }

    /// Remove all outgoing edges from `source` and clean up corresponding reverse edges.
    ///
    /// Used during incremental re-indexing: call this before re-adding the new
    /// edges from a freshly parsed file.
    pub fn remove_edges_for(&mut self, source: &str) {
        if let Some(targets) = self.edges.remove(source) {
            for edge in &targets {
                if let Some(rev) = self.reverse_edges.get_mut(edge.target.as_str()) {
                    rev.retain(|e| e.target != source);
                    if rev.is_empty() {
                        self.reverse_edges.remove(edge.target.as_str());
                    }
                }
            }
        }
    }

    /// BFS from `start_files`, following edges in both directions.
    ///
    /// Returns the set of all reachable file paths, including the start files
    /// themselves.
    pub fn reachable_from(&self, start_files: &[&str]) -> HashSet<String> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        for &path in start_files {
            if visited.insert(path.to_string()) {
                queue.push_back(path.to_string());
            }
        }

        while let Some(current) = queue.pop_front() {
            // Follow outgoing edges (files that `current` imports)
            if let Some(deps) = self.edges.get(&current) {
                for edge in deps {
                    if visited.insert(edge.target.clone()) {
                        queue.push_back(edge.target.clone());
                    }
                }
            }

            // Follow incoming edges (files that import `current`)
            if let Some(importers) = self.reverse_edges.get(&current) {
                for edge in importers {
                    if visited.insert(edge.target.clone()) {
                        queue.push_back(edge.target.clone());
                    }
                }
            }
        }

        visited
    }
}

// ─── Import resolution helpers ────────────────────────────────────────────────

/// Return the directory component of a slash-separated path.
/// `"src/foo/bar.rs"` → `"src/foo"`.  Returns `""` for top-level files.
fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

/// Walk up `levels` directory levels from `dir`.
/// `parent_n("src/a/b/c", 2)` → `"src/a"`.
fn parent_n(dir: &str, levels: usize) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for _ in 0..levels {
        parts.pop();
    }
    parts.join("/")
}

/// Return the first candidate from `candidates` that exists in `all_paths`.
fn try_candidates(candidates: &[String], all_paths: &HashSet<&str>) -> Option<String> {
    for c in candidates {
        if all_paths.contains(c.as_str()) {
            return Some(c.clone());
        }
    }
    None
}

/// Resolve a Rust `use` path to an actual file path present in `all_paths`.
///
/// Handled prefixes:
/// - `crate::X` → `src/X.rs`, `src/X/mod.rs`
/// - `self::X`  → `<source_dir>/X.rs`, `<source_dir>/X/mod.rs`
/// - `super::…::X` (any depth) → walk up that many dirs, then append rest
/// - anything else is treated as an external crate (returns `None`)
fn resolve_rust_import(
    source_path: &str,
    import_source: &str,
    all_paths: &HashSet<&str>,
) -> Option<String> {
    let source_dir = parent_dir(source_path);

    // `crate::X::Y` → `src/X/Y.rs` or `src/X/Y/mod.rs`
    if let Some(rest) = import_source.strip_prefix("crate::") {
        let base = rest.replace("::", "/");
        return try_candidates(
            &[format!("src/{base}.rs"), format!("src/{base}/mod.rs")],
            all_paths,
        );
    }

    // `self::X` → same directory as source
    if let Some(rest) = import_source.strip_prefix("self::") {
        let base = rest.replace("::", "/");
        return try_candidates(
            &[
                format!("{source_dir}/{base}.rs"),
                format!("{source_dir}/{base}/mod.rs"),
            ],
            all_paths,
        );
    }

    // `super::…::X` — count leading `super::` segments, walk up that many dirs
    let mut trimmed = import_source;
    let mut super_count = 0usize;
    while let Some(rest) = trimmed.strip_prefix("super::") {
        super_count += 1;
        trimmed = rest;
    }
    if super_count > 0 {
        let target_dir = parent_n(source_dir, super_count);
        let base = trimmed.replace("::", "/");
        let (a, b) = if target_dir.is_empty() {
            (format!("{base}.rs"), format!("{base}/mod.rs"))
        } else {
            (
                format!("{target_dir}/{base}.rs"),
                format!("{target_dir}/{base}/mod.rs"),
            )
        };
        return try_candidates(&[a, b], all_paths);
    }

    // Not a known Rust path prefix — fall back to the legacy heuristic.
    // This handles unusual import source strings produced by test fixtures or
    // by parsers that emit non-standard module paths.
    resolve_legacy(import_source, all_paths)
}

/// Resolve a Python import source to an actual file path present in `all_paths`.
///
/// Python relative imports keep their leading dots in the source string
/// (e.g. `"."`, `".foo"`, `"..bar"`).
fn resolve_python_import(
    source_path: &str,
    import_source: &str,
    all_paths: &HashSet<&str>,
) -> Option<String> {
    let source_dir = parent_dir(source_path);

    // Count leading dots to determine relative level
    let dots = import_source.chars().take_while(|c| *c == '.').count();
    let rest = &import_source[dots..];
    let rest_path = rest.replace('.', "/");

    if dots == 1 {
        // `from .X import Y` → `<source_dir>/X.py` or `<source_dir>/X/__init__.py`
        if rest_path.is_empty() {
            // `from . import Y` — the package itself
            let candidates = vec![
                format!("{source_dir}/__init__.py"),
                format!("{source_dir}.py"),
            ];
            return try_candidates(&candidates, all_paths);
        }
        return try_candidates(
            &[
                format!("{source_dir}/{rest_path}.py"),
                format!("{source_dir}/{rest_path}/__init__.py"),
            ],
            all_paths,
        );
    } else if dots > 1 {
        // `from ..X import Y` — walk up (dots-1) directories from source_dir
        let target_dir = parent_n(source_dir, dots - 1);
        if rest_path.is_empty() {
            let candidates = if target_dir.is_empty() {
                vec!["__init__.py".to_string()]
            } else {
                vec![
                    format!("{target_dir}/__init__.py"),
                    format!("{target_dir}.py"),
                ]
            };
            return try_candidates(&candidates, all_paths);
        }
        let (a, b) = if target_dir.is_empty() {
            (
                format!("{rest_path}.py"),
                format!("{rest_path}/__init__.py"),
            )
        } else {
            (
                format!("{target_dir}/{rest_path}.py"),
                format!("{target_dir}/{rest_path}/__init__.py"),
            )
        };
        return try_candidates(&[a, b], all_paths);
    }

    // Absolute import: try several roots
    let candidates = vec![
        format!("{rest_path}.py"),
        format!("{rest_path}/__init__.py"),
        format!("src/{rest_path}.py"),
        format!("src/{rest_path}/__init__.py"),
    ];
    try_candidates(&candidates, all_paths)
}

/// Resolve a TypeScript / JavaScript import specifier to an actual file path.
///
/// Handles `./`, `../`, `@/` (tsconfig alias → `src/`), and index fallbacks.
/// Bare module names (e.g. `"react"`, `"@scope/package"`) return `None`.
fn resolve_ts_import(
    source_path: &str,
    import_source: &str,
    all_paths: &HashSet<&str>,
) -> Option<String> {
    let source_dir = parent_dir(source_path);
    let exts = ["ts", "tsx", "js", "jsx", "mjs"];

    let base: String = if let Some(rest) = import_source.strip_prefix("./") {
        if source_dir.is_empty() {
            rest.to_string()
        } else {
            format!("{source_dir}/{rest}")
        }
    } else if import_source.starts_with("../") {
        // Handle one or more `../` segments
        let mut dir = source_dir.to_string();
        let mut remaining = import_source;
        while let Some(r) = remaining.strip_prefix("../") {
            dir = parent_n(&dir, 1);
            remaining = r;
        }
        if dir.is_empty() {
            remaining.to_string()
        } else {
            format!("{dir}/{remaining}")
        }
    } else if let Some(rest) = import_source.strip_prefix("@/") {
        // Common tsconfig alias mapping `@` → `src`
        format!("src/{rest}")
    } else if import_source.starts_with('.') {
        // Edge case: `.x` without slash
        let trimmed = import_source.trim_start_matches('.');
        if source_dir.is_empty() {
            trimmed.to_string()
        } else {
            format!("{source_dir}/{trimmed}")
        }
    } else {
        // Bare module name or scoped package — external, no resolution
        return None;
    };

    // Try each extension, then /index.{ext}
    let mut candidates: Vec<String> = Vec::with_capacity(exts.len() * 2);
    for ext in &exts {
        candidates.push(format!("{base}.{ext}"));
    }
    for ext in &exts {
        candidates.push(format!("{base}/index.{ext}"));
    }
    try_candidates(&candidates, all_paths)
}

/// Legacy best-effort resolver for languages without a dedicated resolver.
///
/// Converts `::` and `.` separators to `/` and tries common extensions.
fn resolve_legacy(import_source: &str, all_paths: &HashSet<&str>) -> Option<String> {
    let candidate_base = import_source.replace("::", "/").replace('.', "/");
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
    try_candidates(&candidates, all_paths)
}

/// Dispatch to the appropriate per-language resolver based on the source file's extension.
fn resolve_import(
    source_path: &str,
    import_source: &str,
    all_paths: &HashSet<&str>,
) -> Option<String> {
    if import_source.is_empty() {
        return None;
    }
    let ext = source_path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => resolve_rust_import(source_path, import_source, all_paths),
        "py" | "pyi" => resolve_python_import(source_path, import_source, all_paths),
        "ts" | "tsx" | "js" | "jsx" | "mjs" => {
            resolve_ts_import(source_path, import_source, all_paths)
        }
        _ => resolve_legacy(import_source, all_paths),
    }
}

// ──────────────────────────────────────────────────────────────────────────────

/// Build a `DependencyGraph` from indexed files by resolving import source paths
/// to indexed file paths using per-language resolution strategies.
///
/// Rust `crate::`, `super::`, and `self::` prefixes are handled correctly.
/// Python relative imports (leading dots) and TypeScript `./`, `../`, `@/`
/// specifiers are resolved relative to the importing file's directory.
///
/// The optional `schema` parameter enables schema-aware edge injection via
/// `build_schema_edges`.  When present, FK, ORM, embedded-SQL,
/// migration-sequence, view-reference and function-reference edges are added
/// in addition to the import edges derived from parse results.
pub fn build_dependency_graph(
    files: &[IndexedFile],
    schema: Option<&crate::schema::SchemaIndex>,
) -> DependencyGraph {
    let all_paths: HashSet<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    let mut graph = DependencyGraph::new();

    for file in files {
        let Some(pr) = &file.parse_result else {
            continue;
        };

        for import in &pr.imports {
            if let Some(target) = resolve_import(&file.relative_path, &import.source, &all_paths) {
                graph.add_edge(&file.relative_path, &target, EdgeType::Import);
            }
        }
    }

    // Inject schema-aware edges when a schema index is available.
    if let Some(schema_index) = schema {
        let schema_edges = crate::schema::link::build_schema_edges(files, schema_index);
        for (from, to, edge_type) in schema_edges {
            graph.add_edge(&from, &to, edge_type);
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::language::ParseResult;

    // ─── helper ──────────────────────────────────────────────────────────────

    fn make_indexed_file(path: &str, lang: &str, imports: Vec<&str>) -> IndexedFile {
        use crate::parser::language::Import;
        IndexedFile {
            relative_path: path.to_string(),
            language: Some(lang.to_string()),
            size_bytes: 0,
            token_count: 0,
            parse_result: Some(ParseResult {
                symbols: vec![],
                imports: imports
                    .into_iter()
                    .map(|s| Import {
                        source: s.to_string(),
                        names: vec![],
                    })
                    .collect(),
                exports: vec![],
            }),
            content: String::new(),
            mtime_secs: None,
        }
    }

    // ─── helper tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_parent_dir_nested() {
        assert_eq!(parent_dir("src/foo/bar.rs"), "src/foo");
    }

    #[test]
    fn test_parent_dir_top_level() {
        assert_eq!(parent_dir("main.rs"), "");
    }

    #[test]
    fn test_parent_n_two_levels() {
        assert_eq!(parent_n("src/a/b/c", 2), "src/a");
    }

    #[test]
    fn test_parent_n_to_root() {
        assert_eq!(parent_n("src/a", 2), "");
    }

    // ─── Rust resolver ───────────────────────────────────────────────────────

    #[test]
    fn test_resolve_rust_crate_prefix() {
        let all: HashSet<&str> = ["src/foo.rs", "src/foo/mod.rs", "src/bar/baz.rs"]
            .iter()
            .copied()
            .collect();
        assert_eq!(
            resolve_rust_import("src/main.rs", "crate::foo", &all),
            Some("src/foo.rs".to_string())
        );
        assert_eq!(
            resolve_rust_import("src/main.rs", "crate::bar::baz", &all),
            Some("src/bar/baz.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_rust_crate_mod_rs_fallback() {
        let all: HashSet<&str> = ["src/foo/mod.rs"].iter().copied().collect();
        assert_eq!(
            resolve_rust_import("src/main.rs", "crate::foo", &all),
            Some("src/foo/mod.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_rust_self_prefix() {
        let all: HashSet<&str> = [
            "src/intelligence/call_graph.rs",
            "src/intelligence/data_flow.rs",
        ]
        .iter()
        .copied()
        .collect();
        assert_eq!(
            resolve_rust_import("src/intelligence/data_flow.rs", "self::call_graph", &all),
            Some("src/intelligence/call_graph.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_rust_super_prefix() {
        let all: HashSet<&str> = ["src/foo.rs", "src/bar.rs", "src/intelligence/data_flow.rs"]
            .iter()
            .copied()
            .collect();
        assert_eq!(
            resolve_rust_import("src/intelligence/data_flow.rs", "super::foo", &all),
            Some("src/foo.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_rust_double_super() {
        // `super::super` from `src/intelligence/a/b/file.rs`:
        //   - file lives in module `b` (dir: src/intelligence/a/b)
        //   - one `super` → parent of `b` = `a`  (dir: src/intelligence/a)
        //   - two `super` → parent of `a` = `intelligence`  (dir: src/intelligence)
        let all: HashSet<&str> = ["src/intelligence/a/b/file.rs", "src/intelligence/target.rs"]
            .iter()
            .copied()
            .collect();
        assert_eq!(
            resolve_rust_import("src/intelligence/a/b/file.rs", "super::super::target", &all),
            Some("src/intelligence/target.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_rust_external_returns_none() {
        let all: HashSet<&str> = ["src/main.rs"].iter().copied().collect();
        assert_eq!(
            resolve_rust_import("src/main.rs", "std::collections", &all),
            None
        );
        assert_eq!(
            resolve_rust_import("src/main.rs", "serde::Serialize", &all),
            None
        );
        assert_eq!(
            resolve_rust_import("src/main.rs", "tokio::sync::mpsc", &all),
            None
        );
    }

    // ─── Python resolver ──────────────────────────────────────────────────────

    #[test]
    fn test_resolve_python_relative_single_dot() {
        let all: HashSet<&str> = ["app/foo.py", "app/bar.py"].iter().copied().collect();
        assert_eq!(
            resolve_python_import("app/bar.py", ".foo", &all),
            Some("app/foo.py".to_string())
        );
    }

    #[test]
    fn test_resolve_python_relative_double_dot() {
        let all: HashSet<&str> = ["app/utils.py", "app/sub/inner.py"]
            .iter()
            .copied()
            .collect();
        assert_eq!(
            resolve_python_import("app/sub/inner.py", "..utils", &all),
            Some("app/utils.py".to_string())
        );
    }

    #[test]
    fn test_resolve_python_single_dot_bare() {
        // `from . import Y` — resolves to the package __init__.py
        let all: HashSet<&str> = ["app/__init__.py", "app/bar.py"].iter().copied().collect();
        assert_eq!(
            resolve_python_import("app/bar.py", ".", &all),
            Some("app/__init__.py".to_string())
        );
    }

    #[test]
    fn test_resolve_python_absolute_module() {
        let all: HashSet<&str> = ["src/app/config.py"].iter().copied().collect();
        assert_eq!(
            resolve_python_import("tests/test_x.py", "app.config", &all),
            Some("src/app/config.py".to_string())
        );
    }

    #[test]
    fn test_resolve_python_absolute_top_level() {
        let all: HashSet<&str> = ["utils.py"].iter().copied().collect();
        assert_eq!(
            resolve_python_import("main.py", "utils", &all),
            Some("utils.py".to_string())
        );
    }

    // ─── TypeScript / JavaScript resolver ────────────────────────────────────

    #[test]
    fn test_resolve_ts_relative_dot_slash() {
        let all: HashSet<&str> = ["src/a/b.ts", "src/a/c.tsx"].iter().copied().collect();
        assert_eq!(
            resolve_ts_import("src/a/b.ts", "./c", &all),
            Some("src/a/c.tsx".to_string())
        );
    }

    #[test]
    fn test_resolve_ts_relative_dot_dot_slash() {
        let all: HashSet<&str> = ["src/a.ts", "src/b/c.ts"].iter().copied().collect();
        assert_eq!(
            resolve_ts_import("src/b/c.ts", "../a", &all),
            Some("src/a.ts".to_string())
        );
    }

    #[test]
    fn test_resolve_ts_multiple_dot_dot() {
        let all: HashSet<&str> = ["src/util.ts", "src/a/b/c.ts"].iter().copied().collect();
        assert_eq!(
            resolve_ts_import("src/a/b/c.ts", "../../util", &all),
            Some("src/util.ts".to_string())
        );
    }

    #[test]
    fn test_resolve_ts_alias_at_slash() {
        let all: HashSet<&str> = ["src/lib/util.ts"].iter().copied().collect();
        assert_eq!(
            resolve_ts_import("src/app/main.ts", "@/lib/util", &all),
            Some("src/lib/util.ts".to_string())
        );
    }

    #[test]
    fn test_resolve_ts_index_file() {
        let all: HashSet<&str> = ["src/components/index.ts"].iter().copied().collect();
        assert_eq!(
            resolve_ts_import("src/app.ts", "./components", &all),
            Some("src/components/index.ts".to_string())
        );
    }

    #[test]
    fn test_resolve_ts_js_extension() {
        let all: HashSet<&str> = ["src/helpers.js"].iter().copied().collect();
        assert_eq!(
            resolve_ts_import("src/main.ts", "./helpers", &all),
            Some("src/helpers.js".to_string())
        );
    }

    #[test]
    fn test_resolve_ts_external_returns_none() {
        let all: HashSet<&str> = ["src/main.ts"].iter().copied().collect();
        assert_eq!(resolve_ts_import("src/main.ts", "react", &all), None);
        assert_eq!(
            resolve_ts_import("src/main.ts", "@scope/package", &all),
            None
        );
        assert_eq!(resolve_ts_import("src/main.ts", "lodash", &all), None);
    }

    // ─── dispatch ─────────────────────────────────────────────────────────────

    #[test]
    fn test_resolve_import_dispatches_rust() {
        let all: HashSet<&str> = ["src/scanner.rs"].iter().copied().collect();
        assert_eq!(
            resolve_import("src/main.rs", "crate::scanner", &all),
            Some("src/scanner.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_import_dispatches_ts() {
        let all: HashSet<&str> = ["src/utils.ts"].iter().copied().collect();
        assert_eq!(
            resolve_import("src/index.ts", "./utils", &all),
            Some("src/utils.ts".to_string())
        );
    }

    #[test]
    fn test_resolve_import_dispatches_python() {
        let all: HashSet<&str> = ["app/models.py"].iter().copied().collect();
        assert_eq!(
            resolve_import("app/views.py", ".models", &all),
            Some("app/models.py".to_string())
        );
    }

    #[test]
    fn test_resolve_import_empty_source_returns_none() {
        let all: HashSet<&str> = ["src/foo.rs"].iter().copied().collect();
        assert_eq!(resolve_import("src/main.rs", "", &all), None);
    }

    // ─── build_dependency_graph integration ───────────────────────────────────

    #[test]
    fn test_build_dependency_graph_resolves_crate_imports() {
        let files = vec![
            make_indexed_file("src/a.rs", "rust", vec!["crate::b"]),
            make_indexed_file("src/b.rs", "rust", vec![]),
        ];
        let graph = build_dependency_graph(&files, None);
        assert!(
            graph.edges.contains_key("src/a.rs"),
            "a.rs should have outgoing edge"
        );
        let deps = &graph.edges["src/a.rs"];
        assert!(
            deps.iter().any(|e| e.target == "src/b.rs"),
            "a.rs should import b.rs"
        );
    }

    #[test]
    fn test_build_dependency_graph_resolves_super_imports() {
        let files = vec![
            make_indexed_file("src/foo.rs", "rust", vec![]),
            make_indexed_file("src/sub/child.rs", "rust", vec!["super::foo"]),
        ];
        let graph = build_dependency_graph(&files, None);
        let deps = &graph.edges["src/sub/child.rs"];
        assert!(
            deps.iter().any(|e| e.target == "src/foo.rs"),
            "child.rs should import foo.rs via super::"
        );
    }

    #[test]
    fn test_build_dependency_graph_ts_relative() {
        let files = vec![
            make_indexed_file("src/utils.ts", "typescript", vec![]),
            make_indexed_file("src/main.ts", "typescript", vec!["./utils"]),
        ];
        let graph = build_dependency_graph(&files, None);
        let deps = &graph.edges["src/main.ts"];
        assert!(
            deps.iter().any(|e| e.target == "src/utils.ts"),
            "main.ts should import utils.ts"
        );
    }

    #[test]
    fn test_build_dependency_graph_python_relative() {
        let files = vec![
            make_indexed_file("app/models.py", "python", vec![]),
            make_indexed_file("app/views.py", "python", vec![".models"]),
        ];
        let graph = build_dependency_graph(&files, None);
        let deps = &graph.edges["app/views.py"];
        assert!(
            deps.iter().any(|e| e.target == "app/models.py"),
            "views.py should import models.py"
        );
    }

    #[test]
    fn test_build_dependency_graph_external_produces_no_edge() {
        let files = vec![make_indexed_file(
            "src/main.rs",
            "rust",
            vec!["std::collections::HashMap", "serde::Serialize"],
        )];
        let graph = build_dependency_graph(&files, None);
        // No edges for external imports
        assert!(
            graph.edges.get("src/main.rs").is_none_or(|s| s.is_empty()),
            "external imports should not produce edges"
        );
    }

    // ─── existing graph tests ─────────────────────────────────────────────────

    #[test]
    fn test_empty_graph() {
        let graph = DependencyGraph::new();
        assert!(graph.edges.is_empty());
        assert!(graph.dependents("any").is_empty());
        assert!(graph.dependencies("any").is_none());
    }

    #[test]
    fn test_add_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        assert!(graph.edges.contains_key("a.rs"));
        assert!(graph.edges["a.rs"].iter().any(|e| e.target == "b.rs"));
    }

    #[test]
    fn test_dependents() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "b.rs", EdgeType::Import);
        let deps = graph.dependents("b.rs");
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|e| e.target == "a.rs"));
        assert!(deps.iter().any(|e| e.target == "c.rs"));
    }

    #[test]
    fn test_dependencies() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);
        let deps = graph.dependencies("a.rs").unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|e| e.target == "b.rs"));
        assert!(deps.iter().any(|e| e.target == "c.rs"));
    }

    #[test]
    fn test_dependencies_none() {
        let graph = DependencyGraph::new();
        assert!(graph.dependencies("nonexistent").is_none());
    }

    #[test]
    fn test_reachable_from_single() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(reachable.contains("c.rs"));
    }

    #[test]
    fn test_reachable_from_reverse() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["b.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
    }

    #[test]
    fn test_reachable_from_cycle() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("b.rs", "c.rs", EdgeType::Import);
        graph.add_edge("c.rs", "a.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["a.rs"]);
        assert_eq!(reachable.len(), 3);
    }

    #[test]
    fn test_reachable_from_disconnected() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "d.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&["a.rs"]);
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b.rs"));
        assert!(!reachable.contains("c.rs"));
        assert!(!reachable.contains("d.rs"));
    }

    #[test]
    fn test_reachable_from_empty_start() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let reachable = graph.reachable_from(&[]);
        assert!(reachable.is_empty());
    }

    #[test]
    fn test_duplicate_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        assert_eq!(graph.edges["a.rs"].len(), 1);
    }

    #[test]
    fn test_reverse_edges_maintained() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("c.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "d.rs", EdgeType::Import);
        // reverse_edges should exist and be populated
        assert!(graph
            .reverse_edges
            .get("b.rs")
            .unwrap()
            .iter()
            .any(|e| e.target == "a.rs"));
        assert!(graph
            .reverse_edges
            .get("b.rs")
            .unwrap()
            .iter()
            .any(|e| e.target == "c.rs"));
        assert!(graph
            .reverse_edges
            .get("d.rs")
            .unwrap()
            .iter()
            .any(|e| e.target == "a.rs"));
        assert_eq!(graph.reverse_edges.get("b.rs").unwrap().len(), 2);
    }

    #[test]
    fn test_remove_edges_for_file() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);
        graph.add_edge("d.rs", "b.rs", EdgeType::Import);

        graph.remove_edges_for("a.rs");

        // a.rs edges should be gone
        assert!(!graph.edges.contains_key("a.rs"));
        // b.rs should only have d.rs as dependent now
        let b_deps = graph.dependents("b.rs");
        assert_eq!(b_deps.len(), 1);
        assert!(b_deps.iter().any(|e| e.target == "d.rs"));
        // c.rs should have no dependents
        assert!(graph.dependents("c.rs").is_empty());
    }

    #[test]
    fn test_remove_edges_for_nonexistent() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.remove_edges_for("z.rs"); // no-op
        assert_eq!(graph.edges["a.rs"].len(), 1);
    }

    #[test]
    fn test_remove_and_readd_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "c.rs", EdgeType::Import);

        // Simulate re-parse: remove old, add new
        graph.remove_edges_for("a.rs");
        graph.add_edge("a.rs", "d.rs", EdgeType::Import);

        assert_eq!(graph.edges["a.rs"].len(), 1);
        assert!(graph.edges["a.rs"].iter().any(|e| e.target == "d.rs"));
        assert!(graph.dependents("b.rs").is_empty());
        assert!(graph.dependents("c.rs").is_empty());
        let deps = graph.dependents("d.rs");
        assert!(deps.iter().any(|e| e.target == "a.rs") && deps.len() == 1);
    }

    #[test]
    fn test_dependents_large_graph() {
        let mut graph = DependencyGraph::new();
        for i in 0..100 {
            graph.add_edge(&format!("file_{i}.rs"), "common.rs", EdgeType::Import);
        }
        let deps = graph.dependents("common.rs");
        assert_eq!(deps.len(), 100);
    }

    #[test]
    fn test_add_typed_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("models/user.rs", "schema.sql", EdgeType::ForeignKey);
        let deps = graph.dependencies("models/user.rs").unwrap();
        assert!(deps
            .iter()
            .any(|e| e.target == "schema.sql" && e.edge_type == EdgeType::ForeignKey));
    }

    #[test]
    fn test_multiple_edge_types_same_target() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        graph.add_edge("a.rs", "b.rs", EdgeType::ForeignKey);
        // Two different TypedEdges (same target, different edge_type) → both stored
        assert_eq!(graph.edges["a.rs"].len(), 2);
    }

    #[test]
    fn test_dependents_returns_typed_edges() {
        let mut graph = DependencyGraph::new();
        graph.add_edge("a.rs", "b.rs", EdgeType::Import);
        let deps = graph.dependents("b.rs");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].target, "a.rs");
        assert_eq!(deps[0].edge_type, EdgeType::Import);
    }

    #[test]
    fn test_edge_type_local_to_module() {
        // EdgeType lives in src/index/graph.rs (this module) — not in crate::schema.
        // This test asserts the type is reachable via the local module path.
        let _import = EdgeType::Import;
        let _fk = EdgeType::ForeignKey;
    }

    #[test]
    fn test_cross_language_edge_hash() {
        let mut set = HashSet::new();
        set.insert(TypedEdge {
            target: "b.rs".into(),
            edge_type: EdgeType::CrossLanguage(BridgeType::HttpCall),
        });
        set.insert(TypedEdge {
            target: "b.rs".into(),
            edge_type: EdgeType::CrossLanguage(BridgeType::FfiBinding),
        });
        // Same target, different bridge types — both unique edges.
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_edge_type_cross_language_serialization() {
        let variants = [
            BridgeType::HttpCall,
            BridgeType::FfiBinding,
            BridgeType::GrpcCall,
            BridgeType::GraphqlCall,
            BridgeType::SharedSchema,
            BridgeType::CommandExec,
        ];
        for bt in variants {
            let edge = TypedEdge {
                target: "x.py".into(),
                edge_type: EdgeType::CrossLanguage(bt.clone()),
            };
            let json = serde_json::to_string(&edge).unwrap();
            let back: TypedEdge = serde_json::from_str(&json).unwrap();
            assert_eq!(back.edge_type, edge.edge_type);
        }
    }

    #[test]
    fn test_add_cross_language_edge() {
        let mut graph = DependencyGraph::new();
        graph.add_edge(
            "a.ts",
            "b.rs",
            EdgeType::CrossLanguage(BridgeType::FfiBinding),
        );
        let deps = graph.dependencies("a.ts").unwrap();
        assert!(deps.iter().any(|e| e.target == "b.rs"
            && e.edge_type == EdgeType::CrossLanguage(BridgeType::FfiBinding)));
    }

    /// A 3-file Rust project where `a.rs` imports `b.rs` and `c.rs` via `crate::`
    /// must produce exactly 2 Import edges from `a.rs`.
    #[test]
    fn test_build_dependency_graph_3_file_rust_produces_2_edges() {
        let files = vec![
            make_indexed_file("src/a.rs", "rust", vec!["crate::b", "crate::c"]),
            make_indexed_file("src/b.rs", "rust", vec![]),
            make_indexed_file("src/c.rs", "rust", vec![]),
        ];
        let graph = build_dependency_graph(&files, None);
        let deps = graph
            .edges
            .get("src/a.rs")
            .expect("src/a.rs should have outgoing edges");
        assert_eq!(
            deps.len(),
            2,
            "expected exactly 2 edges from src/a.rs, got {}",
            deps.len()
        );
        assert!(
            deps.iter().any(|e| e.target == "src/b.rs"),
            "edge to src/b.rs expected"
        );
        assert!(
            deps.iter().any(|e| e.target == "src/c.rs"),
            "edge to src/c.rs expected"
        );
    }

    /// A project with both Rust and Python files where each language uses its own
    /// import syntax must produce correctly resolved edges for both languages.
    #[test]
    fn test_build_dependency_graph_mixed_rust_and_python_imports_both_resolve() {
        let files = vec![
            // Rust file that imports another Rust file via crate::
            make_indexed_file("src/main.rs", "rust", vec!["crate::lib"]),
            make_indexed_file("src/lib.rs", "rust", vec![]),
            // Python file that imports another Python file via relative import
            make_indexed_file("app/views.py", "python", vec![".models"]),
            make_indexed_file("app/models.py", "python", vec![]),
        ];
        let graph = build_dependency_graph(&files, None);

        // Rust edge: src/main.rs → src/lib.rs
        let rust_deps = graph
            .edges
            .get("src/main.rs")
            .expect("src/main.rs should have edges");
        assert!(
            rust_deps.iter().any(|e| e.target == "src/lib.rs"),
            "src/main.rs should import src/lib.rs"
        );

        // Python edge: app/views.py → app/models.py
        let py_deps = graph
            .edges
            .get("app/views.py")
            .expect("app/views.py should have edges");
        assert!(
            py_deps.iter().any(|e| e.target == "app/models.py"),
            "app/views.py should import app/models.py"
        );
    }
}
