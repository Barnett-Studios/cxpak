//! Cross-language symbol resolution (v1.5.0).
//!
//! Detects six types of cross-language boundaries and emits
//! [`CrossLangEdge`] entries that get injected into the
//! [`crate::core_graph::graph::DependencyGraph`] as
//! [`crate::core_graph::graph::EdgeType::CrossLanguage`] edges.
//!
//! Detection is deterministic and regex-based. Each sub-detector reads the
//! existing index (api_surface routes, schema edges, proto/graphql symbol
//! extraction) plus raw file content and emits zero or more
//! [`CrossLangEdge`] values.

use crate::core_graph::graph::{BridgeType, EdgeType};
use crate::core_graph::CodebaseIndex;
use crate::intelligence::api_surface::{
    detect_routes, extract_graphql_types, extract_grpc_services, RouteEndpoint,
};
use regex::Regex;
use std::collections::{HashMap, HashSet};

// `CrossLangEdge` is a data-model type now in `core_graph` (cxpak 3.0.0 Phase 0
// de-cycle); the detection logic below stays here.
pub use crate::core_graph::intel::CrossLangEdge;

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

/// Languages that host web frameworks and therefore legitimately declare HTTP
/// routes or make HTTP client calls. Markdown, JSON, YAML, TOML, SQL, etc.
/// are excluded so documentation files containing code examples don't poison
/// the route map or appear as spurious fetch callers.
fn is_web_code_language(lang: Option<&str>) -> bool {
    matches!(
        lang,
        Some(
            "rust"
                | "typescript"
                | "javascript"
                | "python"
                | "go"
                | "ruby"
                | "java"
                | "kotlin"
                | "csharp"
                | "swift"
                | "php"
                | "clojure"
                | "elixir"
                | "scala"
                | "dart"
                | "cpp"
                | "c"
        )
    )
}

/// Returns true when the file path suggests it's a test file whose string
/// literals are test fixtures, not real code. Skipping these files prevents
/// the HTTP bridge detector from treating `fetch("/api/users")` test
/// fixtures as real client calls.
fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    if lower.starts_with("tests/") || lower.contains("/tests/") {
        return true;
    }
    if lower.contains("/__tests__/") {
        return true;
    }
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    if basename.starts_with("test_") {
        return true;
    }
    for suffix in [
        "_test.rs",
        "_test.go",
        "_test.py",
        ".test.ts",
        ".test.tsx",
        ".test.js",
        ".test.jsx",
        ".spec.ts",
        ".spec.tsx",
        ".spec.js",
        ".spec.jsx",
    ] {
        if basename.ends_with(suffix) {
            return true;
        }
    }
    false
}

/// Return the portion of `content` that should be scanned for cross-language
/// bridges. For Rust source files this strips the `#[cfg(test)] mod tests {
/// ... }` suffix so inline test fixtures containing literal `fetch(...)`
/// strings don't get treated as real HTTP calls.
fn scannable_content<'a>(language: Option<&str>, content: &'a str) -> &'a str {
    if language == Some("rust") {
        if let Some(idx) = content.find("#[cfg(test)]") {
            return &content[..idx];
        }
    }
    content
}

/// Build a map of every route path → the endpoints serving it, scanning every
/// file in the index with [`detect_routes`]. Query strings are stripped from
/// keys.
///
/// The value is a `Vec` and not a single endpoint on purpose: several files can
/// serve one path, and keeping only the first made the graph depend on scan
/// order (#34). Choosing between them is [`resolve_route`]'s job, and it
/// declines rather than guesses.
///
/// Only files with a web-framework-capable language are scanned, and test
/// files are skipped. Inline tests (Rust `#[cfg(test)] mod tests`) are
/// skipped by truncating the scanned content. This keeps documentation
/// code examples and test fixtures from polluting the route map.
fn build_route_map(index: &CodebaseIndex) -> HashMap<String, Vec<RouteEndpoint>> {
    let mut map: HashMap<String, Vec<RouteEndpoint>> = HashMap::new();
    for file in &index.files {
        if !is_web_code_language(file.language.as_deref()) {
            continue;
        }
        if is_test_path(&file.relative_path) {
            continue;
        }
        let content = scannable_content(file.language.as_deref(), &file.content);
        let routes = detect_routes(content, &file.relative_path);
        for r in routes {
            let key = normalize_route_path(&r.path);
            map.entry(key).or_default().push(r);
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

/// The one value `name` maps to, or `None` when several distinct ones do.
///
/// The shape every detector in this module needs (#34). A lookup that answers
/// with the first of several matches is not resolution, it is a coin toss whose
/// result is then reported to PageRank and blast-radius as a fact — and whose
/// outcome moves when file order does.
fn unique_target<'a, T: PartialEq>(map: &'a HashMap<String, Vec<T>>, name: &str) -> Option<&'a T> {
    let all = map.get(name)?;
    let first = all.first()?;
    // Repeats of the SAME target are one answer, not an ambiguity.
    if all.iter().all(|v| v == first) {
        Some(first)
    } else {
        None
    }
}

/// The handler a client call reaches, or `None` when the call does not identify
/// one.
///
/// Refusing is the point (#34). A path several handlers answer to does not say
/// which one a caller reaches, and `or_insert` resolved that by keeping
/// whichever file the scan reached first — a guess, handed to PageRank and
/// blast-radius as a fact, and one that changes when file order does.
///
/// `method` narrows the field when the client states its verb. `fetch` carries
/// its verb in an options object this scanner does not read, so it passes
/// `None` and must resolve on the path alone.
fn resolve_route<'a>(
    candidates: &'a [RouteEndpoint],
    method: Option<&str>,
    calling_file: &str,
) -> Option<&'a RouteEndpoint> {
    // A server calling its own route is not a cross-language edge.
    let mut viable: Vec<&RouteEndpoint> = candidates
        .iter()
        .filter(|r| r.file != calling_file)
        .collect();
    if let Some(m) = method {
        viable.retain(|r| r.method.eq_ignore_ascii_case(m));
    }
    let first = *viable.first()?;
    // Several rows naming the same handler are one answer, not an ambiguity.
    if viable
        .iter()
        .all(|r| r.file == first.file && r.handler == first.handler)
    {
        Some(first)
    } else {
        None
    }
}

/// Detect HTTP client calls that match a known server route.
///
/// Client patterns matched:
/// - `fetch("/api/users")` (JS/TS)
/// - `axios.get("/api/users")` and friends
/// - `reqwest::Client::new().get("https://…/api/users")` (Rust)
///
/// A match whose URL normalizes to a known route emits a [`CrossLangEdge`] of
/// [`BridgeType::HttpCall`] from the calling file to the route's handler file —
/// but only when that route names exactly one handler. Where several do, the
/// call does not say which is reached and no edge is emitted; see
/// [`resolve_route`].
pub fn detect_http_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let route_map = build_route_map(index);
    if route_map.is_empty() {
        return Vec::new();
    }

    // Compiled once; shared across the file scan. Each entry is
    // (pattern, HTTP-verb capture group, path capture group). `fetch` states no
    // verb here — it carries one in an options object this scanner does not
    // read — so its verb group is `None` and it resolves on the path alone.
    let patterns: Vec<(Regex, Option<usize>, usize)> = [
        (r#"fetch\s*\(\s*["'`](/[^"'`\s?]+)"#, None, 1usize),
        (
            r#"axios\.(get|post|put|delete|patch)\s*\(\s*["'`](/[^"'`\s?]+)"#,
            Some(1),
            2,
        ),
        // reqwest: .get("…/api/users"), Client::new().get("…"), http::Request::get("…")
        (
            r#"(?:reqwest::|Client::new\(\)\.)[^(]*(get|post|put|delete|patch)\s*\(\s*["'](?:https?://[^/"']+)?(/[^"'\s?]+)"#,
            Some(1),
            2,
        ),
    ]
    .into_iter()
    .filter_map(|(pat, verb, path)| Regex::new(pat).ok().map(|re| (re, verb, path)))
    .collect();

    let mut out = Vec::new();

    for file in &index.files {
        // Only scan code files — markdown / config / data files may contain
        // example code that looks like a fetch call but isn't.
        if !is_web_code_language(file.language.as_deref()) {
            continue;
        }
        // Skip test files — their string literals are fixtures, not real calls.
        if is_test_path(&file.relative_path) {
            continue;
        }

        let source_language = file.language.clone().unwrap_or_else(|| "unknown".into());
        // Strip the inline test module from Rust files so `fetch(...)` in
        // test fixtures doesn't register as a real HTTP call.
        let content = scannable_content(file.language.as_deref(), &file.content);

        for (re, verb_group, path_group) in &patterns {
            for cap in re.captures_iter(content) {
                let Some(url_match) = cap.get(*path_group) else {
                    continue;
                };
                let raw_url = url_match.as_str();
                let normalized = normalize_route_path(raw_url);
                let Some(candidates) = route_map.get(&normalized) else {
                    continue;
                };
                let method = verb_group.and_then(|g| cap.get(g)).map(|m| m.as_str());
                let Some(route) = resolve_route(candidates, method, &file.relative_path) else {
                    continue;
                };
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

/// The single C/C++ file defining `name`, or `None` when none or several do.
///
/// A bare symbol name is not a binding (#34). `init`, `run`, `start` and their
/// kind are defined in most native code bases more than once, and an
/// `extern "C" { fn init(); }` says nothing about which. Linking to every
/// candidate manufactures one true edge and N-1 false ones; linking to the
/// first manufactures a guess. Neither is knowable from the declaration, so
/// this returns nothing and no edge is emitted.
fn unique_native_target<'a>(
    symbol_index: &'a HashMap<String, Vec<(String, String)>>,
    name: &str,
) -> Option<&'a (String, String)> {
    let mut native = symbol_index
        .get(name)?
        .iter()
        .filter(|(_, lang)| lang == "c" || lang == "cpp");
    let first = native.next()?;
    // The same name twice in ONE file is one target, not an ambiguity.
    match native.find(|(f, _)| *f != first.0) {
        Some(_) => None,
        None => Some(first),
    }
}

/// Detect FFI bindings where one language declares an extern symbol that is
/// defined in another language.
///
/// Patterns matched:
/// - Rust: `extern "C" { fn name(...); }` — links to the C/C++ file exporting
///   a function with that name, and to nothing at all when more than one file
///   exports it (#34). Every `fn` in the block is read, not only the first.
/// - Python: `ctypes.CDLL("libfoo").name` or `ctypes.CFUNCTYPE(...)` with a
///   following attribute access — links to matching C symbols.
/// - napi / Node native modules: `napi::bindgen_prelude` in Rust with a
///   matching symbol name in JS/TS.
pub fn detect_ffi_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    // The block first, then every `fn` inside it. Anchoring the NAME on
    // `extern "C" {` meant `captures_iter` could only ever return the first
    // declaration in a block — the second has no `extern "C" {` of its own left
    // to match, so every later one was dropped in silence (#34).
    let rust_extern_block_re = Regex::new(r#"extern\s+"C"\s*\{([^}]*)\}"#).ok();
    let extern_fn_re = Regex::new(r#"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)"#).ok();
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
            if let (Some(block_re), Some(fn_re)) =
                (rust_extern_block_re.as_ref(), extern_fn_re.as_ref())
            {
                for block in block_re.captures_iter(&file.content) {
                    let Some(body) = block.get(1) else { continue };
                    for cap in fn_re.captures_iter(body.as_str()) {
                        let name = cap[1].to_string();
                        let Some((target_file, target_lang)) =
                            unique_native_target(&symbol_index, &name)
                        else {
                            continue;
                        };
                        let offset = body.start() + cap.get(0).map(|m| m.start()).unwrap_or(0);
                        let caller = guess_containing_symbol(file, offset);
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

        // Python ctypes.CDLL("libfoo").funcname
        if source_lang == "python" {
            if let Some(re) = python_ctypes_re.as_ref() {
                for cap in re.captures_iter(&file.content) {
                    let name = cap[1].to_string();
                    let Some((target_file, target_lang)) =
                        unique_native_target(&symbol_index, &name)
                    else {
                        continue;
                    };
                    let caller =
                        guess_containing_symbol(file, cap.get(0).map(|m| m.start()).unwrap_or(0));
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

    dedup(out)
}

// ---------------------------------------------------------------------------
// gRPC bridge detection
// ---------------------------------------------------------------------------

/// Detect gRPC client calls that match a service method defined in a `.proto`
/// file's symbol set.
///
/// Matching client-call patterns: `<lowercase-name>Client.<MethodName>(`
/// or `<PascalCase>Client.<MethodName>(`. The identifier before `Client` must
/// name a service extracted via [`extract_grpc_services`] (compared without
/// case), and that service must declare the method. Matching on the method name
/// alone linked `httpClient.Get(` and `redisClient.List(` to any proto that
/// happened to share the spelling (#34).
pub fn detect_grpc_bridges(index: &CodebaseIndex) -> Vec<CrossLangEdge> {
    let services = extract_grpc_services(index, None);
    if services.is_empty() {
        return Vec::new();
    }

    // The stub identifier decides the service; the method name alone never did.
    // Keyed on the lowercased service name so `userServiceClient` resolves to
    // `UserService` and `httpClient` resolves to nothing (#34).
    let mut by_service: HashMap<String, (String, String, HashSet<String>)> = HashMap::new();
    for svc in &services {
        let entry = by_service
            .entry(svc.name.to_ascii_lowercase())
            .or_insert_with(|| (svc.file.clone(), svc.name.clone(), HashSet::new()));
        for m in &svc.methods {
            entry.2.insert(m.clone());
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
            let stub = cap[1].to_ascii_lowercase();
            let method = cap[2].to_string();
            let Some((target_file, service_name, methods)) = by_service.get(&stub) else {
                continue;
            };
            if !methods.contains(&method) {
                continue;
            }
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
    // name -> every schema file declaring it. Keeping only the first linked a
    // query to whichever schema was scanned first when two declared the same
    // type name (#34).
    let mut type_map: HashMap<String, Vec<String>> = HashMap::new();
    for t in &types {
        type_map
            .entry(t.name.clone())
            .or_default()
            .push(t.file.clone());
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
            let Some(target_file) = unique_target(&type_map, &name) else {
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
    // basename -> every file with that stem. `main`, `test`, `build` and `run`
    // are stems most repositories carry several of, so keeping only the first
    // attributed `exec.Command("main")` to whichever file was scanned first
    // (#34).
    let mut basename_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for file in &index.files {
        let path = std::path::Path::new(&file.relative_path);
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            basename_map.entry(stem.to_string()).or_default().push((
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
                let Some((target_file, target_lang)) = unique_target(&basename_map, &cmd_basename)
                else {
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
fn guess_containing_symbol(file: &crate::core_graph::IndexedFile, offset: usize) -> String {
    let Some(pr) = &file.parse_result else {
        return "<module>".into();
    };
    // Parser stores start_line / end_line — convert our offset to a line number.
    //
    // Safety: iterating over `.bytes()` and counting `\n` (0x0A) is safe for
    // multi-byte UTF-8 content because `\n` is a single-byte character and
    // can never appear as a continuation byte in a multi-byte sequence.  The
    // slice bound `offset.min(file.content.len())` ensures we never index past
    // the end of the string.  The offsets passed in here originate from
    // `str::find` / `Regex::find` on the same UTF-8 string, so they are always
    // valid char boundaries — no mid-codepoint slicing occurs.
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
    use crate::core_graph::CodebaseIndex;
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
                r#"from fastapi import FastAPI
@app.get("/api/users")
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
                "from fastapi import FastAPI\n@app.get(\"/api/users\")\ndef get_users():\n    return []\n",
            ),
            ("admin/panel.ts", "typescript", r#"fetch("/api/admin");"#),
            (
                "backend/admin.py",
                "python",
                "from fastapi import FastAPI\n@app.get(\"/api/admin\")\ndef get_admin():\n    return []\n",
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

    #[test]
    fn test_is_test_path_matches_dunder_tests_directory() {
        assert!(is_test_path("frontend/__tests__/api.spec.ts"));
        assert!(!is_test_path("frontend/components/api.ts"));
    }

    #[test]
    fn test_normalize_route_path_trims_to_root() {
        // A bare "/" trims to an empty string internally and must normalize
        // back to "/", not "".
        assert_eq!(normalize_route_path("/"), "/");
        assert_eq!(normalize_route_path("/users/"), "/users");
    }

    #[test]
    fn test_detect_http_bridge_skips_self_referencing_route() {
        // A fetch() call to a route registered in the SAME file must not
        // produce a self-referencing edge.
        let index = build_index(&[(
            "backend/api.py",
            "python",
            "from fastapi import FastAPI\n@app.get(\"/api/users\")\ndef get_users():\n    return fetch(\"/api/users\")\n",
        )]);
        let edges = detect_http_bridges(&index);
        assert!(
            edges.is_empty(),
            "a fetch call to a route defined in the same file must not self-link: {edges:#?}"
        );
    }

    #[test]
    fn test_detect_ffi_binding_python_ctypes() {
        // ctypes.CDLL("libfoo.so").my_c_func(...) should resolve to the C
        // file that exports a symbol named `my_c_func`.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let py = dir.path().join("wrapper.py");
        std::fs::write(
            &py,
            "import ctypes\nresult = ctypes.CDLL(\"libfoo.so\").my_c_func(3)\n",
        )
        .unwrap();

        let c = dir.path().join("native/foo.c");
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, "int my_c_func(int x) { return x + 1; }\n").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "wrapper.py".into(),
                absolute_path: py,
                language: Some("python".into()),
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
            "wrapper.py".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "<module>".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "".into(),
                    body: "".into(),
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
            edges.iter().any(|e| e.bridge_type == BridgeType::FfiBinding
                && e.source_file == "wrapper.py"
                && e.target_file == "native/foo.c"
                && e.target_symbol == "my_c_func"
                && e.source_language == "python"
                && e.target_language == "c"),
            "expected Python ctypes.CDLL FFI edge: {edges:#?}"
        );
    }

    #[test]
    fn test_detect_grpc_call_with_service_kinded_symbol() {
        // Unlike `test_detect_grpc_call` (which uses a mis-kinded symbol and
        // tolerates zero matches), this seeds `SymbolKind::Service` so the
        // detector must deterministically produce exactly one edge.
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
                    end_line: 2,
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
                        kind: SymbolKind::Service,
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
        assert_eq!(
            edges.len(),
            1,
            "expected exactly one GrpcCall edge: {edges:#?}"
        );
        let e = &edges[0];
        assert_eq!(e.bridge_type, BridgeType::GrpcCall);
        assert_eq!(e.source_file, "client/main.go");
        assert_eq!(e.source_symbol, "run");
        assert_eq!(e.source_language, "go");
        assert_eq!(e.target_file, "proto/user.proto");
        assert_eq!(e.target_symbol, "UserService.GetUser");
        assert_eq!(e.target_language, "protobuf");
    }

    #[test]
    fn test_detect_graphql_call_with_query_kinded_symbol() {
        // Unlike `test_detect_graphql_call` (which uses a mis-kinded symbol
        // and tolerates zero matches), this seeds `SymbolKind::Query` so the
        // detector must deterministically produce exactly one edge.
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
                    kind: SymbolKind::Query,
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
                symbols: vec![Symbol {
                    name: "<module>".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "".into(),
                    body: "".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let edges = detect_graphql_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "expected exactly one GraphqlCall edge: {edges:#?}"
        );
        let e = &edges[0];
        assert_eq!(e.bridge_type, BridgeType::GraphqlCall);
        assert_eq!(e.source_file, "src/client.ts");
        assert_eq!(e.source_language, "typescript");
        assert_eq!(e.target_file, "schema.graphql");
        assert_eq!(e.target_symbol, "GetUser");
        assert_eq!(e.target_language, "graphql");
    }

    #[test]
    fn test_detect_shared_schema_skips_edges_with_missing_or_langless_source() {
        // Two defensive `continue` branches: an edge keyed by a source_file
        // string that does not correspond to any file in `index.files`, and
        // an edge from a file that IS in `index.files` but has `language:
        // None`. Neither should panic or count as a valid schema toucher.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let py = dir.path().join("known.py");
        std::fs::write(&py, r#"cursor.execute("SELECT * FROM users")"#).unwrap();
        let unknown = dir.path().join("unknown.txt");
        std::fs::write(&unknown, "no language assigned").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "known.py".into(),
                absolute_path: py,
                language: Some("python".into()),
                size_bytes: 40,
            },
            ScannedFile {
                relative_path: "unknown.txt".into(),
                absolute_path: unknown,
                language: None,
                size_bytes: 20,
            },
        ];
        let parse_results = HashMap::new();
        let mut index = CodebaseIndex::build(files, parse_results, &counter);

        // Source file present in index.files but with no language.
        index
            .graph
            .add_edge("unknown.txt", "db/schema.sql", EdgeType::EmbeddedSql);
        // Source file string with no corresponding entry in index.files at all.
        index
            .graph
            .add_edge("ghost.py", "db/schema.sql", EdgeType::EmbeddedSql);
        // One legitimate toucher — without a second, *different-language*
        // toucher counted alongside it, no pair can form.
        index
            .graph
            .add_edge("known.py", "db/schema.sql", EdgeType::EmbeddedSql);

        let edges = detect_shared_schema_bridges(&index);
        assert!(
            edges.is_empty(),
            "missing-file and langless-file edges must be skipped, leaving only \
             one (insufficient for a pair) toucher: {edges:#?}"
        );
    }

    #[test]
    fn test_detect_command_exec_skips_self_target() {
        // A file whose content matches its own basename as the command
        // target must not produce a self-referencing CommandExec edge.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let sh = dir.path().join("bin/my-binary.sh");
        std::fs::create_dir_all(sh.parent().unwrap()).unwrap();
        std::fs::write(
            &sh,
            "std::process::Command::new(\"my-binary\").spawn().unwrap();\n",
        )
        .unwrap();

        let files = vec![ScannedFile {
            relative_path: "bin/my-binary.sh".into(),
            absolute_path: sh,
            language: Some("bash".into()),
            size_bytes: 60,
        }];
        let parse_results = HashMap::new();
        let index = CodebaseIndex::build(files, parse_results, &counter);
        let edges = detect_command_exec_bridges(&index);
        assert!(
            edges.is_empty(),
            "a file invoking a command matching its own basename must not self-link: {edges:#?}"
        );
    }

    #[test]
    fn test_guess_containing_symbol_falls_back_to_module_outside_symbol_ranges() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let content = "fn foo() {}\n\nfn bar() {}\n";
        let fp = dir.path().join("a.rs");
        std::fs::write(&fp, content).unwrap();
        let scanned = ScannedFile {
            relative_path: "a.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        };
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "a.rs".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "foo".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn foo()".into(),
                        body: "{}".into(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "bar".into(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "fn bar()".into(),
                        body: "{}".into(),
                        start_line: 3,
                        end_line: 3,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        let mut content_map = HashMap::new();
        content_map.insert("a.rs".to_string(), content.to_string());
        let index =
            CodebaseIndex::build_with_content(vec![scanned], parse_results, &counter, content_map);
        let file = index
            .files
            .iter()
            .find(|f| f.relative_path == "a.rs")
            .unwrap();
        // The blank line 2 is covered by neither `foo` (1..1) nor `bar` (3..3).
        let offset = content.find("\n\nfn bar").unwrap() + 1;
        assert_eq!(
            guess_containing_symbol(file, offset),
            "<module>",
            "an offset outside every symbol's line range must fall back to <module>"
        );
    }

    // ── #34: an edge asserts a binding, so a name more than one thing answers
    //    to must produce none ──────────────────────────────────────────────
    //
    // Every defect in this ticket is one move: a bare name is matched, several
    // things answer to it, and one is chosen by iteration order. The choice is
    // then handed to PageRank and blast-radius as a fact. Each test below is
    // paired with a control, because "emit nothing when ambiguous" is trivially
    // satisfied by emitting nothing at all.

    /// `build_index` above hardcodes every file's symbol to `module_fn`, which
    /// cannot express a name collision — the subject of every test here.
    /// (relative path, language, content, the symbols the file declares).
    type FileSpec<'a> = (&'a str, &'a str, &'a str, &'a [(&'a str, SymbolKind)]);

    fn index_with_symbols(files: &[FileSpec<'_>]) -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let mut scanned = Vec::new();
        let mut parse_results = HashMap::new();
        let mut content_map = HashMap::new();

        for (path, language, content, symbols) in files {
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
                    symbols: symbols
                        .iter()
                        .map(|(n, k)| Symbol {
                            name: (*n).into(),
                            kind: k.clone(),
                            visibility: Visibility::Public,
                            signature: format!("fn {n}()"),
                            body: "{}".into(),
                            start_line: 1,
                            end_line: content.lines().count().max(1),
                        })
                        .collect(),
                    imports: vec![],
                    exports: vec![],
                },
            );
            content_map.insert((*path).to_string(), (*content).to_string());
        }
        CodebaseIndex::build_with_content(scanned, parse_results, &counter, content_map)
    }

    const F: SymbolKind = SymbolKind::Function;

    // ---- FFI -------------------------------------------------------------

    #[test]
    fn every_extern_fn_in_one_block_resolves_not_only_the_first() {
        let index = index_with_symbols(&[
            (
                "src/ffi.rs",
                "rust",
                "extern \"C\" {\n    fn alpha(x: i32) -> i32;\n    fn beta(y: i32) -> i32;\n}\n",
                &[("call_it", F)],
            ),
            (
                "native/lib.c",
                "c",
                "int alpha(int x) { return x; }\nint beta(int y) { return y; }\n",
                &[("alpha", F), ("beta", F)],
            ),
        ]);
        let mut names: Vec<String> = detect_ffi_bridges(&index)
            .into_iter()
            .map(|e| e.target_symbol)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["alpha".to_string(), "beta".to_string()],
            "the block declares two externs; a pattern anchored on the opening brace can only \
             ever capture the first, and every later declaration is dropped in silence"
        );
    }

    #[test]
    fn an_extern_name_two_c_files_answer_to_creates_no_ffi_edge() {
        let index = index_with_symbols(&[
            (
                "src/ffi.rs",
                "rust",
                "extern \"C\" {\n    fn init();\n}\n",
                &[("call_it", F)],
            ),
            (
                "native/audio.c",
                "c",
                "int init(void) { return 0; }\n",
                &[("init", F)],
            ),
            (
                "native/video.c",
                "c",
                "int init(void) { return 1; }\n",
                &[("init", F)],
            ),
        ]);
        let edges = detect_ffi_bridges(&index);
        assert!(
            edges.is_empty(),
            "two C files define `init` and nothing in the Rust declaration says which is meant, \
             so any edge here is a guess reported as a fact — got {edges:?}"
        );
    }

    #[test]
    fn an_extern_name_exactly_one_c_file_answers_to_still_resolves() {
        let index = index_with_symbols(&[
            (
                "src/ffi.rs",
                "rust",
                "extern \"C\" {\n    fn init();\n}\n",
                &[("call_it", F)],
            ),
            (
                "native/audio.c",
                "c",
                "int init(void) { return 0; }\n",
                &[("init", F)],
            ),
            (
                "native/video.c",
                "c",
                "int teardown(void) { return 1; }\n",
                &[("teardown", F)],
            ),
        ]);
        let edges = detect_ffi_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "one unambiguous target must still resolve, or the fix is just silence — got {edges:?}"
        );
        assert_eq!(edges[0].target_file, "native/audio.c");
    }

    // ---- HTTP routes -----------------------------------------------------

    #[test]
    fn two_backends_serving_one_path_create_no_http_edge() {
        let index = index_with_symbols(&[
            (
                "web/app.ts",
                "typescript",
                "async function ping() { return fetch(\"/api/health\"); }",
                &[("ping", F)],
            ),
            (
                "svc_a/main.py",
                "python",
                "from fastapi import FastAPI\n@app.get(\"/api/health\")\ndef health_a():\n    return {}\n",
                &[("health_a", F)],
            ),
            (
                "svc_b/main.py",
                "python",
                "from fastapi import FastAPI\n@app.get(\"/api/health\")\ndef health_b():\n    return {}\n",
                &[("health_b", F)],
            ),
        ]);
        let edges = detect_http_bridges(&index);
        assert!(
            edges.is_empty(),
            "both services answer GET /api/health; which one the client reaches is not knowable \
             from the path, and first-insertion-wins makes the answer depend on scan order — \
             got {edges:?}"
        );
    }

    #[test]
    fn one_backend_serving_the_path_still_creates_an_http_edge() {
        let index = index_with_symbols(&[
            (
                "web/app.ts",
                "typescript",
                "async function ping() { return fetch(\"/api/health\"); }",
                &[("ping", F)],
            ),
            (
                "svc_a/main.py",
                "python",
                "from fastapi import FastAPI\n@app.get(\"/api/health\")\ndef health_a():\n    return {}\n",
                &[("health_a", F)],
            ),
        ]);
        let edges = detect_http_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "an unambiguous route must still link — got {edges:?}"
        );
        assert_eq!(edges[0].target_file, "svc_a/main.py");
    }

    #[test]
    fn a_get_call_is_not_ambiguous_against_a_same_path_post_handler() {
        // A control for the refusal above: same path, different verbs, and the
        // client states its verb. Refusing here would trade a false edge for a
        // lost one.
        let index = index_with_symbols(&[
            (
                "web/app.ts",
                "typescript",
                "async function load() { return axios.get(\"/api/items\"); }",
                &[("load", F)],
            ),
            (
                "svc_read/main.py",
                "python",
                "from fastapi import FastAPI\n@app.get(\"/api/items\")\ndef list_items():\n    return []\n",
                &[("list_items", F)],
            ),
            (
                "svc_write/main.py",
                "python",
                "from fastapi import FastAPI\n@app.post(\"/api/items\")\ndef create_item():\n    return {}\n",
                &[("create_item", F)],
            ),
        ]);
        let edges = detect_http_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "the verb disambiguates these two — got {edges:?}"
        );
        assert_eq!(edges[0].target_file, "svc_read/main.py");
    }

    // ---- gRPC ------------------------------------------------------------

    #[test]
    fn a_client_identifier_naming_no_service_creates_no_grpc_edge() {
        let index = index_with_symbols(&[
            (
                "client/main.go",
                "go",
                "package main\nfunc run() { httpClient.Get(url) }\n",
                &[("run", F)],
            ),
            (
                "proto/user.proto",
                "protobuf",
                "service UserService { rpc Get (Req) returns (Res); }\n",
                &[
                    ("UserService", SymbolKind::Service),
                    ("Get", SymbolKind::Method),
                ],
            ),
        ]);
        let edges = detect_grpc_bridges(&index);
        assert!(
            edges.is_empty(),
            "`httpClient` is not a stub for `UserService`; the only thing linking them is that \
             both have a method spelled `Get` — got {edges:?}"
        );
    }

    #[test]
    fn a_stub_client_naming_its_service_still_creates_a_grpc_edge() {
        let index = index_with_symbols(&[
            (
                "client/main.go",
                "go",
                "package main\nfunc run() { userServiceClient.Get(ctx) }\n",
                &[("run", F)],
            ),
            (
                "proto/user.proto",
                "protobuf",
                "service UserService { rpc Get (Req) returns (Res); }\n",
                &[
                    ("UserService", SymbolKind::Service),
                    ("Get", SymbolKind::Method),
                ],
            ),
        ]);
        let edges = detect_grpc_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "a real stub call must still link — got {edges:?}"
        );
        assert_eq!(edges[0].target_symbol, "UserService.Get");
    }

    #[test]
    fn a_stub_calling_a_method_its_service_does_not_declare_creates_no_edge() {
        // Resolving the stub is half the check. `UserService` has no `Delete`,
        // so naming the right service does not make this a call to it.
        let index = index_with_symbols(&[
            (
                "client/main.go",
                "go",
                "package main\nfunc run() { userServiceClient.Delete(ctx) }\n",
                &[("run", F)],
            ),
            (
                "proto/user.proto",
                "protobuf",
                "service UserService { rpc Get (Req) returns (Res); }\n",
                &[
                    ("UserService", SymbolKind::Service),
                    ("Get", SymbolKind::Method),
                ],
            ),
        ]);
        let edges = detect_grpc_bridges(&index);
        assert!(
            edges.is_empty(),
            "the service resolves but does not declare `Delete` — got {edges:?}"
        );
    }

    #[test]
    fn two_rows_naming_one_handler_are_one_answer_not_an_ambiguity() {
        // The refusal is about not knowing WHICH handler, so two rows that name
        // the same handler must still resolve. Asserted on the helper directly:
        // producing a duplicate row through `detect_routes` depends on framework
        // syntax this test has no business pinning.
        let same = |method: &str| RouteEndpoint {
            method: method.to_string(),
            path: "/api/items".to_string(),
            handler: "list_items".to_string(),
            file: "svc/main.py".to_string(),
            line: 1,
        };
        let candidates = vec![same("GET"), same("HEAD")];
        let got = resolve_route(&candidates, None, "web/app.ts");
        assert!(
            got.is_some(),
            "both rows point at svc/main.py::list_items, so the handler is not in doubt"
        );

        let elsewhere = RouteEndpoint {
            file: "other/main.py".to_string(),
            handler: "other_items".to_string(),
            ..same("GET")
        };
        assert!(
            resolve_route(&[same("GET"), elsewhere], None, "web/app.ts").is_none(),
            "two different handlers is the case that must refuse"
        );
    }

    // ---- the two detectors #34 does not name, same defect ----------------

    #[test]
    fn a_graphql_type_two_schemas_declare_creates_no_edge() {
        let index = index_with_symbols(&[
            (
                "web/query.ts",
                "typescript",
                "const q = gql`query GetUser { user { id } }`;\n",
                &[("q", F)],
            ),
            (
                "schema/a.graphql",
                "graphql",
                "type GetUser { id: ID }\n",
                &[("GetUser", SymbolKind::Type)],
            ),
            (
                "schema/b.graphql",
                "graphql",
                "type GetUser { id: ID }\n",
                &[("GetUser", SymbolKind::Type)],
            ),
        ]);
        let edges = detect_graphql_bridges(&index);
        assert!(
            edges.is_empty(),
            "two schemas declare `GetUser`; the query does not say which — got {edges:?}"
        );
    }

    #[test]
    fn a_graphql_type_one_schema_declares_still_creates_an_edge() {
        let index = index_with_symbols(&[
            (
                "web/query.ts",
                "typescript",
                "const q = gql`query GetUser { user { id } }`;\n",
                &[("q", F)],
            ),
            (
                "schema/a.graphql",
                "graphql",
                "type GetUser { id: ID }\n",
                &[("GetUser", SymbolKind::Type)],
            ),
        ]);
        let edges = detect_graphql_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "an unambiguous type must still link — got {edges:?}"
        );
        assert_eq!(edges[0].target_file, "schema/a.graphql");
    }

    #[test]
    fn a_command_basename_two_files_answer_to_creates_no_edge() {
        let index = index_with_symbols(&[
            (
                "svc/app.py",
                "python",
                "import subprocess\ndef go():\n    subprocess.run([\"helper\"])\n",
                &[("go", F)],
            ),
            (
                "tools/helper.sh",
                "shell",
                "#!/bin/sh\necho a\n",
                &[("main", F)],
            ),
            ("bin/helper.py", "python", "print('b')\n", &[("main", F)]),
        ]);
        let edges = detect_command_exec_bridges(&index);
        assert!(
            edges.is_empty(),
            "`helper` is the stem of two files and the literal says nothing about which is \
             executed — got {edges:?}"
        );
    }

    #[test]
    fn a_command_basename_one_file_answers_to_still_creates_an_edge() {
        let index = index_with_symbols(&[
            (
                "svc/app.py",
                "python",
                "import subprocess\ndef go():\n    subprocess.run([\"helper\"])\n",
                &[("go", F)],
            ),
            (
                "tools/helper.sh",
                "shell",
                "#!/bin/sh\necho a\n",
                &[("main", F)],
            ),
        ]);
        let edges = detect_command_exec_bridges(&index);
        assert_eq!(
            edges.len(),
            1,
            "one unambiguous binary must still link — got {edges:?}"
        );
        assert_eq!(edges[0].target_file, "tools/helper.sh");
    }
}
