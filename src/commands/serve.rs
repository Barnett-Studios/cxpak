use crate::budget::counter::TokenCounter;
use crate::commands::watch::{apply_incremental_update, classify_changes};
use crate::daemon::watcher::FileWatcher;
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

/// Scan and parse all files in a path, returning a fully built CodebaseIndex.
pub(crate) fn build_index(path: &Path) -> Result<CodebaseIndex, Box<dyn std::error::Error>> {
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

    Ok(CodebaseIndex::build_with_content(
        files,
        parse_results,
        &counter,
        content_map,
    ))
}

/// Build the axum Router for the HTTP server.
fn build_router(shared: SharedIndex) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/overview", get(overview_handler))
        .route("/trace", get(trace_handler))
        .with_state(shared)
}

pub fn run(
    path: &Path,
    port: u16,
    _token_budget: usize,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let index = build_index(path)?;

    eprintln!(
        "cxpak: serving {} ({} files indexed, {} tokens) on port {}",
        path.display(),
        index.total_files,
        index.total_tokens,
        port
    );

    let shared = Arc::new(RwLock::new(index));

    // Background watcher thread
    let watcher_path = path.to_path_buf();
    let watcher_index = Arc::clone(&shared);
    std::thread::spawn(move || {
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
                process_watcher_changes(&changes, &watcher_path, &watcher_index);
            }
        }
    });

    let app = build_router(shared);

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
    let index = build_index(path)?;

    eprintln!(
        "cxpak: MCP server ready ({} files indexed, {} tokens)",
        index.total_files, index.total_tokens
    );

    mcp_stdio_loop(&index)
}

fn mcp_stdio_loop(index: &CodebaseIndex) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    mcp_stdio_loop_with_io(index, stdin.lock(), &mut stdout.lock())
}

fn mcp_stdio_loop_with_io(
    index: &CodebaseIndex,
    reader: impl std::io::BufRead,
    writer: &mut impl std::io::Write,
) -> Result<(), Box<dyn std::error::Error>> {
    for line in reader.lines() {
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

        serde_json::to_writer(&mut *writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
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

/// Process a batch of watcher changes, updating the shared index.
fn process_watcher_changes(
    changes: &[crate::daemon::watcher::FileChange],
    base_path: &Path,
    shared: &SharedIndex,
) {
    let (modified_paths, removed_paths) = classify_changes(changes, base_path);

    if let Ok(mut idx) = shared.write() {
        let update_count =
            apply_incremental_update(&mut idx, base_path, &modified_paths, &removed_paths);
        if update_count > 0 {
            eprintln!(
                "cxpak: updated {} file(s), {} files / {} tokens total",
                update_count, idx.total_files, idx.total_tokens
            );
        }
    }
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
    use crate::budget::counter::TokenCounter;
    use crate::index::CodebaseIndex;
    use crate::scanner::ScannedFile;
    use tower::ServiceExt;

    /// Build a minimal CodebaseIndex for testing handlers.
    fn make_test_index() -> CodebaseIndex {
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
        ];

        let mut parse_results = HashMap::new();
        use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
        parse_results.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "main".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn main()".to_string(),
                    body: "fn main() {}".to_string(),
                    start_line: 1,
                    end_line: 5,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());
        content_map.insert("src/lib.rs".to_string(), "pub fn hello() {}".to_string());

        CodebaseIndex::build_with_content(files, parse_results, &counter, content_map)
    }

    fn make_shared_index() -> SharedIndex {
        Arc::new(RwLock::new(make_test_index()))
    }

    // --- Health handler ---

    #[test]
    fn test_health_handler_returns_ok() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(health_handler());
        assert_eq!(result.0["status"], "ok");
    }

    // --- Stats handler ---

    #[test]
    fn test_stats_handler_returns_index_stats() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let result = rt.block_on(stats_handler(State(shared))).unwrap();
        assert_eq!(result.0["files"], 2);
        assert!(result.0["tokens"].as_u64().unwrap() > 0);
        assert!(result.0["languages"].as_u64().unwrap() >= 1);
    }

    // --- Overview handler ---

    #[test]
    fn test_overview_handler_defaults() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = OverviewParams {
            tokens: None,
            format: None,
        };
        let result = rt
            .block_on(overview_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["format"], "json");
        assert_eq!(result.0["token_budget"], 50_000);
        assert_eq!(result.0["total_files"], 2);
        assert!(result.0["languages"].as_array().is_some());
    }

    #[test]
    fn test_overview_handler_custom_params() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = OverviewParams {
            tokens: Some("100k".to_string()),
            format: Some("markdown".to_string()),
        };
        let result = rt
            .block_on(overview_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["format"], "markdown");
        assert_eq!(result.0["token_budget"], 100_000);
    }

    #[test]
    fn test_overview_handler_invalid_tokens_uses_default() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = OverviewParams {
            tokens: Some("not_a_number".to_string()),
            format: None,
        };
        let result = rt
            .block_on(overview_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["token_budget"], 50_000);
    }

    #[test]
    fn test_overview_handler_languages_array() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = OverviewParams {
            tokens: None,
            format: None,
        };
        let result = rt
            .block_on(overview_handler(State(shared), Query(params)))
            .unwrap();
        let langs = result.0["languages"].as_array().unwrap();
        assert!(!langs.is_empty());
        let first = &langs[0];
        assert!(first["language"].is_string());
        assert!(first["files"].is_number());
        assert!(first["tokens"].is_number());
    }

    // --- Trace handler ---

    #[test]
    fn test_trace_handler_missing_target() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: None,
            tokens: None,
        };
        let result = rt
            .block_on(trace_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(
            result.0["error"],
            "missing required query parameter: target"
        );
    }

    #[test]
    fn test_trace_handler_empty_target() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("".to_string()),
            tokens: None,
        };
        let result = rt
            .block_on(trace_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(
            result.0["error"],
            "missing required query parameter: target"
        );
    }

    #[test]
    fn test_trace_handler_symbol_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("main".to_string()),
            tokens: None,
        };
        let result = rt
            .block_on(trace_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["target"], "main");
        assert_eq!(result.0["found"], true);
        assert_eq!(result.0["token_budget"], 50_000);
    }

    #[test]
    fn test_trace_handler_content_match() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("hello".to_string()),
            tokens: None,
        };
        let result = rt
            .block_on(trace_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["target"], "hello");
        assert_eq!(result.0["found"], true);
    }

    #[test]
    fn test_trace_handler_not_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("nonexistent_xyz".to_string()),
            tokens: None,
        };
        let result = rt
            .block_on(trace_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["target"], "nonexistent_xyz");
        assert_eq!(result.0["found"], false);
    }

    #[test]
    fn test_trace_handler_custom_tokens() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("main".to_string()),
            tokens: Some("10k".to_string()),
        };
        let result = rt
            .block_on(trace_handler(State(shared), Query(params)))
            .unwrap();
        assert_eq!(result.0["token_budget"], 10_000);
    }

    // --- handle_tool_call ---

    #[test]
    fn test_handle_tool_call_stats() {
        let index = make_test_index();
        let resp = handle_tool_call(Some(json!(1)), "cxpak_stats", &json!({}), &index);
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["files"], 2);
        assert!(parsed["tokens"].as_u64().unwrap() > 0);
        assert!(parsed["languages"].as_array().is_some());
    }

    #[test]
    fn test_handle_tool_call_overview() {
        let index = make_test_index();
        let resp = handle_tool_call(Some(json!(2)), "cxpak_overview", &json!({}), &index);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["total_files"], 2);
        assert!(parsed["languages"].as_array().is_some());
    }

    #[test]
    fn test_handle_tool_call_trace_found() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(3)),
            "cxpak_trace",
            &json!({"target": "main"}),
            &index,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["target"], "main");
        assert_eq!(parsed["found"], true);
        assert!(parsed["symbol_matches"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_handle_tool_call_trace_content_fallback() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(4)),
            "cxpak_trace",
            &json!({"target": "hello"}),
            &index,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["found"], true);
        assert!(parsed["content_matches"].as_u64().unwrap() > 0);
        assert_eq!(parsed["symbol_matches"], 0);
    }

    #[test]
    fn test_handle_tool_call_trace_not_found() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(5)),
            "cxpak_trace",
            &json!({"target": "nonexistent_xyz"}),
            &index,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["found"], false);
    }

    #[test]
    fn test_handle_tool_call_trace_empty_target() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(6)),
            "cxpak_trace",
            &json!({"target": ""}),
            &index,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("required"));
    }

    #[test]
    fn test_handle_tool_call_trace_missing_target_arg() {
        let index = make_test_index();
        let resp = handle_tool_call(Some(json!(7)), "cxpak_trace", &json!({}), &index);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("required"));
    }

    #[test]
    fn test_handle_tool_call_unknown_tool() {
        let index = make_test_index();
        let resp = handle_tool_call(Some(json!(8)), "unknown_tool", &json!({}), &index);
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Unknown tool"));
    }

    // --- MCP stdio loop ---

    #[test]
    fn test_mcp_stdio_loop_initialize() {
        let index = make_test_index();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "cxpak");
    }

    #[test]
    fn test_mcp_stdio_loop_tools_list() {
        let index = make_test_index();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
    }

    #[test]
    fn test_mcp_stdio_loop_tool_call() {
        let index = make_test_index();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_stats","arguments":{}}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["files"], 2);
    }

    #[test]
    fn test_mcp_stdio_loop_unknown_method() {
        let index = make_test_index();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"unknown/method","params":{}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_mcp_stdio_loop_notification_skipped() {
        let index = make_test_index();
        // notifications/initialized should produce no output
        let input = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_empty_lines_skipped() {
        let index = make_test_index();
        let input = "\n\n\n".to_string();
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_invalid_json_skipped() {
        let index = make_test_index();
        let input = "not json\n".to_string();
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_multiple_messages() {
        let index = make_test_index();
        let input = format!(
            "{}\n{}\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        );
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(&index, cursor, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = text.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
        let resp1: Value = serde_json::from_str(lines[0]).unwrap();
        let resp2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(resp1["id"], 1);
        assert_eq!(resp2["id"], 2);
    }

    // --- Param struct tests (kept) ---

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

    // --- MCP helper function tests ---

    #[test]
    fn test_mcp_response_structure() {
        let resp = mcp_response(Some(json!(1)), json!({"status": "ok"}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["status"], "ok");
    }

    #[test]
    fn test_mcp_response_null_id() {
        let resp = mcp_response(None, json!({"status": "ok"}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert!(resp["id"].is_null());
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

    // --- build_index ---

    #[test]
    fn test_build_index_from_temp_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        // Initialize a git repo (build_index requires Scanner which needs git)
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let index = build_index(dir.path()).unwrap();
        assert_eq!(index.total_files, 1);
        assert!(index.total_tokens > 0);
    }

    #[test]
    fn test_build_index_empty_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        let index = build_index(dir.path()).unwrap();
        assert_eq!(index.total_files, 0);
        assert_eq!(index.total_tokens, 0);
    }

    #[test]
    fn test_build_index_not_a_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = build_index(dir.path());
        assert!(result.is_err());
    }

    // --- build_router ---

    #[test]
    fn test_build_router_creates_router() {
        let shared = make_shared_index();
        let _router = build_router(shared);
        // Router created without panic = success
    }

    // --- Axum integration (in-process HTTP) ---

    #[test]
    fn test_axum_health_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/health")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["status"], "ok");
        });
    }

    #[test]
    fn test_axum_stats_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/stats")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["files"], 2);
        });
    }

    #[test]
    fn test_axum_overview_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/overview?tokens=10k&format=xml")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["format"], "xml");
            assert_eq!(json["token_budget"], 10_000);
        });
    }

    #[test]
    fn test_axum_trace_endpoint_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/trace?target=main")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["found"], true);
        });
    }

    #[test]
    fn test_axum_trace_endpoint_not_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/trace?target=nonexistent_xyz")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["found"], false);
        });
    }

    #[test]
    fn test_axum_trace_endpoint_missing_target() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/trace")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap();
            assert!(json["error"].as_str().unwrap().contains("missing"));
        });
    }

    #[test]
    fn test_axum_404_unknown_route() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/nonexistent")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        });
    }

    // --- process_watcher_changes ---

    #[test]
    fn test_process_watcher_changes_modify() {
        use crate::daemon::watcher::FileChange;

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "fn updated() {}").unwrap();

        let shared = make_shared_index();
        let changes = vec![FileChange::Modified(file_path)];

        process_watcher_changes(&changes, dir.path(), &shared);

        let idx = shared.read().unwrap();
        // Original index had 2 files; the modified file wasn't one of them,
        // so it gets added as a new file (upsert)
        assert!(idx.total_files >= 2);
    }

    #[test]
    fn test_process_watcher_changes_remove() {
        use crate::daemon::watcher::FileChange;

        let dir = tempfile::TempDir::new().unwrap();
        let shared = make_shared_index();

        // Remove a file that exists in the index
        let changes = vec![FileChange::Removed(dir.path().join("src/main.rs"))];

        process_watcher_changes(&changes, dir.path(), &shared);

        let idx = shared.read().unwrap();
        assert_eq!(idx.total_files, 1); // Was 2, now 1
    }

    #[test]
    fn test_process_watcher_changes_create() {
        use crate::daemon::watcher::FileChange;

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("new.rs");
        std::fs::write(&file_path, "fn brand_new() {}").unwrap();

        let shared = make_shared_index();

        let changes = vec![FileChange::Created(file_path)];
        process_watcher_changes(&changes, dir.path(), &shared);

        let idx = shared.read().unwrap();
        assert_eq!(idx.total_files, 3); // Was 2, added 1
    }

    #[test]
    fn test_process_watcher_changes_mixed() {
        use crate::daemon::watcher::FileChange;

        let dir = tempfile::TempDir::new().unwrap();
        let new_file = dir.path().join("added.rs");
        std::fs::write(&new_file, "fn added() {}").unwrap();

        let shared = make_shared_index();

        let changes = vec![
            FileChange::Created(new_file),
            FileChange::Removed(dir.path().join("src/lib.rs")),
        ];
        process_watcher_changes(&changes, dir.path(), &shared);

        let idx = shared.read().unwrap();
        // 2 original - 1 removed + 1 added = 2
        assert_eq!(idx.total_files, 2);
    }

    #[test]
    fn test_process_watcher_changes_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = make_shared_index();

        process_watcher_changes(&[], dir.path(), &shared);

        let idx = shared.read().unwrap();
        assert_eq!(idx.total_files, 2); // Unchanged
    }

    #[test]
    fn test_process_watcher_changes_outside_base_ignored() {
        use crate::daemon::watcher::FileChange;

        let dir = tempfile::TempDir::new().unwrap();
        let shared = make_shared_index();

        // File outside base path should be ignored
        let changes = vec![FileChange::Created(std::path::PathBuf::from(
            "/other/path/file.rs",
        ))];
        process_watcher_changes(&changes, dir.path(), &shared);

        let idx = shared.read().unwrap();
        assert_eq!(idx.total_files, 2); // Unchanged
    }

    // --- Poisoned lock error path ---

    #[test]
    fn test_stats_handler_poisoned_lock() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();

        // Poison the lock by panicking while holding a write guard
        let shared2 = Arc::clone(&shared);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = shared2.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        let result = rt.block_on(stats_handler(State(shared)));
        assert!(result.is_err());
    }

    #[test]
    fn test_trace_handler_poisoned_lock() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();

        let shared2 = Arc::clone(&shared);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = shared2.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        let params = TraceParams {
            target: Some("main".to_string()),
            tokens: None,
        };
        let result = rt.block_on(trace_handler(State(shared), Query(params)));
        assert!(result.is_err());
    }

    #[test]
    fn test_overview_handler_poisoned_lock() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();

        let shared2 = Arc::clone(&shared);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = shared2.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        let params = OverviewParams {
            tokens: None,
            format: None,
        };
        let result = rt.block_on(overview_handler(State(shared), Query(params)));
        assert!(result.is_err());
    }
}
