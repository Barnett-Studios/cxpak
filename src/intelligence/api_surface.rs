use crate::index::CodebaseIndex;
use crate::parser::language::{SymbolKind, Visibility};
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiSurface {
    pub symbols: SymbolSection,
    pub routes: RouteSection,
    pub grpc_services: Vec<GrpcService>,
    pub graphql_types: Vec<GraphqlType>,
    pub token_count: usize,
}

#[derive(Debug, Serialize)]
pub struct SymbolSection {
    pub total: usize,
    pub by_file: Vec<FileSymbols>,
}

#[derive(Debug, Serialize)]
pub struct FileSymbols {
    pub path: String,
    pub pagerank: f64,
    pub symbols: Vec<ApiSymbol>,
}

#[derive(Debug, Serialize)]
pub struct ApiSymbol {
    pub name: String,
    pub kind: String,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RouteSection {
    pub total: usize,
    pub endpoints: Vec<RouteEndpoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteEndpoint {
    pub method: String,
    pub path: String,
    pub handler: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct GrpcService {
    pub name: String,
    pub file: String,
    pub methods: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GraphqlType {
    pub name: String,
    pub kind: String,
    pub file: String,
}

fn symbol_kind_str(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Class => "class",
        SymbolKind::Method => "method",
        SymbolKind::Constant => "constant",
        SymbolKind::TypeAlias => "type_alias",
        SymbolKind::Selector => "selector",
        SymbolKind::Mixin => "mixin",
        SymbolKind::Variable => "variable",
        SymbolKind::Heading => "heading",
        SymbolKind::Section => "section",
        SymbolKind::Key => "key",
        SymbolKind::Table => "table",
        SymbolKind::Block => "block",
        SymbolKind::Target => "target",
        SymbolKind::Rule => "rule",
        SymbolKind::Element => "element",
        SymbolKind::Message => "message",
        SymbolKind::Service => "service",
        SymbolKind::Query => "query",
        SymbolKind::Mutation => "mutation",
        SymbolKind::Type => "type",
        SymbolKind::Instruction => "instruction",
    }
}

/// Extract the public API surface: public symbols sorted by PageRank, filtered by focus.
pub fn extract_public_symbols(
    index: &CodebaseIndex,
    focus: Option<&str>,
) -> (SymbolSection, usize) {
    let mut file_entries: Vec<FileSymbols> = vec![];
    let mut total_symbols = 0usize;
    let mut token_count = 0usize;

    // Collect files with public symbols, sorted by pagerank descending.
    let mut files_with_symbols: Vec<(&crate::index::IndexedFile, f64)> = index
        .files
        .iter()
        .filter(|f| {
            // Apply focus filter.
            if let Some(prefix) = focus {
                if !f.relative_path.starts_with(prefix) {
                    return false;
                }
            }
            // Must have at least one public symbol.
            f.parse_result
                .as_ref()
                .map(|pr| {
                    pr.symbols
                        .iter()
                        .any(|s| s.visibility == Visibility::Public)
                })
                .unwrap_or(false)
        })
        .map(|f| {
            let pr = index.pagerank.get(&f.relative_path).copied().unwrap_or(0.0);
            (f, pr)
        })
        .collect();

    files_with_symbols.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (file, pagerank) in files_with_symbols {
        let pr = file.parse_result.as_ref().unwrap();
        let symbols: Vec<ApiSymbol> = pr
            .symbols
            .iter()
            .filter(|s| s.visibility == Visibility::Public)
            .map(|s| ApiSymbol {
                name: s.name.clone(),
                kind: symbol_kind_str(&s.kind).to_string(),
                signature: s.signature.clone(),
                doc: None,
            })
            .collect();

        total_symbols += symbols.len();
        // Rough token estimate: 5 tokens per symbol (name + kind + signature words).
        token_count += symbols
            .iter()
            .map(|s| s.signature.split_whitespace().count() + 2)
            .sum::<usize>();

        file_entries.push(FileSymbols {
            path: file.relative_path.clone(),
            pagerank,
            symbols,
        });
    }

    (
        SymbolSection {
            total: total_symbols,
            by_file: file_entries,
        },
        token_count,
    )
}

/// Detect HTTP route endpoints from file content using 12 framework patterns.
pub fn detect_routes(content: &str, file_path: &str) -> Vec<RouteEndpoint> {
    let mut routes = vec![];

    // Helper to compute 1-based line number from a byte offset.
    let line_of =
        |offset: usize| -> usize { content[..offset].chars().filter(|&c| c == '\n').count() + 1 };

    // 1. Express/Koa: (app|router).(get|post|put|delete|patch)("/<path>", handlerName)
    if let Ok(re) = Regex::new(
        r#"(?i)(app|router)\.(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*)["']\s*,\s*([a-zA-Z_$][a-zA-Z0-9_$]*)"#,
    ) {
        let js_keywords = ["function", "async", "new", "class", "return"];
        for cap in re.captures_iter(content) {
            let method = cap[2].to_uppercase();
            let path = cap[3].to_string();
            let raw_handler = cap[4].to_string();
            let handler = if js_keywords.contains(&raw_handler.as_str()) {
                "<anonymous>".to_string()
            } else {
                raw_handler
            };
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }

    // 2. Flask: @(app|blueprint).(route|get|post|put|delete)("/<path>") followed by def func_name
    if let Ok(re) = Regex::new(
        r#"(?i)@(app|blueprint)\.(route|get|post|put|delete)\s*\(\s*["'](/[^"']*?)["'][^)]*\)\s*\n\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
    ) {
        for cap in re.captures_iter(content) {
            let method_or_route = cap[2].to_lowercase();
            let method = if method_or_route == "route" {
                "GET".to_string()
            } else {
                method_or_route.to_uppercase()
            };
            let path = cap[3].to_string();
            let handler = cap[4].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }
    // Flask fallback: decorator without def on next line
    if let Ok(re) =
        Regex::new(r#"(?i)@(app|blueprint)\.(route|get|post|put|delete)\s*\(\s*["'](/[^"']*)"#)
    {
        for cap in re.captures_iter(content) {
            let method_or_route = cap[2].to_lowercase();
            let method = if method_or_route == "route" {
                "GET".to_string()
            } else {
                method_or_route.to_uppercase()
            };
            let path = cap[3].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            if !routes
                .iter()
                .any(|r| r.line == line && r.method == method && r.path == path)
            {
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler: "<anonymous>".to_string(),
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 3. Django: path("/<path>", view_func) — only in files containing "urls" in the path
    if file_path.contains("urls") {
        if let Ok(re) =
            Regex::new(r#"(?i)path\s*\(\s*["']([^"']*)["']\s*,\s*([a-zA-Z_][a-zA-Z0-9_.]*)"#)
        {
            for cap in re.captures_iter(content) {
                let path_val = cap[1].to_string();
                let handler = cap[2].to_string();
                let line = line_of(cap.get(0).unwrap().start());
                routes.push(RouteEndpoint {
                    method: "GET".to_string(),
                    path: format!("/{}", path_val.trim_start_matches('/')),
                    handler,
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 4. FastAPI: @(app|router).(get|post|put|delete|patch)("/<path>") followed by def func_name
    if let Ok(re) = Regex::new(
        r#"(?i)@(app|router)\.(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*?)["'][^)]*\)\s*\n\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
    ) {
        for cap in re.captures_iter(content) {
            let method = cap[2].to_uppercase();
            let path = cap[3].to_string();
            let handler = cap[4].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }
    // FastAPI fallback: decorator without def on next line
    if let Ok(re) =
        Regex::new(r#"(?i)@(app|router)\.(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*)"#)
    {
        for cap in re.captures_iter(content) {
            let method = cap[2].to_uppercase();
            let path = cap[3].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            if !routes
                .iter()
                .any(|r| r.line == line && r.method == method && r.path == path)
            {
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler: "<anonymous>".to_string(),
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 5. Spring: @(Get|Post|Put|Delete|Patch|Request)Mapping("/<path>") followed by method name
    if let Ok(re) = Regex::new(
        r#"(?s)@(Get|Post|Put|Delete|Patch|Request)Mapping\s*\(\s*["'](/[^"']*?)["'][^)]*\)\s*(?:public\s+)?(?:\S+\s+)+?([a-zA-Z_][a-zA-Z0-9_]*)\s*\("#,
    ) {
        for cap in re.captures_iter(content) {
            let verb = cap[1].to_lowercase();
            let method = if verb == "request" {
                "GET".to_string()
            } else {
                verb.to_uppercase()
            };
            let path = cap[2].to_string();
            let handler = cap[3].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }
    // Spring fallback: no method signature found after annotation
    if let Ok(re) =
        Regex::new(r#"@(Get|Post|Put|Delete|Patch|Request)Mapping\s*\(\s*["'](/[^"']*)"#)
    {
        for cap in re.captures_iter(content) {
            let verb = cap[1].to_lowercase();
            let method = if verb == "request" {
                "GET".to_string()
            } else {
                verb.to_uppercase()
            };
            let path = cap[2].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            if !routes
                .iter()
                .any(|r| r.line == line && r.method == method && r.path == path)
            {
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler: "<anonymous>".to_string(),
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 6. actix-web: #[get("/path")] followed by fn handler_name
    if let Ok(re) = Regex::new(
        r#"(?s)#\[(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*?)["']\s*\)\s*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
    ) {
        for cap in re.captures_iter(content) {
            let method = cap[1].to_uppercase();
            let path = cap[2].to_string();
            let handler = cap[3].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }
    // actix-web fallback: attribute without fn on next line
    if let Ok(re) = Regex::new(r#"#\[(get|post|put|delete|patch)\s*\(\s*["'](/[^"']*)"#) {
        for cap in re.captures_iter(content) {
            let method = cap[1].to_uppercase();
            let path = cap[2].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            if !routes
                .iter()
                .any(|r| r.line == line && r.method == method && r.path == path)
            {
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler: "<anonymous>".to_string(),
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 7. axum: .route("/path", get(handler_name))
    if let Ok(re) = Regex::new(
        r#"\.route\s*\(\s*["'](/[^"']*?)["']\s*,\s*(?:get|post|put|delete|patch|any)\(([a-zA-Z_][a-zA-Z0-9_]*)\)"#,
    ) {
        for cap in re.captures_iter(content) {
            let path = cap[1].to_string();
            let handler = cap[2].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method: "GET".to_string(),
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }
    // axum fallback: .route("/path") without handler capture
    if let Ok(re) = Regex::new(r#"\.route\s*\(\s*["'](/[^"']*)"#) {
        for cap in re.captures_iter(content) {
            let path = cap[1].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            if !routes.iter().any(|r| r.line == line && r.path == path) {
                routes.push(RouteEndpoint {
                    method: "GET".to_string(),
                    path,
                    handler: "<anonymous>".to_string(),
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 8. Gin: (r|router|group).(GET|POST|PUT|DELETE|PATCH)("/path", handlerFunc)
    if let Ok(re) = Regex::new(
        r#"(?i)(r|router|group)\.(GET|POST|PUT|DELETE|PATCH)\s*\(\s*["'](/[^"']*?)["']\s*,\s*([a-zA-Z_][a-zA-Z0-9_]*)"#,
    ) {
        for cap in re.captures_iter(content) {
            let method = cap[2].to_uppercase();
            let path = cap[3].to_string();
            let handler = cap[4].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }

    // 9. Echo: (e|g|echo|group).(GET|POST|PUT|DELETE|PATCH)("/path", handlerFunc)
    if let Ok(re) = Regex::new(
        r#"(?i)(e|g|echo|group)\.(GET|POST|PUT|DELETE|PATCH)\s*\(\s*["'](/[^"']*?)["']\s*,\s*([a-zA-Z_][a-zA-Z0-9_]*)"#,
    ) {
        for cap in re.captures_iter(content) {
            let method = cap[2].to_uppercase();
            let path = cap[3].to_string();
            let handler = cap[4].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }

    // 10. Rails: (get|post|put|patch|delete) "/path", to: "controller#action" — only in files containing "routes"
    if file_path.contains("routes") {
        // Try to capture "to: 'controller#action'" first
        if let Ok(re) = Regex::new(
            r#"(?i)(get|post|put|patch|delete)\s+["'](/[^"']*?)["'](?:[^,\n]*,\s*to:\s*["']([^"']+)["'])?"#,
        ) {
            for cap in re.captures_iter(content) {
                let method = cap[1].to_uppercase();
                let path = cap[2].to_string();
                let handler = cap
                    .get(3)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "<anonymous>".to_string());
                let line = line_of(cap.get(0).unwrap().start());
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler,
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 11. ASP.NET: [(HttpGet|HttpPost|HttpPut|HttpDelete|HttpPatch|Route)("/path")] followed by method
    if let Ok(re) = Regex::new(
        r#"(?s)\[(Http(Get|Post|Put|Delete|Patch)|Route)\s*\(\s*["'](/[^"']*?)["']\s*\)\s*\]\s*(?:public\s+)?(?:async\s+)?(?:\w+(?:<[^>]+>)?\s+)+([a-zA-Z_][a-zA-Z0-9_]*)\s*\("#,
    ) {
        for cap in re.captures_iter(content) {
            let verb_raw = if cap.get(2).is_some() {
                cap[2].to_lowercase()
            } else {
                "get".to_string()
            };
            let method = verb_raw.to_uppercase();
            let path = cap[3].to_string();
            let handler = cap[4].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            routes.push(RouteEndpoint {
                method,
                path,
                handler,
                file: file_path.to_string(),
                line,
            });
        }
    }
    // ASP.NET fallback
    if let Ok(re) = Regex::new(r#"\[(Http(Get|Post|Put|Delete|Patch)|Route)\s*\(\s*["'](/[^"']*)"#)
    {
        for cap in re.captures_iter(content) {
            let verb_raw = if cap.get(2).is_some() {
                cap[2].to_lowercase()
            } else {
                "get".to_string()
            };
            let method = verb_raw.to_uppercase();
            let path = cap[3].to_string();
            let line = line_of(cap.get(0).unwrap().start());
            if !routes
                .iter()
                .any(|r| r.line == line && r.method == method && r.path == path)
            {
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler: "<anonymous>".to_string(),
                    file: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // 12. Phoenix: (get|post|put|patch|delete) "/path", Controller, :action — only in files containing "router"
    if file_path.contains("router") {
        // Try to capture Controller, :action
        if let Ok(re) = Regex::new(
            r#"(?i)(get|post|put|patch|delete)\s+["'](/[^"']*?)["']\s*,\s*([A-Z][a-zA-Z0-9.]*)\s*,\s*:([a-zA-Z_][a-zA-Z0-9_]*)"#,
        ) {
            for cap in re.captures_iter(content) {
                let method = cap[1].to_uppercase();
                let path = cap[2].to_string();
                let handler = format!("{}/{}", &cap[3], &cap[4]);
                let line = line_of(cap.get(0).unwrap().start());
                routes.push(RouteEndpoint {
                    method,
                    path,
                    handler,
                    file: file_path.to_string(),
                    line,
                });
            }
        }
        // Phoenix fallback: no Controller, :action captured
        if let Ok(re) = Regex::new(r#"(?i)(get|post|put|patch|delete)\s+["'](/[^"']*)"#) {
            for cap in re.captures_iter(content) {
                let method = cap[1].to_uppercase();
                let path = cap[2].to_string();
                let line = line_of(cap.get(0).unwrap().start());
                if !routes
                    .iter()
                    .any(|r| r.line == line && r.method == method && r.path == path)
                {
                    routes.push(RouteEndpoint {
                        method,
                        path,
                        handler: "<anonymous>".to_string(),
                        file: file_path.to_string(),
                        line,
                    });
                }
            }
        }
    }

    // Deduplicate routes: multiple framework patterns may match the same route.
    // Dedup by (line, method, path) and sort by line number for stable, predictable output.
    let mut seen: std::collections::HashSet<(usize, String, String)> =
        std::collections::HashSet::new();
    routes.retain(|r| seen.insert((r.line, r.method.clone(), r.path.clone())));
    routes.sort_by_key(|r| r.line);

    routes
}

/// Extract gRPC services from proto file parse results.
fn extract_grpc_services(index: &CodebaseIndex, focus: Option<&str>) -> Vec<GrpcService> {
    let mut services: std::collections::HashMap<String, GrpcService> =
        std::collections::HashMap::new();

    for file in &index.files {
        if let Some(prefix) = focus {
            if !file.relative_path.starts_with(prefix) {
                continue;
            }
        }
        let is_proto = file
            .language
            .as_deref()
            .map(|l| l == "protobuf")
            .unwrap_or(false)
            || file.relative_path.ends_with(".proto");

        if !is_proto {
            continue;
        }

        if let Some(pr) = &file.parse_result {
            let mut current_service: Option<String> = None;

            for symbol in &pr.symbols {
                let kind_str = symbol_kind_str(&symbol.kind);
                if kind_str == "service" {
                    let entry = services.entry(symbol.name.clone()).or_insert(GrpcService {
                        name: symbol.name.clone(),
                        file: file.relative_path.clone(),
                        methods: vec![],
                    });
                    current_service = Some(symbol.name.clone());
                    // Suppress unused assignment warning for entry when no methods added yet.
                    let _ = entry;
                } else if kind_str == "method" {
                    if let Some(ref svc_name) = current_service {
                        if let Some(svc) = services.get_mut(svc_name) {
                            svc.methods.push(symbol.name.clone());
                        }
                    }
                }
            }
        }
    }

    services.into_values().collect()
}

/// Extract GraphQL types from GraphQL file parse results.
fn extract_graphql_types(index: &CodebaseIndex, focus: Option<&str>) -> Vec<GraphqlType> {
    let mut types = vec![];

    for file in &index.files {
        if let Some(prefix) = focus {
            if !file.relative_path.starts_with(prefix) {
                continue;
            }
        }
        let is_graphql = file
            .language
            .as_deref()
            .map(|l| l == "graphql")
            .unwrap_or(false)
            || file.relative_path.ends_with(".graphql")
            || file.relative_path.ends_with(".gql");

        if !is_graphql {
            continue;
        }

        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                let kind_str = symbol_kind_str(&symbol.kind);
                if matches!(kind_str, "type" | "query" | "mutation") {
                    types.push(GraphqlType {
                        name: symbol.name.clone(),
                        kind: kind_str.to_string(),
                        file: file.relative_path.clone(),
                    });
                }
            }
        }
    }

    types
}

/// Orchestrator: combine public symbols + routes + gRPC + GraphQL within a token budget.
pub fn extract_api_surface(
    index: &CodebaseIndex,
    focus: Option<&str>,
    include: &str,
    token_budget: usize,
) -> ApiSurface {
    let (symbols_section, sym_tokens) = if include == "all" || include == "symbols" {
        extract_public_symbols(index, focus)
    } else {
        (
            SymbolSection {
                total: 0,
                by_file: vec![],
            },
            0,
        )
    };

    let mut route_endpoints: Vec<RouteEndpoint> = vec![];
    if include == "all" || include == "routes" {
        for file in &index.files {
            if let Some(prefix) = focus {
                if !file.relative_path.starts_with(prefix) {
                    continue;
                }
            }
            let found = detect_routes(&file.content, &file.relative_path);
            route_endpoints.extend(found);
        }
    }

    let grpc_services = if include == "all" {
        extract_grpc_services(index, focus)
    } else {
        vec![]
    };

    let graphql_types = if include == "all" {
        extract_graphql_types(index, focus)
    } else {
        vec![]
    };

    let route_tokens = route_endpoints.len() * 8;
    let grpc_tokens = grpc_services
        .iter()
        .map(|s| s.methods.len() * 4 + 4)
        .sum::<usize>();
    let graphql_tokens = graphql_types.len() * 4;
    let raw_total = sym_tokens + route_tokens + grpc_tokens + graphql_tokens;

    // Apply token budget: trim symbol section if over budget.
    let (final_symbols, final_token_count) = if raw_total > token_budget && token_budget > 0 {
        let available_for_symbols =
            token_budget.saturating_sub(route_tokens + grpc_tokens + graphql_tokens);
        let mut trimmed_by_file = vec![];
        let mut remaining = available_for_symbols;
        let mut total_kept = 0usize;
        for file_syms in symbols_section.by_file {
            let file_tokens: usize = file_syms
                .symbols
                .iter()
                .map(|s| s.signature.split_whitespace().count() + 2)
                .sum();
            if file_tokens <= remaining {
                remaining = remaining.saturating_sub(file_tokens);
                total_kept += file_syms.symbols.len();
                trimmed_by_file.push(file_syms);
            }
        }
        (
            SymbolSection {
                total: total_kept,
                by_file: trimmed_by_file,
            },
            available_for_symbols - remaining + route_tokens + grpc_tokens + graphql_tokens,
        )
    } else {
        (symbols_section, raw_total)
    };

    let route_total = route_endpoints.len();
    ApiSurface {
        symbols: final_symbols,
        routes: RouteSection {
            total: route_total,
            endpoints: route_endpoints,
        },
        grpc_services,
        graphql_types,
        token_count: final_token_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::scanner::ScannedFile;
    use std::collections::HashMap;

    fn make_index_with_symbols() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let fp1 = dir.path().join("src/api.rs");
        std::fs::create_dir_all(fp1.parent().unwrap()).unwrap();
        std::fs::write(&fp1, "pub fn get_users() {} pub fn internal() {}").unwrap();

        let fp2 = dir.path().join("src/internal.rs");
        std::fs::write(&fp2, "fn private_helper() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/api.rs".into(),
                absolute_path: fp1,
                language: Some("rust".into()),
                size_bytes: 40,
            },
            ScannedFile {
                relative_path: "src/internal.rs".into(),
                absolute_path: fp2,
                language: Some("rust".into()),
                size_bytes: 20,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/api.rs".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "get_users".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: "pub fn get_users()".to_string(),
                        body: "{}".to_string(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "internal".to_string(),
                        kind: SymbolKind::Function,
                        visibility: Visibility::Private,
                        signature: "fn internal()".to_string(),
                        body: "{}".to_string(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/internal.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "private_helper".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn private_helper()".to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        CodebaseIndex::build(files, parse_results, &counter)
    }

    #[test]
    fn test_extract_public_symbols_only() {
        let index = make_index_with_symbols();
        let (section, _tokens) = extract_public_symbols(&index, None);
        // Only one public symbol: get_users
        assert_eq!(section.total, 1);
        assert_eq!(section.by_file.len(), 1);
        assert_eq!(section.by_file[0].path, "src/api.rs");
        assert_eq!(section.by_file[0].symbols[0].name, "get_users");
    }

    #[test]
    fn test_private_symbols_excluded() {
        let index = make_index_with_symbols();
        let (section, _tokens) = extract_public_symbols(&index, None);
        // private_helper and internal must not appear
        for file_syms in &section.by_file {
            for sym in &file_syms.symbols {
                assert_ne!(sym.name, "private_helper");
                assert_ne!(sym.name, "internal");
            }
        }
    }

    #[test]
    fn test_sorted_by_pagerank() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let fp_a = dir.path().join("a.rs");
        let fp_b = dir.path().join("b.rs");
        std::fs::write(&fp_a, "pub fn alpha() {}").unwrap();
        std::fs::write(&fp_b, "pub fn beta() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "a.rs".into(),
                absolute_path: fp_a,
                language: Some("rust".into()),
                size_bytes: 18,
            },
            ScannedFile {
                relative_path: "b.rs".into(),
                absolute_path: fp_b,
                language: Some("rust".into()),
                size_bytes: 16,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "a.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "alpha".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn alpha()".to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "b.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "beta".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn beta()".to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let (section, _tokens) = extract_public_symbols(&index, None);

        // Verify sorted order: higher pagerank first (or at least stable order is maintained).
        let pageranks: Vec<f64> = section.by_file.iter().map(|f| f.pagerank).collect();
        for i in 1..pageranks.len() {
            assert!(
                pageranks[i - 1] >= pageranks[i],
                "files should be sorted by pagerank descending"
            );
        }
    }

    #[test]
    fn test_focus_filter() {
        let index = make_index_with_symbols();
        // With focus "src/api", only api.rs should be included.
        let (section, _tokens) = extract_public_symbols(&index, Some("src/api"));
        assert_eq!(section.by_file.len(), 1);
        assert!(section.by_file[0].path.starts_with("src/api"));

        // With a non-matching focus, no files.
        let (section_empty, _) = extract_public_symbols(&index, Some("lib/"));
        assert_eq!(section_empty.total, 0);
        assert!(section_empty.by_file.is_empty());
    }

    // --- Route detection tests ---

    #[test]
    fn test_detect_routes_express() {
        let content = r#"app.get('/users', listUsers); router.post("/items", createItem);"#;
        let routes = detect_routes(content, "routes/index.js");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "listUsers");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/items");
        assert_eq!(routes[1].handler, "createItem");
    }

    #[test]
    fn test_detect_routes_flask() {
        let content = "@app.route('/home')\ndef home_view():\n    pass\n@blueprint.get('/api/data')\ndef get_data():\n    pass";
        let routes = detect_routes(content, "app.py");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/home");
        assert_eq!(routes[0].handler, "home_view");
        assert_eq!(routes[1].method, "GET");
        assert_eq!(routes[1].path, "/api/data");
        assert_eq!(routes[1].handler, "get_data");
    }

    #[test]
    fn test_detect_routes_django() {
        let content = r#"path('users/', views.user_list)"#;
        // Only matches when file_path contains "urls"
        let routes_no_match = detect_routes(content, "app/views.py");
        assert_eq!(routes_no_match.len(), 0);

        let routes = detect_routes(content, "app/urls.py");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].path, "/users/");
        assert_eq!(routes[0].handler, "views.user_list");
    }

    #[test]
    fn test_detect_routes_fastapi() {
        let content = "@app.get(\"/users\")\nasync def list_users():\n    pass\n@router.post(\"/items\")\ndef create_item():\n    pass";
        let routes = detect_routes(content, "main.py");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "list_users");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/items");
        assert_eq!(routes[1].handler, "create_item");
    }

    #[test]
    fn test_detect_routes_spring() {
        let content = "@GetMapping(\"/users\")\npublic List<User> getUsers() {}\n@PostMapping(\"/users\")\npublic User createUser() {}\n@RequestMapping(\"/api\")\npublic String apiRoot() {}";
        let routes = detect_routes(content, "UserController.java");
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "getUsers");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/users");
        assert_eq!(routes[1].handler, "createUser");
        assert_eq!(routes[2].method, "GET");
        assert_eq!(routes[2].path, "/api");
        assert_eq!(routes[2].handler, "apiRoot");
    }

    #[test]
    fn test_detect_routes_actix() {
        let content = "#[get(\"/health\")]\nasync fn health_check() -> impl Responder { HttpResponse::Ok() }\n#[post(\"/login\")]\nasync fn do_login() -> impl Responder { HttpResponse::Ok() }";
        let routes = detect_routes(content, "src/main.rs");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/health");
        assert_eq!(routes[0].handler, "health_check");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/login");
        assert_eq!(routes[1].handler, "do_login");
    }

    #[test]
    fn test_detect_routes_axum() {
        let content = r#"Router::new().route("/users", get(list_users)).route("/users/:id", post(create_user))"#;
        let routes = detect_routes(content, "src/server.rs");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "list_users");
        assert_eq!(routes[1].path, "/users/:id");
        assert_eq!(routes[1].handler, "create_user");
    }

    #[test]
    fn test_detect_routes_gin() {
        let content = r#"r.GET("/ping", pingHandler)
router.POST("/users", createUser)"#;
        let routes = detect_routes(content, "main.go");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/ping");
        assert_eq!(routes[0].handler, "pingHandler");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/users");
        assert_eq!(routes[1].handler, "createUser");
    }

    #[test]
    fn test_detect_routes_echo() {
        let content = r#"e.GET("/users", getUsers)
g.POST("/items", createItem)
echo.DELETE("/users/:id", deleteUser)"#;
        let routes = detect_routes(content, "server.go");
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "getUsers");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/items");
        assert_eq!(routes[1].handler, "createItem");
        assert_eq!(routes[2].method, "DELETE");
        assert_eq!(routes[2].path, "/users/:id");
        assert_eq!(routes[2].handler, "deleteUser");
    }

    #[test]
    fn test_detect_routes_rails() {
        let content = r#"get '/users', to: 'users#index'
post "/items", to: 'items#create'"#;
        // Only matches when file_path contains "routes"
        let routes_no = detect_routes(content, "app/controllers/users_controller.rb");
        assert_eq!(routes_no.len(), 0);

        let routes = detect_routes(content, "config/routes.rb");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "users#index");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/items");
        assert_eq!(routes[1].handler, "items#create");
    }

    #[test]
    fn test_detect_routes_aspnet() {
        let content = "[HttpGet(\"/api/users\")]\npublic IActionResult GetUsers() {}\n[HttpPost(\"/api/items\")]\npublic IActionResult CreateItem() {}\n[Route(\"/api/health\")]\npublic IActionResult HealthCheck() {}";
        let routes = detect_routes(content, "Controllers/UsersController.cs");
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/api/users");
        assert_eq!(routes[0].handler, "GetUsers");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/api/items");
        assert_eq!(routes[1].handler, "CreateItem");
    }

    #[test]
    fn test_detect_routes_phoenix() {
        let content = r#"get "/users", UserController, :index
post "/items", ItemController, :create"#;
        // Only matches when file_path contains "router"
        let routes_no = detect_routes(content, "lib/my_app/controllers/user_controller.ex");
        assert_eq!(routes_no.len(), 0);

        let routes = detect_routes(content, "lib/my_app_web/router.ex");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "UserController/index");
        assert_eq!(routes[1].method, "POST");
        assert_eq!(routes[1].path, "/items");
        assert_eq!(routes[1].handler, "ItemController/create");
    }

    #[test]
    fn test_detect_routes_express_anonymous_handler() {
        let content = r#"app.get('/x', function(req, res) { res.send('ok'); });"#;
        let routes = detect_routes(content, "app.js");
        assert!(!routes.is_empty());
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_no_match() {
        let content = r#"// This is a comment
fn regular_function() {}
let x = 42;
"#;
        let routes = detect_routes(content, "src/util.rs");
        assert_eq!(
            routes.len(),
            0,
            "non-route strings should produce no results"
        );
    }
}
