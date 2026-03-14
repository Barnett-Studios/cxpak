use crate::budget::counter::TokenCounter;
use crate::daemon::watcher::{FileChange, FileWatcher};
use crate::index::CodebaseIndex;
use crate::parser::LanguageRegistry;
use crate::scanner::Scanner;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

type SharedIndex = Arc<RwLock<CodebaseIndex>>;

pub fn run(
    path: &Path,
    port: u16,
    _token_budget: usize,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let registry = LanguageRegistry::new();

    // Initial full build (same pattern as watch.rs)
    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;

    let mut parse_results = HashMap::new();
    let mut content_map = HashMap::new();
    for file in &files {
        let source = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
        if let Some(lang_name) = &file.language {
            if let Some(lang) = registry.get(lang_name) {
                let ts_lang = lang.ts_language();
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&ts_lang).ok();
                if let Some(tree) = parser.parse(&source, None) {
                    let result = lang.extract(&source, &tree);
                    parse_results.insert(file.relative_path.clone(), result);
                }
            }
        }
        content_map.insert(file.relative_path.clone(), source);
    }

    let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    eprintln!(
        "cxpak: serving {} ({} files indexed, {} tokens) on port {}",
        path.display(),
        index.total_files,
        index.total_tokens,
        port
    );

    let shared = Arc::new(RwLock::new(index));

    // Background watcher thread — uses std::thread since FileWatcher uses std::sync::mpsc
    let watcher_path = path.to_path_buf();
    let watcher_index = Arc::clone(&shared);
    std::thread::spawn(move || {
        let counter = TokenCounter::new();
        let registry = LanguageRegistry::new();
        let watcher = match FileWatcher::new(&watcher_path) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("cxpak: watcher failed to start: {e}");
                return;
            }
        };

        loop {
            if let Some(first) = watcher.recv_timeout(Duration::from_secs(1)) {
                let mut changes = vec![first];
                std::thread::sleep(Duration::from_millis(50));
                changes.extend(watcher.drain());

                let mut modified_paths = std::collections::HashSet::new();
                let mut removed_paths = std::collections::HashSet::new();

                for change in changes {
                    match change {
                        FileChange::Created(p) | FileChange::Modified(p) => {
                            if let Ok(rel) = p.strip_prefix(&watcher_path) {
                                modified_paths.insert(rel.to_string_lossy().to_string());
                            }
                        }
                        FileChange::Removed(p) => {
                            if let Ok(rel) = p.strip_prefix(&watcher_path) {
                                removed_paths.insert(rel.to_string_lossy().to_string());
                            }
                        }
                    }
                }

                let mut update_count = 0;

                if let Ok(mut idx) = watcher_index.write() {
                    for rel_path in &removed_paths {
                        idx.remove_file(rel_path);
                        update_count += 1;
                    }

                    for rel_path in &modified_paths {
                        if removed_paths.contains(rel_path) {
                            continue;
                        }
                        let abs_path = watcher_path.join(rel_path);
                        if let Ok(content) = std::fs::read_to_string(&abs_path) {
                            let lang_name = crate::scanner::detect_language(Path::new(rel_path));
                            let parse_result = lang_name.as_deref().and_then(|ln| {
                                registry.get(ln).and_then(|lang| {
                                    let ts_lang = lang.ts_language();
                                    let mut parser = tree_sitter::Parser::new();
                                    parser.set_language(&ts_lang).ok()?;
                                    let tree = parser.parse(&content, None)?;
                                    Some(lang.extract(&content, &tree))
                                })
                            });

                            idx.upsert_file(
                                rel_path,
                                lang_name.as_deref(),
                                &content,
                                parse_result,
                                &counter,
                            );
                            update_count += 1;
                        }
                    }
                }

                if update_count > 0 {
                    if let Ok(idx) = watcher_index.read() {
                        eprintln!(
                            "cxpak: updated {} file(s), {} files / {} tokens total",
                            update_count, idx.total_files, idx.total_tokens
                        );
                    }
                }
            }
        }
    });

    // Build axum router with shared state
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/overview", get(overview_handler))
        .route("/trace", get(trace_handler))
        .with_state(shared);

    // Run the async HTTP server using a fresh tokio runtime
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        eprintln!("cxpak: listening on http://{addr}");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok::<(), std::io::Error>(())
    })?;

    Ok(())
}

async fn health_handler() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn stats_handler(State(index): State<SharedIndex>) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({
        "files": idx.total_files,
        "tokens": idx.total_tokens,
        "languages": idx.language_stats.len(),
    })))
}

#[derive(Deserialize)]
struct OverviewParams {
    tokens: Option<String>,
    format: Option<String>,
}

async fn overview_handler(
    State(index): State<SharedIndex>,
    Query(params): Query<OverviewParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);
    let format = params.format.as_deref().unwrap_or("json");

    let languages: Vec<Value> = idx
        .language_stats
        .iter()
        .map(|(lang, stats)| {
            json!({
                "language": lang,
                "files": stats.file_count,
                "tokens": stats.total_tokens,
            })
        })
        .collect();

    Ok(Json(json!({
        "format": format,
        "token_budget": token_budget,
        "total_files": idx.total_files,
        "total_tokens": idx.total_tokens,
        "languages": languages,
    })))
}

#[derive(Deserialize)]
struct TraceParams {
    target: Option<String>,
    tokens: Option<String>,
}

async fn trace_handler(
    State(index): State<SharedIndex>,
    Query(params): Query<TraceParams>,
) -> Result<Json<Value>, StatusCode> {
    let target = match params.target {
        Some(t) if !t.is_empty() => t,
        _ => {
            return Ok(Json(json!({
                "error": "missing required query parameter: target"
            })));
        }
    };

    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);

    let found =
        !idx.find_symbol(&target).is_empty() || !idx.find_content_matches(&target).is_empty();

    Ok(Json(json!({
        "target": target,
        "token_budget": token_budget,
        "found": found,
        "total_files": idx.total_files,
        "total_tokens": idx.total_tokens,
    })))
}

// --- MCP server mode (JSON-RPC over stdio) ---

pub fn run_mcp(
    path: &Path,
    _token_budget: usize,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let registry = LanguageRegistry::new();

    let scanner = Scanner::new(path)?;
    let files = scanner.scan()?;

    let mut parse_results = HashMap::new();
    let mut content_map = HashMap::new();
    for file in &files {
        let source = std::fs::read_to_string(&file.absolute_path).unwrap_or_default();
        if let Some(lang_name) = &file.language {
            if let Some(lang) = registry.get(lang_name) {
                let ts_lang = lang.ts_language();
                let mut parser = tree_sitter::Parser::new();
                parser.set_language(&ts_lang).ok();
                if let Some(tree) = parser.parse(&source, None) {
                    let result = lang.extract(&source, &tree);
                    parse_results.insert(file.relative_path.clone(), result);
                }
            }
        }
        content_map.insert(file.relative_path.clone(), source);
    }

    let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    eprintln!(
        "cxpak: MCP server ready ({} files indexed, {} tokens)",
        index.total_files, index.total_tokens
    );

    mcp_stdio_loop(&index)
}

fn mcp_stdio_loop(index: &CodebaseIndex) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = request.get("id").cloned();
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => mcp_response(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "cxpak",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),
            "notifications/initialized" => continue, // no response for notifications
            "tools/list" => mcp_response(
                id,
                json!({
                    "tools": [
                        {
                            "name": "cxpak_overview",
                            "description": "Get a structured overview of the codebase",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "tokens": {
                                        "type": "string",
                                        "description": "Token budget (e.g. '50k', '100k')",
                                        "default": "50k"
                                    }
                                }
                            }
                        },
                        {
                            "name": "cxpak_trace",
                            "description": "Trace a symbol through the codebase dependency graph",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "target": {
                                        "type": "string",
                                        "description": "Symbol or text to trace"
                                    },
                                    "tokens": {
                                        "type": "string",
                                        "description": "Token budget",
                                        "default": "50k"
                                    }
                                },
                                "required": ["target"]
                            }
                        },
                        {
                            "name": "cxpak_stats",
                            "description": "Get index statistics (file count, tokens, languages)",
                            "inputSchema": {
                                "type": "object",
                                "properties": {}
                            }
                        }
                    ]
                }),
            ),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                handle_tool_call(id, tool_name, &arguments, index)
            }
            _ => mcp_error_response(id, -32601, "Method not found"),
        };

        let mut out = stdout.lock();
        serde_json::to_writer(&mut out, &response)?;
        out.write_all(b"\n")?;
        out.flush()?;
    }

    Ok(())
}

fn handle_tool_call(
    id: Option<Value>,
    tool_name: &str,
    args: &Value,
    index: &CodebaseIndex,
) -> Value {
    match tool_name {
        "cxpak_stats" => {
            let languages: Vec<Value> = index
                .language_stats
                .iter()
                .map(|(lang, stats)| {
                    json!({"language": lang, "files": stats.file_count, "tokens": stats.total_tokens})
                })
                .collect();

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "files": index.total_files,
                    "tokens": index.total_tokens,
                    "languages": languages,
                }))
                .unwrap_or_default(),
            )
        }
        "cxpak_overview" => {
            let languages: Vec<Value> = index
                .language_stats
                .iter()
                .map(|(lang, stats)| {
                    json!({"language": lang, "files": stats.file_count, "tokens": stats.total_tokens})
                })
                .collect();

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "total_files": index.total_files,
                    "total_tokens": index.total_tokens,
                    "languages": languages,
                }))
                .unwrap_or_default(),
            )
        }
        "cxpak_trace" => {
            let target = args.get("target").and_then(|t| t.as_str()).unwrap_or("");
            if target.is_empty() {
                return mcp_tool_result(id, "Error: 'target' argument is required");
            }

            let symbol_matches = index.find_symbol(target);
            let content_matches = if symbol_matches.is_empty() {
                index.find_content_matches(target)
            } else {
                vec![]
            };

            let found = !symbol_matches.is_empty() || !content_matches.is_empty();

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "target": target,
                    "found": found,
                    "symbol_matches": symbol_matches.len(),
                    "content_matches": content_matches.len(),
                    "total_files": index.total_files,
                }))
                .unwrap_or_default(),
            )
        }
        _ => mcp_response(
            id,
            json!({
                "content": [{"type": "text", "text": format!("Unknown tool: {tool_name}")}],
                "isError": true
            }),
        ),
    }
}

fn mcp_response(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn mcp_tool_result(id: Option<Value>, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{"type": "text", "text": text}]
        }
    })
}

fn mcp_error_response(id: Option<Value>, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_handler_returns_ok() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(health_handler());
        assert_eq!(result.0["status"], "ok");
    }

    #[test]
    fn test_overview_params_defaults() {
        let params = OverviewParams {
            tokens: None,
            format: None,
        };
        let token_budget = params
            .tokens
            .as_deref()
            .and_then(|t| crate::cli::parse_token_count(t).ok())
            .unwrap_or(50_000);
        assert_eq!(token_budget, 50_000);
        assert_eq!(params.format.as_deref().unwrap_or("json"), "json");
    }

    #[test]
    fn test_overview_params_custom_tokens() {
        let params = OverviewParams {
            tokens: Some("100k".to_string()),
            format: Some("markdown".to_string()),
        };
        let token_budget = params
            .tokens
            .as_deref()
            .and_then(|t| crate::cli::parse_token_count(t).ok())
            .unwrap_or(50_000);
        assert_eq!(token_budget, 100_000);
        assert_eq!(params.format.as_deref().unwrap_or("json"), "markdown");
    }

    #[test]
    fn test_trace_params_missing_target() {
        let params = TraceParams {
            target: None,
            tokens: None,
        };
        assert!(params.target.is_none());
    }

    #[test]
    fn test_trace_params_with_target() {
        let params = TraceParams {
            target: Some("my_function".to_string()),
            tokens: Some("50k".to_string()),
        };
        assert_eq!(params.target.as_deref(), Some("my_function"));
        let budget = params
            .tokens
            .as_deref()
            .and_then(|t| crate::cli::parse_token_count(t).ok())
            .unwrap_or(50_000);
        assert_eq!(budget, 50_000);
    }

    #[test]
    fn test_mcp_response_structure() {
        let resp = mcp_response(Some(json!(1)), json!({"status": "ok"}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "ok");
    }

    #[test]
    fn test_mcp_tool_result_structure() {
        let resp = mcp_tool_result(Some(json!(2)), "hello world");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 2);
        assert_eq!(resp["result"]["content"][0]["type"], "text");
        assert_eq!(resp["result"]["content"][0]["text"], "hello world");
    }

    #[test]
    fn test_mcp_error_response_structure() {
        let resp = mcp_error_response(Some(json!(3)), -32601, "Method not found");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 3);
        assert_eq!(resp["error"]["code"], -32601);
        assert_eq!(resp["error"]["message"], "Method not found");
    }
}
