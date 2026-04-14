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

/// Infer language tag from file extension. Returns `""` for unknown extensions.
fn language_from_path(path: &str) -> &'static str {
    let ext = match path.rsplit_once('.') {
        Some((_, e)) => e.to_lowercase(),
        None => return "",
    };
    match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "ex" | "exs" => "elixir",
        "cs" => "csharp",
        "scala" => "scala",
        "php" => "php",
        _ => "",
    }
}

/// Returns true when the file content contains an import/use of a web framework
/// that matches the inferred language of the file.
///
/// This guard prevents route-detection regexes from firing on test fixtures,
/// docstrings, or any source file that merely mentions route-like strings
/// without actually being a web-framework handler file.
fn has_framework_import(content: &str, language: Option<&str>) -> bool {
    let lang = language.unwrap_or("");
    match lang {
        "rust" => {
            content.contains("use axum")
                || content.contains("use actix_web")
                || content.contains("use rocket")
                || content.contains("use warp")
                || content.contains("use tide")
                || content.contains("use poem")
                || content.contains("use salvo")
        }
        "python" => {
            content.contains("from flask")
                || content.contains("import flask")
                || content.contains("from fastapi")
                || content.contains("import fastapi")
                || content.contains("from django")
                || content.contains("from aiohttp")
                || content.contains("from starlette")
                || content.contains("from sanic")
                || content.contains("import bottle")
                || content.contains("from bottle")
        }
        "javascript" | "typescript" => {
            content.contains("require('express')")
                || content.contains("require(\"express\")")
                || content.contains("from 'express'")
                || content.contains("from \"express\"")
                || content.contains("from 'koa'")
                || content.contains("from \"koa\"")
                || content.contains("from 'fastify'")
                || content.contains("from \"fastify\"")
                || content.contains("from '@nestjs/")
                || content.contains("from \"@nestjs/")
                || content.contains("from 'hono'")
                || content.contains("from \"hono\"")
                || content.contains("Router()")
                || content.contains("new Koa()")
                || content.contains("fastify(")
        }
        "go" => {
            content.contains("\"github.com/gin-gonic/gin\"")
                || content.contains("\"github.com/labstack/echo")
                || content.contains("\"github.com/gofiber/fiber")
                || content.contains("\"net/http\"")
        }
        "ruby" => {
            content.contains("Rails.application.routes")
                || content.contains("require 'sinatra'")
                || content.contains("require \"sinatra\"")
                || content.contains("ActionController")
        }
        "java" | "kotlin" => {
            content.contains("@RestController")
                || content.contains("@Controller")
                || content.contains("@RequestMapping")
                || content.contains("@GetMapping")
                || content.contains("@PostMapping")
                || content.contains("@PutMapping")
                || content.contains("@DeleteMapping")
                || content.contains("@PatchMapping")
                || content.contains("import org.springframework")
        }
        "elixir" => content.contains("use Phoenix.Router") || content.contains("use Plug.Router"),
        "csharp" => {
            content.contains("using Microsoft.AspNetCore")
                || content.contains("using Microsoft.AspNet")
                || content.contains("[ApiController]")
                || content.contains("[Controller]")
                || content.contains("[Route(")
                || content.contains("[HttpGet(")
                || content.contains("[HttpPost(")
                || content.contains("[HttpPut(")
                || content.contains("[HttpDelete(")
                || content.contains("[HttpPatch(")
        }
        "scala" => {
            content.contains("import play.api")
                || content.contains("import akka.http")
                || content.contains("import org.http4s")
        }
        "php" => {
            content.contains("use Illuminate\\")
                || content.contains("use Symfony\\")
                || content.contains("Route::")
                || content.contains("$router->")
        }
        _ => false,
    }
}

/// Returns true when the file extension indicates source code that may contain route
/// registrations. Non-source files (markdown, YAML, JSON, plain text, etc.) return false
/// to avoid false-positive route matches from documentation code examples.
fn is_source_code_file(path: &str) -> bool {
    let ext = match path.rsplit_once('.') {
        Some((_, e)) => e.to_lowercase(),
        None => return false,
    };
    matches!(
        ext.as_str(),
        "rs" | "py"
            | "pyi"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "rb"
            | "ex"
            | "exs"
            | "elm"
            | "scala"
            | "cs"
            | "swift"
            | "php"
            | "c"
            | "cpp"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "hxx"
    )
}

/// Detect HTTP route endpoints from file content using 12 framework patterns.
pub fn detect_routes(content: &str, file_path: &str) -> Vec<RouteEndpoint> {
    // Gate 1: skip non-source files (markdown, YAML, JSON, HTML, plain text, etc.)
    // to avoid false positives from documentation code examples.
    if !is_source_code_file(file_path) {
        return vec![];
    }

    // Gate 2: skip source files that don't import a web framework.
    // This prevents test fixtures, docstrings, and files that merely contain
    // route-looking strings (e.g., api_surface.rs itself) from generating
    // spurious route matches. Files without a framework import cannot contain
    // real HTTP handlers.
    //
    // Exemption: Rails and Phoenix routes files are identified by file-path
    // convention ("routes" / "router" in the path) rather than import — those
    // framework-specific gates still apply inside the route-detection logic.
    let lang = language_from_path(file_path);
    // Some frameworks are identified purely by file-path convention rather than imports:
    // - Django: `urls.py` files contain route registrations (no explicit import check needed)
    // - Rails: `config/routes.rb` and similar (file path contains "routes")
    // - Phoenix: `router.ex` and similar (file path contains "router")
    let is_path_gated =
        file_path.contains("routes") || file_path.contains("router") || file_path.contains("urls");
    if !is_path_gated && !has_framework_import(content, Some(lang)) {
        return vec![];
    }

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
pub(crate) fn extract_grpc_services(
    index: &CodebaseIndex,
    focus: Option<&str>,
) -> Vec<GrpcService> {
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
pub(crate) fn extract_graphql_types(
    index: &CodebaseIndex,
    focus: Option<&str>,
) -> Vec<GraphqlType> {
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
        let content = "const express = require('express');\napp.get('/users', listUsers); router.post(\"/items\", createItem);";
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
        let content = "from flask import Flask, Blueprint\n@app.route('/home')\ndef home_view():\n    pass\n@blueprint.get('/api/data')\ndef get_data():\n    pass";
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
        let content = "from fastapi import FastAPI\n@app.get(\"/users\")\nasync def list_users():\n    pass\n@router.post(\"/items\")\ndef create_item():\n    pass";
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
        let content = "import org.springframework.web.bind.annotation.*;\n@GetMapping(\"/users\")\npublic List<User> getUsers() {}\n@PostMapping(\"/users\")\npublic User createUser() {}\n@RequestMapping(\"/api\")\npublic String apiRoot() {}";
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
        let content = "use actix_web::{get, post, HttpResponse, Responder};\n#[get(\"/health\")]\nasync fn health_check() -> impl Responder { HttpResponse::Ok() }\n#[post(\"/login\")]\nasync fn do_login() -> impl Responder { HttpResponse::Ok() }";
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
        let content = "use axum::Router;\nuse axum::routing::{get, post};\nRouter::new().route(\"/users\", get(list_users)).route(\"/users/:id\", post(create_user))";
        let routes = detect_routes(content, "src/server.rs");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "list_users");
        assert_eq!(routes[1].path, "/users/:id");
        assert_eq!(routes[1].handler, "create_user");
    }

    #[test]
    fn test_detect_routes_gin() {
        let content = "import \"github.com/gin-gonic/gin\"\nr.GET(\"/ping\", pingHandler)\nrouter.POST(\"/users\", createUser)";
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
        let content = "import \"github.com/labstack/echo/v4\"\ne.GET(\"/users\", getUsers)\ng.POST(\"/items\", createItem)\necho.DELETE(\"/users/:id\", deleteUser)";
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
        let content = "const express = require('express');\napp.get('/x', function(req, res) { res.send('ok'); });";
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

    // --- gRPC extraction tests ---

    fn make_index_with_proto() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let proto_content = r#"syntax = "proto3";
service UserService {
  rpc GetUser (GetUserRequest) returns (GetUserResponse);
  rpc ListUsers (ListUsersRequest) returns (ListUsersResponse);
}
service OrderService {
  rpc CreateOrder (CreateOrderRequest) returns (CreateOrderResponse);
}"#;

        let fp = dir.path().join("api/user.proto");
        std::fs::create_dir_all(fp.parent().unwrap()).unwrap();
        std::fs::write(&fp, proto_content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "api/user.proto".into(),
            absolute_path: fp,
            language: Some("protobuf".into()),
            size_bytes: proto_content.len() as u64,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "api/user.proto".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "UserService".to_string(),
                        kind: SymbolKind::Service,
                        visibility: Visibility::Public,
                        signature: "service UserService".to_string(),
                        body: String::new(),
                        start_line: 2,
                        end_line: 5,
                    },
                    Symbol {
                        name: "GetUser".to_string(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc GetUser".to_string(),
                        body: String::new(),
                        start_line: 3,
                        end_line: 3,
                    },
                    Symbol {
                        name: "ListUsers".to_string(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc ListUsers".to_string(),
                        body: String::new(),
                        start_line: 4,
                        end_line: 4,
                    },
                    Symbol {
                        name: "OrderService".to_string(),
                        kind: SymbolKind::Service,
                        visibility: Visibility::Public,
                        signature: "service OrderService".to_string(),
                        body: String::new(),
                        start_line: 6,
                        end_line: 8,
                    },
                    Symbol {
                        name: "CreateOrder".to_string(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc CreateOrder".to_string(),
                        body: String::new(),
                        start_line: 7,
                        end_line: 7,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        CodebaseIndex::build(files, parse_results, &counter)
    }

    #[test]
    fn test_extract_grpc_services() {
        let index = make_index_with_proto();
        let services = extract_grpc_services(&index, None);
        assert_eq!(services.len(), 2);

        let user_svc = services.iter().find(|s| s.name == "UserService");
        assert!(
            user_svc.is_some(),
            "UserService should be extracted from proto"
        );
        let user_svc = user_svc.unwrap();
        assert_eq!(user_svc.methods.len(), 2);
        assert!(user_svc.methods.contains(&"GetUser".to_string()));
        assert!(user_svc.methods.contains(&"ListUsers".to_string()));
        assert_eq!(user_svc.file, "api/user.proto");

        let order_svc = services.iter().find(|s| s.name == "OrderService");
        assert!(
            order_svc.is_some(),
            "OrderService should be extracted from proto"
        );
        assert_eq!(order_svc.unwrap().methods.len(), 1);
        assert!(order_svc
            .unwrap()
            .methods
            .contains(&"CreateOrder".to_string()));
    }

    #[test]
    fn test_extract_grpc_services_with_focus_filter() {
        let index = make_index_with_proto();
        // Focus on a non-matching prefix => no services.
        let services = extract_grpc_services(&index, Some("src/"));
        assert_eq!(
            services.len(),
            0,
            "focus filter should exclude proto files not under src/"
        );

        // Focus matching prefix => includes services.
        let services = extract_grpc_services(&index, Some("api/"));
        assert_eq!(services.len(), 2);
    }

    // --- GraphQL extraction tests ---

    fn make_index_with_graphql() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let gql_content = r#"type Query {
  users: [User]
  user(id: ID!): User
}

type Mutation {
  createUser(input: CreateUserInput!): User
}

type User {
  id: ID!
  name: String!
  email: String!
}"#;

        let fp = dir.path().join("schema.graphql");
        std::fs::write(&fp, gql_content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "schema.graphql".into(),
            absolute_path: fp,
            language: Some("graphql".into()),
            size_bytes: gql_content.len() as u64,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "schema.graphql".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "Query".to_string(),
                        kind: SymbolKind::Query,
                        visibility: Visibility::Public,
                        signature: "type Query".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 4,
                    },
                    Symbol {
                        name: "Mutation".to_string(),
                        kind: SymbolKind::Mutation,
                        visibility: Visibility::Public,
                        signature: "type Mutation".to_string(),
                        body: String::new(),
                        start_line: 6,
                        end_line: 8,
                    },
                    Symbol {
                        name: "User".to_string(),
                        kind: SymbolKind::Type,
                        visibility: Visibility::Public,
                        signature: "type User".to_string(),
                        body: String::new(),
                        start_line: 10,
                        end_line: 14,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        CodebaseIndex::build(files, parse_results, &counter)
    }

    #[test]
    fn test_extract_graphql_types() {
        let index = make_index_with_graphql();
        let types = extract_graphql_types(&index, None);
        assert_eq!(types.len(), 3);

        let query = types.iter().find(|t| t.name == "Query");
        assert!(query.is_some(), "Query type should be extracted");
        assert_eq!(query.unwrap().kind, "query");
        assert_eq!(query.unwrap().file, "schema.graphql");

        let mutation = types.iter().find(|t| t.name == "Mutation");
        assert!(mutation.is_some(), "Mutation type should be extracted");
        assert_eq!(mutation.unwrap().kind, "mutation");

        let user = types.iter().find(|t| t.name == "User");
        assert!(user.is_some(), "User type should be extracted");
        assert_eq!(user.unwrap().kind, "type");
    }

    #[test]
    fn test_extract_graphql_types_with_focus_filter() {
        let index = make_index_with_graphql();
        // Non-matching focus => no types.
        let types = extract_graphql_types(&index, Some("api/"));
        assert_eq!(
            types.len(),
            0,
            "focus filter should exclude graphql files not under api/"
        );

        // Matching focus => includes types.
        let types = extract_graphql_types(&index, Some("schema"));
        assert_eq!(types.len(), 3);
    }

    #[test]
    fn test_extract_graphql_gql_extension() {
        // Verify .gql extension is also recognized.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let gql_content = "type Subscription { onMessage: Message }";
        let fp = dir.path().join("events.gql");
        std::fs::write(&fp, gql_content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "events.gql".into(),
            absolute_path: fp,
            language: None, // language not set, but extension is .gql
            size_bytes: gql_content.len() as u64,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "events.gql".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "Subscription".to_string(),
                    kind: SymbolKind::Type,
                    visibility: Visibility::Public,
                    signature: "type Subscription".to_string(),
                    body: String::new(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let types = extract_graphql_types(&index, None);
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].name, "Subscription");
        assert_eq!(types[0].file, "events.gql");
    }

    // --- extract_api_surface orchestrator tests ---

    #[test]
    fn test_extract_api_surface_all_includes_grpc_and_graphql() {
        // Build an index with proto + graphql + rust files.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let proto_content = "syntax = \"proto3\"; service Greeter { rpc SayHello(); }";
        let gql_content = "type Query { hello: String }";
        let rs_content = "pub fn hello() {}";

        let fp_proto = dir.path().join("api.proto");
        let fp_gql = dir.path().join("schema.graphql");
        let fp_rs = dir.path().join("src/lib.rs");
        std::fs::create_dir_all(fp_rs.parent().unwrap()).unwrap();
        std::fs::write(&fp_proto, proto_content).unwrap();
        std::fs::write(&fp_gql, gql_content).unwrap();
        std::fs::write(&fp_rs, rs_content).unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "api.proto".into(),
                absolute_path: fp_proto,
                language: Some("protobuf".into()),
                size_bytes: proto_content.len() as u64,
            },
            ScannedFile {
                relative_path: "schema.graphql".into(),
                absolute_path: fp_gql,
                language: Some("graphql".into()),
                size_bytes: gql_content.len() as u64,
            },
            ScannedFile {
                relative_path: "src/lib.rs".into(),
                absolute_path: fp_rs,
                language: Some("rust".into()),
                size_bytes: rs_content.len() as u64,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "api.proto".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "Greeter".to_string(),
                        kind: SymbolKind::Service,
                        visibility: Visibility::Public,
                        signature: "service Greeter".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "SayHello".to_string(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc SayHello".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "schema.graphql".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "Query".to_string(),
                    kind: SymbolKind::Query,
                    visibility: Visibility::Public,
                    signature: "type Query".to_string(),
                    body: String::new(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "hello".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn hello()".to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);

        let surface = extract_api_surface(&index, None, "all", 100_000);
        assert!(
            surface.symbols.total >= 1,
            "should include public symbols from src/lib.rs"
        );
        assert_eq!(surface.grpc_services.len(), 1);
        assert_eq!(surface.grpc_services[0].name, "Greeter");
        assert_eq!(surface.grpc_services[0].methods.len(), 1);
        assert_eq!(surface.grpc_services[0].methods[0], "SayHello");
        assert_eq!(surface.graphql_types.len(), 1);
        assert_eq!(surface.graphql_types[0].name, "Query");
        assert_eq!(surface.graphql_types[0].kind, "query");
    }

    #[test]
    fn test_extract_api_surface_include_symbols_only() {
        let index = make_index_with_symbols();
        let surface = extract_api_surface(&index, None, "symbols", 100_000);
        assert!(surface.symbols.total >= 1, "symbols should be included");
        assert!(
            surface.routes.endpoints.is_empty(),
            "routes should be empty when include=symbols"
        );
        assert!(
            surface.grpc_services.is_empty(),
            "grpc should be empty when include=symbols"
        );
        assert!(
            surface.graphql_types.is_empty(),
            "graphql should be empty when include=symbols"
        );
    }

    #[test]
    fn test_extract_api_surface_include_routes_only() {
        let index = make_index_with_symbols();
        let surface = extract_api_surface(&index, None, "routes", 100_000);
        assert_eq!(
            surface.symbols.total, 0,
            "symbols should be empty when include=routes"
        );
        assert!(
            surface.grpc_services.is_empty(),
            "grpc should be empty when include=routes"
        );
        assert!(
            surface.graphql_types.is_empty(),
            "graphql should be empty when include=routes"
        );
    }

    #[test]
    fn test_extract_api_surface_token_budget_trims_symbols() {
        // Use a large index with many public symbols so the budget clips them.
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let content = "pub fn a() {} pub fn b() {} pub fn c() {} pub fn d() {}";
        let fp = dir.path().join("src/big.rs");
        std::fs::create_dir_all(fp.parent().unwrap()).unwrap();
        std::fs::write(&fp, content).unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/big.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: content.len() as u64,
        }];

        let mut parse_results = HashMap::new();
        let symbols: Vec<Symbol> = ["a", "b", "c", "d"]
            .iter()
            .enumerate()
            .map(|(i, name)| Symbol {
                name: name.to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: format!("pub fn {}()", name),
                body: "{}".to_string(),
                start_line: i + 1,
                end_line: i + 1,
            })
            .collect();

        parse_results.insert(
            "src/big.rs".to_string(),
            ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);

        // With a very generous budget, all symbols fit.
        let surface_full = extract_api_surface(&index, None, "all", 100_000);
        assert_eq!(surface_full.symbols.total, 4);

        // With a tiny budget (e.g., 1 token), symbols should be trimmed.
        let surface_tiny = extract_api_surface(&index, None, "all", 1);
        assert!(
            surface_tiny.symbols.total < 4,
            "tiny budget should trim symbols; got {}",
            surface_tiny.symbols.total
        );
    }

    #[test]
    fn test_extract_api_surface_with_focus() {
        let index = make_index_with_symbols();
        // Focus on "src/api" => only api.rs symbols.
        let surface = extract_api_surface(&index, Some("src/api"), "all", 100_000);
        assert_eq!(surface.symbols.total, 1);
        assert_eq!(surface.symbols.by_file[0].path, "src/api.rs");

        // Focus on non-matching prefix => empty.
        let surface_empty = extract_api_surface(&index, Some("lib/"), "all", 100_000);
        assert_eq!(surface_empty.symbols.total, 0);
        assert!(surface_empty.symbols.by_file.is_empty());
    }

    #[test]
    fn test_extract_api_surface_empty_index() {
        let index = CodebaseIndex::empty();
        let surface = extract_api_surface(&index, None, "all", 100_000);
        assert_eq!(surface.symbols.total, 0);
        assert!(surface.routes.endpoints.is_empty());
        assert!(surface.grpc_services.is_empty());
        assert!(surface.graphql_types.is_empty());
        assert_eq!(surface.token_count, 0);
    }

    // --- symbol_kind_str coverage via extract_public_symbols ---
    //
    // `symbol_kind_str` is a private helper with one match arm per SymbolKind
    // variant.  Cover every arm by building an index with one public symbol
    // of each kind and verifying the rendered `kind` string in the output.
    #[test]
    fn test_symbol_kind_str_all_variants_via_public_symbols() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("src/all_kinds.rs");
        std::fs::create_dir_all(fp.parent().unwrap()).unwrap();
        std::fs::write(&fp, "// placeholder content").unwrap();

        // One public symbol for each SymbolKind variant.
        let kinds: Vec<(SymbolKind, &'static str, &'static str)> = vec![
            (SymbolKind::Function, "s_fn", "function"),
            (SymbolKind::Struct, "s_struct", "struct"),
            (SymbolKind::Enum, "s_enum", "enum"),
            (SymbolKind::Trait, "s_trait", "trait"),
            (SymbolKind::Interface, "s_interface", "interface"),
            (SymbolKind::Class, "s_class", "class"),
            (SymbolKind::Method, "s_method", "method"),
            (SymbolKind::Constant, "s_const", "constant"),
            (SymbolKind::TypeAlias, "s_alias", "type_alias"),
            (SymbolKind::Selector, "s_sel", "selector"),
            (SymbolKind::Mixin, "s_mix", "mixin"),
            (SymbolKind::Variable, "s_var", "variable"),
            (SymbolKind::Heading, "s_head", "heading"),
            (SymbolKind::Section, "s_sect", "section"),
            (SymbolKind::Key, "s_key", "key"),
            (SymbolKind::Table, "s_tab", "table"),
            (SymbolKind::Block, "s_blk", "block"),
            (SymbolKind::Target, "s_tgt", "target"),
            (SymbolKind::Rule, "s_rule", "rule"),
            (SymbolKind::Element, "s_elem", "element"),
            (SymbolKind::Message, "s_msg", "message"),
            (SymbolKind::Service, "s_svc", "service"),
            (SymbolKind::Query, "s_qry", "query"),
            (SymbolKind::Mutation, "s_mut", "mutation"),
            (SymbolKind::Type, "s_type", "type"),
            (SymbolKind::Instruction, "s_instr", "instruction"),
        ];

        let symbols: Vec<Symbol> = kinds
            .iter()
            .map(|(k, name, _)| Symbol {
                name: (*name).to_string(),
                kind: k.clone(),
                visibility: Visibility::Public,
                signature: format!("sig {}", name),
                body: String::new(),
                start_line: 1,
                end_line: 1,
            })
            .collect();

        let files = vec![ScannedFile {
            relative_path: "src/all_kinds.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 20,
        }];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/all_kinds.rs".to_string(),
            ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let (section, _tokens) = extract_public_symbols(&index, None);
        assert_eq!(section.total, kinds.len());
        assert_eq!(section.by_file.len(), 1);

        // Every expected kind string must appear in the output.
        let rendered: Vec<&str> = section.by_file[0]
            .symbols
            .iter()
            .map(|s| s.kind.as_str())
            .collect();
        for (_, name, expected_kind_str) in &kinds {
            assert!(
                rendered.iter().any(|k| k == expected_kind_str),
                "missing kind `{}` for symbol `{}` in rendered output: {:?}",
                expected_kind_str,
                name,
                rendered
            );
        }
    }

    // --- Fallback route regex paths ---
    //
    // Each supported framework has a "strict" regex that matches a full
    // function signature, and a "fallback" regex that matches only the
    // decorator/annotation when no signature follows.  Cover each fallback.

    #[test]
    fn test_detect_routes_flask_fallback() {
        // Flask decorator with no `def` on the following line — triggers the
        // fallback branch which emits `<anonymous>` as the handler.
        let content = "from flask import Flask\n@app.route('/fallback')\n# no def follows";
        let routes = detect_routes(content, "app.py");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/fallback");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_fastapi_fallback() {
        // FastAPI decorator with no `def` on the following line.
        let content = "from fastapi import FastAPI, APIRouter\n@router.delete('/fallback-fast')\n# dangling decorator";
        let routes = detect_routes(content, "main.py");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "DELETE");
        assert_eq!(routes[0].path, "/fallback-fast");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_spring_fallback() {
        // Spring annotation with no method signature following it.
        let content = "import org.springframework.web.bind.annotation.PostMapping;\n@PostMapping(\"/fallback-spring\")\n// end of file";
        let routes = detect_routes(content, "Controller.java");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "POST");
        assert_eq!(routes[0].path, "/fallback-spring");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_actix_fallback() {
        // actix attribute with no `fn` on the following line.
        let content = "use actix_web::{put};\n#[put(\"/fallback-actix\")]\n// trailing comment";
        let routes = detect_routes(content, "src/lib.rs");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "PUT");
        assert_eq!(routes[0].path, "/fallback-actix");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_axum_fallback() {
        // .route("/path") without an explicit handler signature matched
        // by the strict regex (no verb wrapper).
        let content = "use axum::Router;\n.route(\"/fallback-axum\")";
        let routes = detect_routes(content, "src/server.rs");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/fallback-axum");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_rails_anonymous_handler() {
        // Rails route without an explicit `to:` clause — handler should
        // default to "<anonymous>" (covers the `unwrap_or_else` branch).
        let content = "get '/health'\n";
        let routes = detect_routes(content, "config/routes.rb");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/health");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_aspnet_fallback() {
        // ASP.NET attribute with no method signature afterwards.
        let content = "[HttpPut(\"/fallback-aspnet\")]\n// trailing";
        let routes = detect_routes(content, "Controllers/Foo.cs");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "PUT");
        assert_eq!(routes[0].path, "/fallback-aspnet");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    #[test]
    fn test_detect_routes_phoenix_fallback() {
        // Phoenix route without a Controller/action pair.  The strict regex
        // requires `Controller, :action`; without it, the fallback branch
        // emits an `<anonymous>` handler.
        let content = "get \"/ping\"\n";
        let routes = detect_routes(content, "lib/my_app_web/router.ex");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/ping");
        assert_eq!(routes[0].handler, "<anonymous>");
    }

    // --- extract_api_surface token budget: fit-then-skip branch ---
    //
    // The orchestrator's trim loop walks each file and keeps it iff its
    // per-file token cost fits in the remaining budget.  Covers the branch
    // where at least one file fits AND a subsequent file is skipped.
    #[test]
    fn test_extract_api_surface_budget_keeps_fitting_files_skips_rest() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let content_a = "pub fn a() {}";
        let content_b = "pub fn b_with_lots_of_tokens_in_signature() {}";
        let fp_a = dir.path().join("src/small.rs");
        let fp_b = dir.path().join("src/big.rs");
        std::fs::create_dir_all(fp_a.parent().unwrap()).unwrap();
        std::fs::write(&fp_a, content_a).unwrap();
        std::fs::write(&fp_b, content_b).unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/small.rs".into(),
                absolute_path: fp_a,
                language: Some("rust".into()),
                size_bytes: content_a.len() as u64,
            },
            ScannedFile {
                relative_path: "src/big.rs".into(),
                absolute_path: fp_b,
                language: Some("rust".into()),
                size_bytes: content_b.len() as u64,
            },
        ];

        // "small" has a trivial signature (2 words + 2 = 4 tokens).
        // "big" has many words in the signature (plus 2) so it won't fit.
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/small.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "a".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    // 2 words => token cost = 2 + 2 = 4
                    signature: "pub fn".to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/big.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "b".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    // ~10 words => token cost well above 4.
                    signature:
                        "pub fn b one two three four five six seven eight nine ten eleven twelve"
                            .to_string(),
                    body: "{}".to_string(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);

        // Budget is 6 tokens — fits "small" (4) but not "big" (~16).
        let surface = extract_api_surface(&index, None, "all", 6);

        // Exactly one of the two files must be kept.
        assert_eq!(
            surface.symbols.by_file.len(),
            1,
            "exactly one file should fit the tight budget"
        );
        assert_eq!(
            surface.symbols.total, 1,
            "total kept count must match kept file symbols"
        );
        // The kept file must be the one with the smaller signature.
        assert_eq!(surface.symbols.by_file[0].path, "src/small.rs");
    }

    // --- extract_api_surface: gRPC + GraphQL end-to-end coverage ---
    //
    // Verify the orchestrator correctly wires both extractors when the
    // `include` knob is set to "all", with parseable proto and graphql
    // files present in the index.  This exercises the loop bodies in
    // `extract_grpc_services` and `extract_graphql_types` that walk
    // parse_result symbols.
    #[test]
    fn test_extract_api_surface_orchestrator_grpc_and_graphql_loops() {
        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let proto_content = "syntax = \"proto3\"; service Ping { rpc SendPing(); rpc Recv(); }";
        let gql_content = "type Foo { id: ID } type Query { items: [Foo] }";
        let fp_proto = dir.path().join("rpc/ping.proto");
        let fp_gql = dir.path().join("gql/schema.graphql");
        std::fs::create_dir_all(fp_proto.parent().unwrap()).unwrap();
        std::fs::create_dir_all(fp_gql.parent().unwrap()).unwrap();
        std::fs::write(&fp_proto, proto_content).unwrap();
        std::fs::write(&fp_gql, gql_content).unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "rpc/ping.proto".into(),
                absolute_path: fp_proto,
                language: Some("protobuf".into()),
                size_bytes: proto_content.len() as u64,
            },
            ScannedFile {
                relative_path: "gql/schema.graphql".into(),
                absolute_path: fp_gql,
                language: Some("graphql".into()),
                size_bytes: gql_content.len() as u64,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "rpc/ping.proto".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "Ping".to_string(),
                        kind: SymbolKind::Service,
                        visibility: Visibility::Public,
                        signature: "service Ping".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "SendPing".to_string(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc SendPing".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "Recv".to_string(),
                        kind: SymbolKind::Method,
                        visibility: Visibility::Public,
                        signature: "rpc Recv".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "gql/schema.graphql".to_string(),
            ParseResult {
                symbols: vec![
                    Symbol {
                        name: "Foo".to_string(),
                        kind: SymbolKind::Type,
                        visibility: Visibility::Public,
                        signature: "type Foo".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                    Symbol {
                        name: "Query".to_string(),
                        kind: SymbolKind::Query,
                        visibility: Visibility::Public,
                        signature: "type Query".to_string(),
                        body: String::new(),
                        start_line: 1,
                        end_line: 1,
                    },
                ],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build(files, parse_results, &counter);
        let surface = extract_api_surface(&index, None, "all", 100_000);

        // gRPC: one service with two methods.
        assert_eq!(surface.grpc_services.len(), 1);
        let svc = &surface.grpc_services[0];
        assert_eq!(svc.name, "Ping");
        assert_eq!(svc.file, "rpc/ping.proto");
        assert_eq!(svc.methods.len(), 2);
        assert!(svc.methods.contains(&"SendPing".to_string()));
        assert!(svc.methods.contains(&"Recv".to_string()));

        // GraphQL: two types extracted (Foo as "type", Query as "query").
        assert_eq!(surface.graphql_types.len(), 2);
        let foo = surface
            .graphql_types
            .iter()
            .find(|t| t.name == "Foo")
            .expect("Foo type should be extracted");
        assert_eq!(foo.kind, "type");
        assert_eq!(foo.file, "gql/schema.graphql");
        let query = surface
            .graphql_types
            .iter()
            .find(|t| t.name == "Query")
            .expect("Query type should be extracted");
        assert_eq!(query.kind, "query");

        // Token budget sanity: token_count is non-zero and under the budget.
        assert!(surface.token_count > 0);
        assert!(surface.token_count <= 100_000);
    }

    // ---- source-code gate tests ----

    #[test]
    fn test_detect_routes_skips_markdown() {
        let content = "# Example\n\n```javascript\napp.get('/users', getUsers);\n```\n";
        let routes = detect_routes(content, "docs/api.md");
        assert!(
            routes.is_empty(),
            "markdown files must not produce routes, got: {routes:?}"
        );
    }

    #[test]
    fn test_detect_routes_scans_javascript() {
        let content = "const express = require('express');\napp.get('/users', getUsers);";
        let routes = detect_routes(content, "src/app.js");
        assert_eq!(routes.len(), 1, "JavaScript route must be detected");
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].path, "/users");
        assert_eq!(routes[0].handler, "getUsers");
    }

    #[test]
    fn test_detect_routes_skips_txt() {
        let content = "app.get('/x', h);";
        assert!(
            detect_routes(content, "notes.txt").is_empty(),
            ".txt files must not produce routes"
        );
    }

    // ---- framework-import gate tests (Bug 6) ----

    #[test]
    fn test_detect_routes_requires_framework_import_rust() {
        // Rust file with route-looking string literal but no axum/actix import.
        let content = r#"let s = "app.get(\"/users\", getUsers)";"#;
        assert!(
            detect_routes(content, "src/foo.rs").is_empty(),
            "Rust file without framework import must produce no routes"
        );
    }

    #[test]
    fn test_detect_routes_requires_framework_import_js() {
        // JS file with route call but no express/koa import.
        let content = "app.get('/users', getUsers);";
        assert!(
            detect_routes(content, "src/app.js").is_empty(),
            "JS file without framework import must produce no routes"
        );
    }

    #[test]
    fn test_detect_routes_with_axum_import_no_panic() {
        // Axum import present — detect_routes runs without panicking.
        let content = "use axum::Router;\nuse axum::routing::get;\n\npub fn router() -> Router {\n    Router::new().route(\"/health\", get(health_handler))\n}\n";
        let _ = detect_routes(content, "src/app.rs");
    }

    #[test]
    fn test_detect_routes_express_with_import() {
        let content = "const express = require('express');\nconst app = express();\napp.get('/users', getUsers);\n";
        let routes = detect_routes(content, "src/app.js");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].path, "/users");
    }

    #[test]
    fn test_detect_routes_express_without_import_skipped() {
        // No framework import — must be skipped even if route pattern matches.
        let content = "app.get('/users', getUsers);";
        assert!(
            detect_routes(content, "src/app.js").is_empty(),
            "JS file without express import must not produce routes"
        );
    }

    #[test]
    fn test_has_framework_import_rust_axum() {
        assert!(has_framework_import("use axum::Router;", Some("rust")));
        assert!(!has_framework_import("fn main() {}", Some("rust")));
    }

    #[test]
    fn test_has_framework_import_rust_actix() {
        assert!(has_framework_import(
            "use actix_web::{get, post};",
            Some("rust")
        ));
    }

    #[test]
    fn test_has_framework_import_python_flask() {
        assert!(has_framework_import(
            "from flask import Flask",
            Some("python")
        ));
        assert!(!has_framework_import("import os", Some("python")));
    }

    #[test]
    fn test_has_framework_import_python_fastapi() {
        assert!(has_framework_import(
            "from fastapi import FastAPI",
            Some("python")
        ));
    }

    #[test]
    fn test_has_framework_import_js_express_require() {
        assert!(has_framework_import(
            "const express = require('express');",
            Some("javascript")
        ));
    }

    #[test]
    fn test_has_framework_import_ts_express_esm() {
        assert!(has_framework_import(
            "import express from 'express';",
            Some("typescript")
        ));
    }

    #[test]
    fn test_has_framework_import_go_gin() {
        assert!(has_framework_import(
            "import \"github.com/gin-gonic/gin\"",
            Some("go")
        ));
        assert!(!has_framework_import("import \"fmt\"", Some("go")));
    }

    #[test]
    fn test_has_framework_import_unknown_lang_returns_false() {
        assert!(!has_framework_import(
            "app.get('/x', h);",
            Some("brainfuck")
        ));
        assert!(!has_framework_import("app.get('/x', h);", None));
    }

    #[test]
    fn test_language_from_path_known_extensions() {
        assert_eq!(language_from_path("src/main.rs"), "rust");
        assert_eq!(language_from_path("app.py"), "python");
        assert_eq!(language_from_path("index.js"), "javascript");
        assert_eq!(language_from_path("app.ts"), "typescript");
        assert_eq!(language_from_path("main.go"), "go");
        assert_eq!(language_from_path("app.rb"), "ruby");
        assert_eq!(language_from_path("Foo.java"), "java");
        assert_eq!(language_from_path("Foo.kt"), "kotlin");
        assert_eq!(language_from_path("router.ex"), "elixir");
        assert_eq!(language_from_path("Foo.cs"), "csharp");
    }

    #[test]
    fn test_language_from_path_unknown_extension() {
        assert_eq!(language_from_path("file.xyz"), "");
        assert_eq!(language_from_path("no_extension"), "");
    }
}
