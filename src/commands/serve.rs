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
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

type SharedIndex = Arc<RwLock<CodebaseIndex>>;

fn matches_focus(path: &str, focus: Option<&str>) -> bool {
    focus.map_or(true, |f| path.starts_with(f))
}

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

type SharedPath = Arc<std::path::PathBuf>;

#[derive(Clone)]
struct AppState {
    index: SharedIndex,
    repo_path: SharedPath,
}

impl axum::extract::FromRef<AppState> for SharedIndex {
    fn from_ref(state: &AppState) -> Self {
        state.index.clone()
    }
}

impl axum::extract::FromRef<AppState> for SharedPath {
    fn from_ref(state: &AppState) -> Self {
        state.repo_path.clone()
    }
}

/// Build the axum Router for the HTTP server.
fn build_router(shared: SharedIndex, repo_path: SharedPath) -> Router {
    let state = AppState {
        index: shared,
        repo_path,
    };
    Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/overview", get(overview_handler))
        .route("/trace", get(trace_handler))
        .route("/diff", get(diff_handler))
        .route("/search", axum::routing::post(search_handler))
        .with_state(state)
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
    let shared_path = Arc::new(path.to_path_buf());

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

    let app = build_router(shared, shared_path);

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
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

#[derive(Deserialize)]
struct DiffParams {
    git_ref: Option<String>,
    tokens: Option<String>,
}

async fn diff_handler(
    State(repo_path): State<SharedPath>,
    Query(params): Query<DiffParams>,
) -> Result<Json<Value>, StatusCode> {
    let git_ref = params.git_ref.as_deref();
    let _token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);

    let changes = crate::commands::diff::extract_changes(&repo_path, git_ref)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let files: Vec<Value> = changes
        .iter()
        .map(|c| {
            json!({
                "path": c.path,
                "diff": c.diff_text,
            })
        })
        .collect();

    Ok(Json(json!({
        "git_ref": git_ref.unwrap_or("working tree"),
        "changed_files": changes.len(),
        "files": files,
    })))
}

#[derive(Deserialize)]
struct SearchParams {
    pattern: String,
    limit: Option<usize>,
    focus: Option<String>,
    context_lines: Option<usize>,
}

async fn search_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<SearchParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if params.pattern.is_empty() {
        return Ok(Json(
            json!({"error": "pattern is required and must not be empty"}),
        ));
    }

    let re = match regex::Regex::new(&params.pattern) {
        Ok(r) => r,
        Err(e) => return Ok(Json(json!({"error": format!("invalid regex: {e}")}))),
    };

    let limit = params.limit.unwrap_or(20);
    let focus = params.focus.as_deref();
    let context_lines = params.context_lines.unwrap_or(2);

    let mut matches_vec = vec![];
    let mut total_matches = 0usize;
    let mut files_searched = 0usize;

    for file in &idx.files {
        if !matches_focus(&file.relative_path, focus) {
            continue;
        }
        if file.content.is_empty() {
            continue;
        }
        files_searched += 1;

        let lines: Vec<&str> = file.content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                total_matches += 1;
                if matches_vec.len() < limit {
                    let start = i.saturating_sub(context_lines);
                    let end = (i + context_lines + 1).min(lines.len());
                    let ctx_before: Vec<&str> = lines[start..i].to_vec();
                    let ctx_after: Vec<&str> = lines[(i + 1)..end].to_vec();
                    matches_vec.push(json!({
                        "path": &file.relative_path,
                        "line": i + 1,
                        "content": line,
                        "context_before": ctx_before,
                        "context_after": ctx_after,
                    }));
                }
            }
        }
    }

    Ok(Json(json!({
        "pattern": params.pattern,
        "matches": matches_vec,
        "total_matches": total_matches,
        "files_searched": files_searched,
        "truncated": total_matches > limit,
    })))
}

// --- MCP server mode (JSON-RPC over stdio) ---

pub fn run_mcp(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let index = build_index(path)?;

    eprintln!(
        "cxpak: MCP server ready ({} files indexed, {} tokens)",
        index.total_files, index.total_tokens
    );

    mcp_stdio_loop(path, &index)
}

/// Run the MCP stdio loop.
///
/// NOTE: The index is built once at startup and not refreshed during the
/// session. This is acceptable because MCP connections are typically
/// short-lived (one task ≈ one connection). If long-lived sessions become
/// common, consider rebuilding the index periodically.
fn mcp_stdio_loop(
    repo_path: &Path,
    index: &CodebaseIndex,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    mcp_stdio_loop_with_io(repo_path, index, stdin.lock(), &mut stdout.lock())
}

fn mcp_stdio_loop_with_io(
    repo_path: &Path,
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
                                    },
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
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
                                    },
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
                                },
                                "required": ["target"]
                            }
                        },
                        {
                            "name": "cxpak_diff",
                            "description": "Show changes with dependency context",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "git_ref": {
                                        "type": "string",
                                        "description": "Git ref to diff against (e.g. 'main', 'HEAD~1'). Omit to diff working tree vs HEAD."
                                    },
                                    "tokens": {
                                        "type": "string",
                                        "description": "Token budget",
                                        "default": "50k"
                                    },
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
                                }
                            }
                        },
                        {
                            "name": "cxpak_stats",
                            "description": "Get index statistics (file count, tokens, languages)",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
                                }
                            }
                        },
                        {
                            "name": "cxpak_context_for_task",
                            "description": "Score and rank codebase files by relevance to a task description",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "task": { "type": "string", "description": "Natural language task description" },
                                    "limit": { "type": "number", "description": "Maximum number of candidates to return (default 15)" },
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
                                },
                                "required": ["task"]
                            }
                        },
                        {
                            "name": "cxpak_pack_context",
                            "description": "Pack selected files into a token-budgeted context bundle with dependency context",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "files": { "type": "array", "items": { "type": "string" }, "description": "File paths to include" },
                                    "tokens": { "type": "string", "description": "Token budget (e.g. '30k', '50k')", "default": "50k" },
                                    "include_dependencies": { "type": "boolean", "description": "Include 1-hop dependencies", "default": false },
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
                                },
                                "required": ["files"]
                            }
                        },
                        {
                            "name": "cxpak_search",
                            "description": "Search codebase content with regex patterns. Returns matching lines with surrounding context.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                                    "limit": { "type": "number", "description": "Maximum number of matches to return (default 20)", "default": 20 },
                                    "focus": { "type": "string", "description": "Path prefix to scope search (e.g. 'src/api/')" },
                                    "context_lines": { "type": "number", "description": "Lines of context before and after each match (default 2)", "default": 2 }
                                },
                                "required": ["pattern"]
                            }
                        }
                    ]
                }),
            ),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                handle_tool_call(id, tool_name, &arguments, index, repo_path)
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
    repo_path: &Path,
) -> Value {
    match tool_name {
        "cxpak_stats" => {
            let focus = args.get("focus").and_then(|f| f.as_str());

            if focus.is_some() {
                // Recompute stats from files matching focus
                let mut lang_counts: HashMap<String, (usize, usize)> = HashMap::new();
                let mut total_files = 0usize;
                let mut total_tokens = 0usize;
                for file in &index.files {
                    if !matches_focus(&file.relative_path, focus) {
                        continue;
                    }
                    total_files += 1;
                    total_tokens += file.token_count;
                    if let Some(ref lang) = file.language {
                        let entry = lang_counts.entry(lang.clone()).or_insert((0, 0));
                        entry.0 += 1;
                        entry.1 += file.token_count;
                    }
                }
                let languages: Vec<Value> = lang_counts
                    .iter()
                    .map(|(lang, (fc, tc))| json!({"language": lang, "files": fc, "tokens": tc}))
                    .collect();

                mcp_tool_result(
                    id,
                    &serde_json::to_string_pretty(&json!({
                        "files": total_files,
                        "tokens": total_tokens,
                        "languages": languages,
                        "focus": focus,
                    }))
                    .unwrap_or_default(),
                )
            } else {
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
        }
        "cxpak_overview" => {
            let focus = args.get("focus").and_then(|f| f.as_str());

            if focus.is_some() {
                let mut lang_counts: HashMap<String, (usize, usize)> = HashMap::new();
                let mut total_files = 0usize;
                let mut total_tokens = 0usize;
                for file in &index.files {
                    if !matches_focus(&file.relative_path, focus) {
                        continue;
                    }
                    total_files += 1;
                    total_tokens += file.token_count;
                    if let Some(ref lang) = file.language {
                        let entry = lang_counts.entry(lang.clone()).or_insert((0, 0));
                        entry.0 += 1;
                        entry.1 += file.token_count;
                    }
                }
                let languages: Vec<Value> = lang_counts
                    .iter()
                    .map(|(lang, (fc, tc))| json!({"language": lang, "files": fc, "tokens": tc}))
                    .collect();

                mcp_tool_result(
                    id,
                    &serde_json::to_string_pretty(&json!({
                        "total_files": total_files,
                        "total_tokens": total_tokens,
                        "languages": languages,
                        "focus": focus,
                    }))
                    .unwrap_or_default(),
                )
            } else {
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
        }
        "cxpak_trace" => {
            let target = args.get("target").and_then(|t| t.as_str()).unwrap_or("");
            if target.is_empty() {
                return mcp_tool_result(id, "Error: 'target' argument is required");
            }
            let focus = args.get("focus").and_then(|f| f.as_str());

            let symbol_matches = index.find_symbol(target);
            let content_matches = if symbol_matches.is_empty() {
                index.find_content_matches(target)
            } else {
                vec![]
            };

            let found = !symbol_matches.is_empty() || !content_matches.is_empty();

            let mut result = json!({
                "target": target,
                "found": found,
                "symbol_matches": symbol_matches.len(),
                "content_matches": content_matches.len(),
                "total_files": index.total_files,
            });
            if let Some(f) = focus {
                result["focus"] = json!(f);
            }

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "cxpak_diff" => {
            let git_ref = args.get("git_ref").and_then(|r| r.as_str());
            let focus = args.get("focus").and_then(|f| f.as_str());
            let _token_budget = args
                .get("tokens")
                .and_then(|t| t.as_str())
                .and_then(|t| crate::cli::parse_token_count(t).ok())
                .unwrap_or(50_000);

            match crate::commands::diff::extract_changes(repo_path, git_ref) {
                Ok(changes) => {
                    let filtered: Vec<&crate::commands::diff::FileChange> = changes
                        .iter()
                        .filter(|c| matches_focus(&c.path, focus))
                        .collect();

                    let files: Vec<Value> = filtered
                        .iter()
                        .map(|c| {
                            json!({
                                "path": c.path,
                                "diff": c.diff_text,
                            })
                        })
                        .collect();

                    let mut result = json!({
                        "git_ref": git_ref.unwrap_or("working tree"),
                        "changed_files": filtered.len(),
                        "files": files,
                    });
                    if let Some(f) = focus {
                        result["focus"] = json!(f);
                    }

                    mcp_tool_result(
                        id,
                        &serde_json::to_string_pretty(&result).unwrap_or_default(),
                    )
                }
                Err(e) => mcp_tool_result(id, &format!("Error: {e}")),
            }
        }
        "cxpak_context_for_task" => {
            let task = args.get("task").and_then(|t| t.as_str()).unwrap_or("");
            if task.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'task' argument is required and must not be empty",
                );
            }
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(15) as usize;
            let focus = args.get("focus").and_then(|f| f.as_str());

            let scorer = crate::relevance::MultiSignalScorer::new();
            let all_scored = scorer.score_all(task, index);
            let graph = crate::index::graph::build_dependency_graph(index);
            let seeds = crate::relevance::seed::select_seeds_with_graph(
                &all_scored,
                index,
                crate::relevance::seed::SEED_THRESHOLD,
                limit,
                Some(&graph),
            );
            let candidates: Vec<Value> = seeds
                .iter()
                .filter(|s| matches_focus(&s.path, focus))
                .map(|s| {
                    let deps: Vec<&str> = graph
                        .dependencies(&s.path)
                        .map(|d| d.iter().map(String::as_str).collect())
                        .unwrap_or_default();
                    let signals: Vec<Value> = s
                        .signals
                        .iter()
                        .map(|sig| {
                            json!({"name": sig.name, "score": sig.score, "detail": &sig.detail})
                        })
                        .collect();
                    json!({
                        "path": &s.path,
                        "score": (s.score * 10000.0).round() / 10000.0,
                        "signals": signals,
                        "tokens": s.token_count,
                        "dependencies": deps,
                    })
                })
                .collect();

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "task": task,
                    "candidates": candidates,
                    "total_files_scored": all_scored.len(),
                    "hint": "Review candidates and call cxpak_pack_context with selected paths, or use these as-is."
                }))
                .unwrap_or_default(),
            )
        }
        "cxpak_pack_context" => {
            let files: Vec<String> = args
                .get("files")
                .and_then(|f| f.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            if files.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'files' argument is required and must not be empty",
                );
            }

            let token_budget = args
                .get("tokens")
                .and_then(|t| t.as_str())
                .and_then(|t| crate::cli::parse_token_count(t).ok())
                .unwrap_or(50_000);
            let include_deps = args
                .get("include_dependencies")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            let focus = args.get("focus").and_then(|f| f.as_str());

            // Build a lookup map from path -> index position for O(1) access
            let index_map: HashMap<&str, usize> = index
                .files
                .iter()
                .enumerate()
                .map(|(i, f)| (f.relative_path.as_str(), i))
                .collect();

            let mut target_files: Vec<(String, &str)> = vec![];
            let mut seen: HashSet<String> = HashSet::new();
            let graph = if include_deps {
                Some(crate::index::graph::build_dependency_graph(index))
            } else {
                None
            };

            for path in &files {
                if !matches_focus(path, focus) {
                    continue;
                }
                if seen.insert(path.clone()) {
                    target_files.push((path.clone(), "selected"));
                }
                if let Some(ref g) = graph {
                    if let Some(deps) = g.dependencies(path) {
                        for dep in deps {
                            if seen.insert(dep.clone()) {
                                target_files.push((dep.clone(), "dependency"));
                            }
                        }
                    }
                }
            }

            let mut packed = vec![];
            let mut omitted = vec![];
            let mut not_found = vec![];
            let mut total_tokens = 0usize;

            for (path, included_as) in &target_files {
                match index_map.get(path.as_str()) {
                    Some(&idx) => {
                        let file = &index.files[idx];
                        if total_tokens + file.token_count <= token_budget {
                            packed.push(json!({
                                "path": path,
                                "tokens": file.token_count,
                                "content": file.content,
                                "included_as": included_as,
                            }));
                            total_tokens += file.token_count;
                        } else {
                            omitted.push(json!({
                                "path": path,
                                "tokens": file.token_count,
                                "reason": "budget exceeded",
                            }));
                        }
                    }
                    None => {
                        not_found.push(json!({ "path": path }));
                    }
                }
            }

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "packed_files": packed.len(),
                    "total_tokens": total_tokens,
                    "budget": token_budget,
                    "files": packed,
                    "omitted": omitted,
                    "not_found": not_found,
                }))
                .unwrap_or_default(),
            )
        }
        "cxpak_search" => {
            let pattern = args.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
            if pattern.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'pattern' argument is required and must not be empty",
                );
            }
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
            let focus = args.get("focus").and_then(|f| f.as_str());
            let context_lines = args
                .get("context_lines")
                .and_then(|c| c.as_u64())
                .unwrap_or(2) as usize;

            let re = match regex::Regex::new(pattern) {
                Ok(r) => r,
                Err(e) => return mcp_tool_result(id, &format!("Error: invalid regex: {e}")),
            };

            let mut matches_vec = vec![];
            let mut total_matches = 0usize;
            let mut files_searched = 0usize;

            for file in &index.files {
                if !matches_focus(&file.relative_path, focus) {
                    continue;
                }
                if file.content.is_empty() {
                    continue;
                }
                files_searched += 1;

                let lines: Vec<&str> = file.content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        total_matches += 1;
                        if matches_vec.len() < limit {
                            let start = i.saturating_sub(context_lines);
                            let end = (i + context_lines + 1).min(lines.len());
                            let ctx_before: Vec<&str> = lines[start..i].to_vec();
                            let ctx_after: Vec<&str> = lines[(i + 1)..end].to_vec();
                            matches_vec.push(json!({
                                "path": &file.relative_path,
                                "line": i + 1,
                                "content": line,
                                "context_before": ctx_before,
                                "context_after": ctx_after,
                            }));
                        }
                    }
                }
            }

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "pattern": pattern,
                    "matches": matches_vec,
                    "total_matches": total_matches,
                    "files_searched": files_searched,
                    "truncated": total_matches > limit,
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
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_stats",
            &json!({}),
            &index,
            Path::new("/tmp"),
        );
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
        let resp = handle_tool_call(
            Some(json!(2)),
            "cxpak_overview",
            &json!({}),
            &index,
            Path::new("/tmp"),
        );
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
            Path::new("/tmp"),
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
            Path::new("/tmp"),
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
            Path::new("/tmp"),
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
            Path::new("/tmp"),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("required"));
    }

    #[test]
    fn test_handle_tool_call_trace_missing_target_arg() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(7)),
            "cxpak_trace",
            &json!({}),
            &index,
            Path::new("/tmp"),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("required"));
    }

    #[test]
    fn test_handle_tool_call_unknown_tool() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(8)),
            "unknown_tool",
            &json!({}),
            &index,
            Path::new("/tmp"),
        );
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
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
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
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 7);
    }

    #[test]
    fn test_mcp_stdio_loop_tool_call() {
        let index = make_test_index();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_stats","arguments":{}}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
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
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
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
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_empty_lines_skipped() {
        let index = make_test_index();
        let input = "\n\n\n".to_string();
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_invalid_json_skipped() {
        let index = make_test_index();
        let input = "not json\n".to_string();
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
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
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, cursor, &mut output).unwrap();
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
        let repo_path = Arc::new(std::path::PathBuf::from("/tmp"));
        let _router = build_router(shared, repo_path);
        // Router created without panic = success
    }

    // --- Axum integration (in-process HTTP) ---

    #[test]
    fn test_axum_health_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
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

    // --- cxpak_context_for_task MCP tool ---

    #[test]
    fn test_mcp_tools_list_includes_new_tools() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            7,
            "should have 7 tools (4 existing + 2 v0.9 + 1 search)"
        );
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(tool_names.contains(&"cxpak_context_for_task"));
        assert!(tool_names.contains(&"cxpak_pack_context"));
    }

    #[test]
    fn test_mcp_context_for_task_happy_path() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"main function","limit":5}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert_eq!(result["task"], "main function");
        assert!(!result["candidates"].as_array().unwrap().is_empty());
        assert!(result["total_files_scored"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_mcp_context_for_task_empty_query() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":""}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("Error") || content.contains("error"));
    }

    #[test]
    fn test_mcp_context_for_task_default_limit() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"hello"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert!(result["candidates"].as_array().unwrap().len() <= 15); // default limit
    }

    // --- cxpak_pack_context MCP tool ---

    #[test]
    fn test_mcp_pack_context_happy_path() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs","src/lib.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert!(result["packed_files"].as_u64().unwrap() > 0);
        assert!(result["total_tokens"].as_u64().unwrap() > 0);
        let files = result["files"].as_array().unwrap();
        assert!(files.iter().any(|f| f["path"] == "src/main.rs"));
    }

    #[test]
    fn test_mcp_pack_context_with_dependencies() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs"],"tokens":"50k","include_dependencies":true}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert!(result["packed_files"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_mcp_pack_context_budget_overflow() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs","src/lib.rs"],"tokens":"1"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert!(
            result["omitted"].as_array().is_some_and(|a| !a.is_empty())
                || result["packed_files"].as_u64().unwrap() == 0
        );
    }

    #[test]
    fn test_mcp_pack_context_missing_files() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["nonexistent.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert_eq!(result["packed_files"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_mcp_pack_context_empty_files_list() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":[],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("Error") || content.contains("error"));
    }

    #[test]
    fn test_mcp_pack_context_invalid_token_budget_defaults() {
        // Invalid token string "xyz" should fall back to 50k default
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs"],"tokens":"xyz"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        // Should succeed (not error) with the default 50k budget
        assert_eq!(result["budget"].as_u64().unwrap(), 50_000);
        assert!(result["packed_files"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_mcp_pack_context_duplicate_files_deduped() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        // Same file listed twice — should only appear once in output
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs","src/main.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert_eq!(
            result["packed_files"].as_u64().unwrap(),
            1,
            "duplicate file should be deduped to 1"
        );
    }

    // --- Two-phase handshake integration test ---

    #[test]
    fn test_mcp_two_phase_handshake() {
        // Simulates: context_for_task → review candidates → pack_context
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");

        // Phase 1: Get candidates
        let request1 = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"main function"}}}"#;
        let input1 = format!("{request1}\n");
        let mut output1 = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input1.as_bytes(), &mut output1).unwrap();
        let response1: Value = serde_json::from_slice(&output1).unwrap();
        let content1 = response1["result"]["content"][0]["text"].as_str().unwrap();
        let result1: Value = serde_json::from_str(content1).unwrap();

        // Extract candidate paths (simulating Claude reviewing and selecting)
        let candidates = result1["candidates"].as_array().unwrap();
        assert!(!candidates.is_empty(), "should have candidates");
        let selected_paths: Vec<String> = candidates
            .iter()
            .take(2)
            .map(|c| c["path"].as_str().unwrap().to_string())
            .collect();

        // Phase 2: Pack selected files
        let request2 = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"cxpak_pack_context","arguments":{{"files":{},"tokens":"50k","include_dependencies":true}}}}}}"#,
            serde_json::to_string(&selected_paths).unwrap()
        );
        let input2 = format!("{request2}\n");
        let mut output2 = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input2.as_bytes(), &mut output2).unwrap();
        let response2: Value = serde_json::from_slice(&output2).unwrap();
        let content2 = response2["result"]["content"][0]["text"].as_str().unwrap();
        let result2: Value = serde_json::from_str(content2).unwrap();

        assert!(result2["packed_files"].as_u64().unwrap() > 0);
        let packed_files = result2["files"].as_array().unwrap();
        // All selected files should be in the pack
        for path in &selected_paths {
            assert!(
                packed_files
                    .iter()
                    .any(|f| f["path"].as_str().unwrap() == path),
                "selected file {} should be in pack",
                path
            );
        }
        // Content should be present
        for file in packed_files {
            assert!(
                !file["content"].as_str().unwrap().is_empty(),
                "packed file should have content"
            );
        }
    }

    // --- cxpak_search MCP tool ---

    #[test]
    fn test_mcp_search_happy_path() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":"fn main"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert_eq!(result["pattern"], "fn main");
        assert!(result["total_matches"].as_u64().unwrap() > 0);
        assert!(result["files_searched"].as_u64().unwrap() > 0);
        let matches = result["matches"].as_array().unwrap();
        assert!(!matches.is_empty());
        assert!(matches[0]["path"].as_str().is_some());
        assert!(matches[0]["line"].as_u64().unwrap() > 0);
        assert!(matches[0]["content"].as_str().unwrap().contains("fn main"));
    }

    #[test]
    fn test_mcp_search_no_matches() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":"zzz_nonexistent_pattern_zzz"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert_eq!(result["total_matches"].as_u64().unwrap(), 0);
        assert!(result["matches"].as_array().unwrap().is_empty());
        assert_eq!(result["truncated"], false);
    }

    #[test]
    fn test_mcp_search_invalid_regex() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":"[invalid"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("invalid regex"));
    }

    #[test]
    fn test_mcp_search_with_focus() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        // Search with focus on src/main.rs path prefix — should only find matches there
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":"fn","focus":"src/main"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        // All matches should be in files starting with "src/main"
        let matches = result["matches"].as_array().unwrap();
        for m in matches {
            assert!(
                m["path"].as_str().unwrap().starts_with("src/main"),
                "match path should start with focus prefix"
            );
        }
        assert_eq!(result["files_searched"].as_u64().unwrap(), 1);
    }

    #[test]
    fn test_mcp_search_with_limit() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        // "fn" appears in both files; limit to 1
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":"fn","limit":1}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "should respect limit of 1");
        // total_matches may be > 1 since both files have "fn"
        assert!(result["total_matches"].as_u64().unwrap() >= 1);
        assert_eq!(result["truncated"], true);
    }

    #[test]
    fn test_mcp_search_empty_pattern() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_search","arguments":{"pattern":""}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("Error") || content.contains("error"),
            "empty pattern should return error"
        );
    }

    // --- focus on existing tools ---

    #[test]
    fn test_mcp_overview_with_focus() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_overview",
            &json!({"focus": "src/main"}),
            &index,
            Path::new("/tmp"),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        // Focus on "src/main" should only include src/main.rs (1 file)
        assert_eq!(parsed["total_files"], 1);
        assert_eq!(parsed["focus"], "src/main");
        let langs = parsed["languages"].as_array().unwrap();
        assert_eq!(langs.len(), 1);
    }

    #[test]
    fn test_mcp_stats_with_focus() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_stats",
            &json!({"focus": "src/lib"}),
            &index,
            Path::new("/tmp"),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        // Focus on "src/lib" should only include src/lib.rs (1 file)
        assert_eq!(parsed["files"], 1);
        assert_eq!(parsed["focus"], "src/lib");
    }

    #[test]
    fn test_mcp_stats_with_focus_no_match() {
        let index = make_test_index();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_stats",
            &json!({"focus": "nonexistent/"}),
            &index,
            Path::new("/tmp"),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["files"], 0);
        assert_eq!(parsed["tokens"], 0);
    }

    #[test]
    fn test_mcp_tools_list_includes_search() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(
            tool_names.contains(&"cxpak_search"),
            "tools/list should include cxpak_search"
        );
        // Verify all tools have focus property
        for tool in tools {
            let props = tool["inputSchema"]["properties"].as_object().unwrap();
            assert!(
                props.contains_key("focus"),
                "tool {} should have focus property",
                tool["name"]
            );
        }
    }

    #[test]
    fn test_matches_focus_utility() {
        assert!(matches_focus("src/main.rs", None));
        assert!(matches_focus("src/main.rs", Some("src/")));
        assert!(matches_focus("src/main.rs", Some("src/main")));
        assert!(!matches_focus("src/main.rs", Some("tests/")));
        assert!(!matches_focus("lib/foo.rs", Some("src/")));
        assert!(matches_focus("", Some("")));
        assert!(matches_focus("anything", Some("")));
    }
}
