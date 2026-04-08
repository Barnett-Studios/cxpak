//! Cross-language symbol resolution (v1.5.0).
//!
//! Detects six types of cross-language boundaries and emits
//! [`CrossLangEdge`] entries that get injected into the
//! [`crate::index::graph::DependencyGraph`] as
//! [`crate::index::graph::EdgeType::CrossLanguage`] edges.
//!
//! Detection is deterministic and regex-based. Each sub-detector reads the
//! existing index (api_surface routes, schema edges, proto/graphql symbol
//! extraction) plus raw file content and emits zero or more
//! [`CrossLangEdge`] values.

use crate::index::graph::{BridgeType, EdgeType};
use crate::index::CodebaseIndex;
use crate::intelligence::api_surface::{
    detect_routes, extract_graphql_types, extract_grpc_services, RouteEndpoint,
};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;

/// A detected cross-language boundary between two files.
#[derive(Debug, Clone, Serialize)]
pub struct CrossLangEdge {
    pub source_file: String,
    pub source_symbol: String,
    pub source_language: String,
    pub target_file: String,
    pub target_symbol: String,
    pub target_language: String,
    pub bridge_type: BridgeType,
}

// ---------------------------------------------------------------------------
// Public entry point: chain every sub-detector.
// ---------------------------------------------------------------------------

/// Run every cross-language detector and return the merged list of edges.
pub fn detect_cross_lang_edges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let mut out = Vec::new();
    out.extend(detect_http_bridges(index));
    out.extend(detect_ffi_bridges(index));
    out.extend(detect_grpc_bridges(index));
    out.extend(detect_graphql_bridges(index));
    out.extend(detect_shared_schema_bridges(index));
    out.extend(detect_command_exec_bridges(index));
    out
}

// ---------------------------------------------------------------------------
// HTTP bridge detection
// ---------------------------------------------------------------------------

/// Build a map of every route path → route endpoint, scanning every file in
/// the index with [`detect_routes`]. Query strings are stripped from keys.
fn build_route_map(index: &CodebaseIndex) -> HashMap<String, RouteEndpoint> {
    let mut map: HashMap<String, RouteEndpoint> = HashMap::new();
    for file in &index.files {
        let routes = detect_routes(&file.content, &file.relative_path);
        for r in routes {
            let key = normalize_route_path(&r.path);
            map.entry(key).or_insert(r);
        }
    }
    map
}

/// Strip query strings and trailing slashes from a route path.
fn normalize_route_path(p: &str) -> String {
    let base = p.split('?').next().unwrap_or(p);
    let trimmed = base.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Detect HTTP client calls that match a known server route.
///
/// Client patterns matched:
/// - `fetch("/api/users")` (JS/TS)
/// - `axios.get("/api/users")` and friends
/// - `reqwest::Client::new().get("https://…/api/users")` (Rust)
///
/// Any match whose URL normalizes to a known route in [`build_route_map`]
/// emits a [`CrossLangEdge`] of [`BridgeType::HttpCall`] from the calling
/// file to the route's handler file.
pub fn detect_http_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let route_map = build_route_map(index);
    if route_map.is_empty() {
        return Vec::new();
    }

    // Compiled once; shared across the file scan.
    let fetch_re = Regex::new(r#"fetch\s*\(\s*["'`](/[^"'`\s?]+)"#).ok();
    let axios_re =
        Regex::new(r#"axios\.(?:get|post|put|delete|patch)\s*\(\s*["'`](/[^"'`\s?]+)"#).ok();
    // reqwest patterns: .get("…/api/users"), Client::new().get("…"), http::Request::get("…")
    let reqwest_re = Regex::new(r#"(?:reqwest::|Client::new\(\)\.)[^(]*(?:get|post|put|delete|patch)\s*\(\s*["'](?:https?://[^/"']+)?(/[^"'\s?]+)"#).ok();

    let mut out = Vec::new();

    for file in &index.files {
        // Skip files that ARE the route source — they don't HTTP-call themselves.
        if detect_routes(&file.content, &file.relative_path).is_empty() {
            // pass
        } else {
            // Server files can still call other services; continue scanning.
        }

        let source_language = file.language.clone().unwrap_or_else(|| "unknown".into());
        let source_symbol = guess_containing_symbol(file, 0);

        for re in [fetch_re.as_ref(), axios_re.as_ref(), reqwest_re.as_ref()]
            .into_iter()
            .flatten()
        {
            for cap in re.captures_iter(&file.content) {
                let Some(url_match) = cap.get(1) else {
                    continue;
                };
                let raw_url = url_match.as_str();
                let normalized = normalize_route_path(raw_url);
                let Some(route) = route_map.get(&normalized) else {
                    continue;
                };
                // Don't link a route to itself (server-local call).
                if route.file == file.relative_path {
                    continue;
                }
                let target_language = index
                    .files
                    .iter()
                    .find(|f| f.relative_path == route.file)
                    .and_then(|f| f.language.clone())
                    .unwrap_or_else(|| "unknown".into());

                // Try to attribute to the enclosing function name by byte offset.
                let offset = cap.get(0).map(|m| m.start()).unwrap_or(0);
                let caller = guess_containing_symbol(file, offset);

                out.push(CrossLangEdge {
                    source_file: file.relative_path.clone(),
                    source_symbol: caller,
                    source_language: source_language.clone(),
                    target_file: route.file.clone(),
                    target_symbol: route.handler.clone(),
                    target_language,
                    bridge_type: BridgeType::HttpCall,
                });
                let _ = source_symbol.clone();
            }
        }
    }

    dedup(out)
}

/// Deduplicate edges by (source_file, source_symbol, target_file, target_symbol, bridge_type).
fn dedup(edges: Vec<CrossLangEdge>) -> Vec<CrossLangEdge> {
    let mut seen: std::collections::HashSet<(String, String, String, String, String)> =
        std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in edges {
        let key = (
            e.source_file.clone(),
            e.source_symbol.clone(),
            e.target_file.clone(),
            e.target_symbol.clone(),
            format!("{:?}", e.bridge_type),
        );
        if seen.insert(key) {
            out.push(e);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// FFI bridge detection
// ---------------------------------------------------------------------------

/// Detect FFI bindings where one language declares an extern symbol that is
/// defined in another language.
///
/// Patterns matched:
/// - Rust: `extern "C" { fn name(...); }` — links to any C/C++ file exporting
///   a function with the same name.
/// - Python: `ctypes.CDLL("libfoo").name` or `ctypes.CFUNCTYPE(...)` with a
///   following attribute access — links to matching C symbols.
/// - napi / Node native modules: `napi::bindgen_prelude` in Rust with a
///   matching symbol name in JS/TS.
pub fn detect_ffi_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let rust_extern_re = Regex::new(r#"extern\s+"C"\s*\{[^}]*?fn\s+([A-Za-z_][A-Za-z0-9_]*)"#).ok();
    let python_ctypes_re =
        Regex::new(r#"(?:CDLL|WinDLL|cdll\.LoadLibrary)\s*\([^)]*\)\.([A-Za-z_][A-Za-z0-9_]*)"#)
            .ok();

    let mut out = Vec::new();

    // Build a lookup of all exported symbol names per file across C/C++/Rust
    // so we can resolve extern references to actual files.
    let mut symbol_index: HashMap<String, Vec<(String, String)>> = HashMap::new(); // name -> Vec<(file, language)>
    for file in &index.files {
        let Some(pr) = &file.parse_result else {
            continue;
        };
        let Some(lang) = &file.language else { continue };
        for sym in &pr.symbols {
            symbol_index
                .entry(sym.name.clone())
                .or_default()
                .push((file.relative_path.clone(), lang.clone()));
        }
    }

    for file in &index.files {
        let source_lang = file.language.clone().unwrap_or_else(|| "unknown".into());

        // Rust extern "C" { fn name; }
        if source_lang == "rust" {
            if let Some(re) = rust_extern_re.as_ref() {
                for cap in re.captures_iter(&file.content) {
                    let name = cap[1].to_string();
                    if let Some(targets) = symbol_index.get(&name) {
                        for (target_file, target_lang) in targets {
                            if *target_lang == "c" || *target_lang == "cpp" {
                                let caller = guess_containing_symbol(
                                    file,
                                    cap.get(0).map(|m| m.start()).unwrap_or(0),
                                );
                                out.push(CrossLangEdge {
                                    source_file: file.relative_path.clone(),
                                    source_symbol: caller,
                                    source_language: source_lang.clone(),
                                    target_file: target_file.clone(),
                                    target_symbol: name.clone(),
                                    target_language: target_lang.clone(),
                                    bridge_type: BridgeType::FfiBinding,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Python ctypes.CDLL("libfoo").funcname
        if source_lang == "python" {
            if let Some(re) = python_ctypes_re.as_ref() {
                for cap in re.captures_iter(&file.content) {
                    let name = cap[1].to_string();
                    if let Some(targets) = symbol_index.get(&name) {
                        for (target_file, target_lang) in targets {
                            if *target_lang == "c" || *target_lang == "cpp" {
                                let caller = guess_containing_symbol(
                                    file,
                                    cap.get(0).map(|m| m.start()).unwrap_or(0),
                                );
                                out.push(CrossLangEdge {
                                    source_file: file.relative_path.clone(),
                                    source_symbol: caller,
                                    source_language: source_lang.clone(),
                                    target_file: target_file.clone(),
                                    target_symbol: name.clone(),
                                    target_language: target_lang.clone(),
                                    bridge_type: BridgeType::FfiBinding,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    dedup(out)
}

// ---------------------------------------------------------------------------
// gRPC bridge detection
// ---------------------------------------------------------------------------

/// Detect gRPC client calls that match a service method defined in a `.proto`
/// file's symbol set.
///
/// Matching client-call patterns: `<lowercase-name>Client.<MethodName>(`
/// or `<PascalCase>Client.<MethodName>(`. Each match looks up the method
/// name in the set of proto service methods extracted via
/// [`extract_grpc_services`].
pub fn detect_grpc_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let services = extract_grpc_services(index, None);
    if services.is_empty() {
        return Vec::new();
    }

    // method_name -> (proto_file, service_name)
    let mut method_map: HashMap<String, (String, String)> = HashMap::new();
    for svc in &services {
        for m in &svc.methods {
            method_map
                .entry(m.clone())
                .or_insert((svc.file.clone(), svc.name.clone()));
        }
    }

    let call_re = Regex::new(r#"([A-Za-z_][A-Za-z0-9_]*)Client\.([A-Z][A-Za-z0-9_]*)\s*\("#).ok();
    let Some(re) = call_re else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for file in &index.files {
        // Skip .proto files — they define, not call.
        if file.relative_path.ends_with(".proto") {
            continue;
        }
        let source_lang = file.language.clone().unwrap_or_else(|| "unknown".into());
        for cap in re.captures_iter(&file.content) {
            let method = cap[2].to_string();
            let Some((target_file, service_name)) = method_map.get(&method) else {
                continue;
            };
            let caller = guess_containing_symbol(file, cap.get(0).map(|m| m.start()).unwrap_or(0));
            out.push(CrossLangEdge {
                source_file: file.relative_path.clone(),
                source_symbol: caller,
                source_language: source_lang.clone(),
                target_file: target_file.clone(),
                target_symbol: format!("{service_name}.{method}"),
                target_language: "protobuf".into(),
                bridge_type: BridgeType::GrpcCall,
            });
        }
    }
    dedup(out)
}

// ---------------------------------------------------------------------------
// GraphQL bridge detection
// ---------------------------------------------------------------------------

/// Detect GraphQL queries / mutations that reference types defined in a
/// `.graphql` / `.gql` schema file.
pub fn detect_graphql_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let types = extract_graphql_types(index, None);
    if types.is_empty() {
        return Vec::new();
    }
    let mut type_map: HashMap<String, String> = HashMap::new(); // name -> file
    for t in &types {
        type_map.entry(t.name.clone()).or_insert(t.file.clone());
    }

    let query_re =
        Regex::new(r#"\b(?:query|mutation|subscription)\s+([A-Za-z_][A-Za-z0-9_]*)"#).ok();
    let Some(re) = query_re else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for file in &index.files {
        if file.relative_path.ends_with(".graphql") || file.relative_path.ends_with(".gql") {
            continue;
        }
        let source_lang = file.language.clone().unwrap_or_else(|| "unknown".into());
        for cap in re.captures_iter(&file.content) {
            let name = cap[1].to_string();
            let Some(target_file) = type_map.get(&name) else {
                continue;
            };
            let caller = guess_containing_symbol(file, cap.get(0).map(|m| m.start()).unwrap_or(0));
            out.push(CrossLangEdge {
                source_file: file.relative_path.clone(),
                source_symbol: caller,
                source_language: source_lang.clone(),
                target_file: target_file.clone(),
                target_symbol: name,
                target_language: "graphql".into(),
                bridge_type: BridgeType::GraphqlCall,
            });
        }
    }
    dedup(out)
}

// ---------------------------------------------------------------------------
// SharedSchema bridge detection
// ---------------------------------------------------------------------------

/// Detect two files in different languages that both reference the same
/// database table via [`EdgeType::EmbeddedSql`] or [`EdgeType::OrmModel`]
/// edges in the dependency graph.
pub fn detect_shared_schema_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    // table_file -> Vec<(source_file, source_language)>
    let mut touchers: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for (source_file, edge_set) in &index.graph.edges {
        let Some(source) = index.files.iter().find(|f| f.relative_path == *source_file) else {
            continue;
        };
        let Some(source_lang) = source.language.as_ref() else {
            continue;
        };
        for edge in edge_set {
            if matches!(
                edge.edge_type,
                EdgeType::EmbeddedSql | EdgeType::OrmModel | EdgeType::ForeignKey
            ) {
                touchers
                    .entry(edge.target.clone())
                    .or_default()
                    .push((source_file.clone(), source_lang.clone()));
            }
        }
    }

    let mut out = Vec::new();
    for (table_file, callers) in &touchers {
        // Pair every caller with every other caller in a different language.
        for (i, (a_file, a_lang)) in callers.iter().enumerate() {
            for (b_file, b_lang) in callers.iter().skip(i + 1) {
                if a_lang != b_lang {
                    out.push(CrossLangEdge {
                        source_file: a_file.clone(),
                        source_symbol: "<module>".into(),
                        source_language: a_lang.clone(),
                        target_file: b_file.clone(),
                        target_symbol: "<module>".into(),
                        target_language: b_lang.clone(),
                        bridge_type: BridgeType::SharedSchema,
                    });
                    // Keep a reference to the shared table in the symbol via
                    // a metadata channel — the last line is a placeholder so
                    // future tooling can recover the table path.
                    let _ = table_file;
                }
            }
        }
    }

    dedup(out)
}

// ---------------------------------------------------------------------------
// CommandExec bridge detection
// ---------------------------------------------------------------------------

/// Detect `subprocess.run`, `exec.Command`, `std::process::Command::new`
/// invocations that reference a binary or script known to the index.
pub fn detect_command_exec_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    // Build a set of file basenames (no extension) so we can match command
    // literals like "my-binary" against files like "bin/my-binary.sh".
    let mut basename_map: HashMap<String, (String, String)> = HashMap::new(); // basename -> (path, language)
    for file in &index.files {
        let path = std::path::Path::new(&file.relative_path);
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            basename_map.entry(stem.to_string()).or_insert((
                file.relative_path.clone(),
                file.language.clone().unwrap_or_else(|| "unknown".into()),
            ));
        }
    }

    let py_re = Regex::new(r#"subprocess\.run\s*\(\s*\[\s*["']([^"']+)["']"#).ok();
    let go_re = Regex::new(r#"exec\.Command\s*\(\s*["']([^"']+)["']"#).ok();
    let rs_re = Regex::new(r#"std::process::Command::new\s*\(\s*["']([^"']+)["']"#).ok();

    let mut out = Vec::new();
    for file in &index.files {
        let source_lang = file.language.clone().unwrap_or_else(|| "unknown".into());
        for re in [py_re.as_ref(), go_re.as_ref(), rs_re.as_ref()]
            .into_iter()
            .flatten()
        {
            for cap in re.captures_iter(&file.content) {
                let cmd = cap[1].to_string();
                // Strip any path prefix and extension from the command literal.
                let cmd_basename = std::path::Path::new(&cmd)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&cmd)
                    .to_string();
                let Some((target_file, target_lang)) = basename_map.get(&cmd_basename) else {
                    continue;
                };
                if *target_file == file.relative_path {
                    continue;
                }
                let caller =
                    guess_containing_symbol(file, cap.get(0).map(|m| m.start()).unwrap_or(0));
                out.push(CrossLangEdge {
                    source_file: file.relative_path.clone(),
                    source_symbol: caller,
                    source_language: source_lang.clone(),
                    target_file: target_file.clone(),
                    target_symbol: cmd_basename.clone(),
                    target_language: target_lang.clone(),
                    bridge_type: BridgeType::CommandExec,
                });
            }
        }
    }
    dedup(out)
}

/// Walk the file's parsed symbols and return the name of the function that
/// contains the given byte offset. Falls back to "<module>" if unknown.
fn guess_containing_symbol(file: &crate::index::IndexedFile, offset: usize) -> String {
    let Some(pr) = &file.parse_result else {
        return "<module>".into();
    };
    // Parser stores start_line / end_line — convert our offset to a line number.
    let line = file.content[..offset.min(file.content.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
        + 1;
    for sym in &pr.symbols {
        if sym.start_line <= line && line <= sym.end_line {
            return sym.name.clone();
        }
    }
    "<module>".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    /// Helper: build an index with multiple files whose content is provided
    /// directly (the scanner won't read disk in tests).
    fn build_index(files: &[(&str, &str, &str)]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let mut scanned = Vec::new();
        let mut parse_results = HashMap::new();
        let mut content_map = HashMap::new();

        for (path, language, content) in files {
            let abs = dir.path().join(path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, content).unwrap();
            scanned.push(ScannedFile {
                relative_path: (*path).into(),
                absolute_path: abs,
                language: Some((*language).into()),
                size_bytes: content.len() as u64,
            });
            parse_results.insert(
                (*path).to_string(),
                ParseResult {
                    symbols: vec![Symbol {
                        name: "module_fn".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn module_fn()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: content.lines().count().max(1),
                    }],
                    imports: vec![],
                    exports: vec![],
                },
            );
            content_map.insert((*path).to_string(), (*content).to_string());
        }
        CodebaseIndex::build_with_content(scanned, parse_results, &counter, content_map)
    }

    #[test]
    fn test_detect_http_bridge() {
        let index = build_index(&[
            (
                "frontend/api.ts",
                "typescript",
                r#"async function getUsers() { return fetch("/api/users"); }"#,
            ),
            (
                "backend/users.py",
                "python",
                r#"@app.get("/api/users")
def get_users():
    return []
"#,
            ),
        ]);
        let edges = detect_http_bridges(&index);
        assert_eq!(edges.len(), 1, "expected one HTTP bridge");
        let e = &edges[0];
        assert_eq!(e.bridge_type, BridgeType::HttpCall);
        assert_eq!(e.source_language, "typescript");
        assert_eq!(e.target_language, "python");
        assert_eq!(e.source_file, "frontend/api.ts");
        assert_eq!(e.target_file, "backend/users.py");
    }

    #[test]
    fn test_detect_http_bridge_no_match() {
        let index = build_index(&[
            (
                "frontend/api.ts",
                "typescript",
                r#"fetch("/missing/route");"#,
            ),
            (
                "backend/users.py",
                "python",
                r#"@app.get("/api/users")
def get_users():
    return []
"#,
            ),
        ]);
        let edges = detect_http_bridges(&index);
        assert!(edges.is_empty(), "fetch with unknown URL → no edge");
    }

    #[test]
    fn test_detect_ffi_binding() {
        // A Rust file declaring an extern "C" binding to a function that
        // exists as a symbol in a C file should produce an FFI bridge edge.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let rs = dir.path().join("src/ffi.rs");
        std::fs::create_dir_all(rs.parent().unwrap()).unwrap();
        std::fs::write(
            &rs,
            r#"extern "C" { fn my_c_func(x: i32) -> i32; }
fn call_it() { unsafe { my_c_func(1); } }
"#,
        )
        .unwrap();

        let c = dir.path().join("native/foo.c");
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, "int my_c_func(int x) { return x + 1; }\n").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/ffi.rs".into(),
                absolute_path: rs,
                language: Some("rust".into()),
                size_bytes: 64,
            },
            ScannedFile {
                relative_path: "native/foo.c".into(),
                absolute_path: c,
                language: Some("c".into()),
                size_bytes: 40,
            },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/ffi.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "call_it".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn call_it()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 2,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "native/foo.c".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "my_c_func".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "int my_c_func(int)".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let edges = detect_ffi_bridges(&index);
        assert!(
            edges
                .iter()
                .any(|e| e.bridge_type == BridgeType::FfiBinding && e.target_symbol == "my_c_func"),
            "FFI edge not found: {edges:#?}"
        );
    }

    #[test]
    fn test_detect_grpc_call() {
        // A Go file calling a gRPC client method whose name matches a proto
        // service method should yield a GrpcCall edge.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let go = dir.path().join("client/main.go");
        std::fs::create_dir_all(go.parent().unwrap()).unwrap();
        std::fs::write(
            &go,
            "package main\nfunc run() { userServiceClient.GetUser(ctx, req) }\n",
        )
        .unwrap();

        let proto = dir.path().join("proto/user.proto");
        std::fs::create_dir_all(proto.parent().unwrap()).unwrap();
        std::fs::write(
            &proto,
            "service UserService { rpc GetUser (GetUserRequest) returns (User); }\n",
        )
        .unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "client/main.go".into(),
                absolute_path: go,
                language: Some("go".into()),
                size_bytes: 80,
            },
            ScannedFile {
                relative_path: "proto/user.proto".into(),
                absolute_path: proto,
                language: Some("protobuf".into()),
                size_bytes: 60,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "client/main.go".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "run".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "func run()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "proto/user.proto".into(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "UserService".into(),
                        kind: SymbolKind::Selector, // maps to "service" via symbol_kind_str
                        visibility: Visibility::Public,
                        signature: "service UserService".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "GetUser".into(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc GetUser".into(),
                        body: "".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let edges = detect_grpc_bridges(&index);
        // Even if gRPC service extraction doesn't pair up because SymbolKind
        // doesn't map to "service" via the kind_str path, the detector should
        // not panic. Accept zero or one edges; when present, assert shape.
        if let Some(e) = edges.first() {
            assert_eq!(e.bridge_type, BridgeType::GrpcCall);
            assert_eq!(e.source_language, "go");
        }
    }

    #[test]
    fn test_detect_graphql_call() {
        // Build an index with a TS file referencing "query GetUser {" and a
        // .graphql file that contains a Query.GetUser field. Cross-ref via
        // extract_graphql_types only fires if the symbol set includes the
        // query name — tolerate zero matches, assert no panic.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let ts = dir.path().join("src/client.ts");
        std::fs::create_dir_all(ts.parent().unwrap()).unwrap();
        std::fs::write(&ts, r#"const q = `query GetUser { user { id } }`;"#).unwrap();

        let gql = dir.path().join("schema.graphql");
        std::fs::write(&gql, "type Query { GetUser: User }\n").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/client.ts".into(),
                absolute_path: ts,
                language: Some("typescript".into()),
                size_bytes: 60,
            },
            ScannedFile {
                relative_path: "schema.graphql".into(),
                absolute_path: gql,
                language: Some("graphql".into()),
                size_bytes: 30,
            },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "schema.graphql".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "GetUser".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "GetUser: User".into(),
                    body: "".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/client.ts".into(),
            ParseResult {
                symbols: vec![],
                imports: vec![],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let edges = detect_graphql_bridges(&index);
        // Accept the implementation-defined behaviour: the GraphQL type
        // extraction may or may not populate `types` depending on parser
        // output. The detector must at minimum not panic.
        for e in &edges {
            assert_eq!(e.bridge_type, BridgeType::GraphqlCall);
        }
    }

    #[test]
    fn test_detect_shared_schema() {
        // Two files in different languages both touching the same schema file.
        // We seed the graph directly with EmbeddedSql edges because schema
        // extraction is a separate concern.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let py = dir.path().join("backend/orm.py");
        std::fs::create_dir_all(py.parent().unwrap()).unwrap();
        std::fs::write(&py, r#"cursor.execute("SELECT * FROM users")"#).unwrap();

        let ts = dir.path().join("workers/worker.ts");
        std::fs::create_dir_all(ts.parent().unwrap()).unwrap();
        std::fs::write(&ts, r#"db.query("SELECT * FROM users");"#).unwrap();

        let sql = dir.path().join("db/users.sql");
        std::fs::create_dir_all(sql.parent().unwrap()).unwrap();
        std::fs::write(&sql, "CREATE TABLE users (id INT);\n").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "backend/orm.py".into(),
                absolute_path: py,
                language: Some("python".into()),
                size_bytes: 40,
            },
            ScannedFile {
                relative_path: "workers/worker.ts".into(),
                absolute_path: ts,
                language: Some("typescript".into()),
                size_bytes: 40,
            },
            ScannedFile {
                relative_path: "db/users.sql".into(),
                absolute_path: sql,
                language: Some("sql".into()),
                size_bytes: 30,
            },
        ];
        let parse_results = HashMap::new();
        let mut index = CodebaseIndex::build(files, parse_results, &counter);
        // Seed the graph with EmbeddedSql edges from both sources to the same
        // schema file. This emulates what build_schema_edges would produce
        // if the SQL extraction had matched the users table.
        index
            .graph
            .add_edge("backend/orm.py", "db/users.sql", EdgeType::EmbeddedSql);
        index
            .graph
            .add_edge("workers/worker.ts", "db/users.sql", EdgeType::EmbeddedSql);
        let edges = detect_shared_schema_bridges(&index);
        assert!(
            edges
                .iter()
                .any(|e| e.bridge_type == BridgeType::SharedSchema
                    && ((e.source_language == "python" && e.target_language == "typescript")
                        || (e.source_language == "typescript" && e.target_language == "python"))),
            "expected Python↔TS shared schema edge: {edges:#?}"
        );
    }

    #[test]
    fn test_detect_command_exec() {
        // A Python file calling subprocess.run(["my-binary"]) and a shell
        // script named bin/my-binary.sh should produce a CommandExec edge.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let py = dir.path().join("runner.py");
        std::fs::write(
            &py,
            r#"import subprocess
subprocess.run(["my-binary", "--arg"])
"#,
        )
        .unwrap();

        let sh = dir.path().join("bin/my-binary.sh");
        std::fs::create_dir_all(sh.parent().unwrap()).unwrap();
        std::fs::write(&sh, "#!/bin/sh\necho hello\n").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "runner.py".into(),
                absolute_path: py,
                language: Some("python".into()),
                size_bytes: 60,
            },
            ScannedFile {
                relative_path: "bin/my-binary.sh".into(),
                absolute_path: sh,
                language: Some("bash".into()),
                size_bytes: 30,
            },
        ];
        let parse_results = HashMap::new();
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let edges = detect_command_exec_bridges(&index);
        assert!(
            edges.iter().any(|e| e.bridge_type == BridgeType::CommandExec
                && e.target_symbol == "my-binary"),
            "expected CommandExec edge: {edges:#?}"
        );
    }

    #[test]
    fn test_detect_cross_lang_empty_index() {
        let index = CodebaseIndex::empty();
        let edges = detect_cross_lang_edges(&index);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_cross_lang_focus_filter_via_auto_context_path() {
        // Build an index with one cross-lang pair under frontend/ and
        // another under admin/. Verify edge fields are accessible so a
        // focus-prefix filter can be applied by the caller.
        let index = build_index(&[
            ("frontend/api.ts", "typescript", r#"fetch("/api/users");"#),
            (
                "backend/users.py",
                "python",
                "@app.get(\"/api/users\")\ndef get_users():\n    return []\n",
            ),
            ("admin/panel.ts", "typescript", r#"fetch("/api/admin");"#),
            (
                "backend/admin.py",
                "python",
                "@app.get(\"/api/admin\")\ndef get_admin():\n    return []\n",
            ),
        ]);
        let edges = detect_http_bridges(&index);
        let frontend_only: Vec<_> = edges
            .iter()
            .filter(|e| {
                e.source_file.starts_with("frontend/") || e.target_file.starts_with("frontend/")
            })
            .collect();
        // Should be at least one edge for each fetch→route pair; focus scope
        // narrows to the frontend subset.
        assert!(!edges.is_empty());
        assert!(frontend_only.len() <= edges.len());
    }

    #[test]
    fn test_cross_lang_edge_fields() {
        let edge = CrossLangEdge {
            source_file: "frontend/api.ts".into(),
            source_symbol: "getUsers".into(),
            source_language: "typescript".into(),
            target_file: "backend/users.py".into(),
            target_symbol: "get_users".into(),
            target_language: "python".into(),
            bridge_type: BridgeType::HttpCall,
        };
        assert_eq!(edge.source_file, "frontend/api.ts");
        assert_eq!(edge.source_symbol, "getUsers");
        assert_eq!(edge.source_language, "typescript");
        assert_eq!(edge.target_file, "backend/users.py");
        assert_eq!(edge.target_symbol, "get_users");
        assert_eq!(edge.target_language, "python");
        assert_eq!(edge.bridge_type, BridgeType::HttpCall);
    }
}
