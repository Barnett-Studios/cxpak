use crate::budget::counter::TokenCounter;
use crate::commands::watch::{apply_incremental_update, classify_changes};
use crate::context_quality::annotation::{annotate_file, AnnotationContext};
use crate::context_quality::degradation::{allocate_with_degradation, FileRole};
use crate::context_quality::expansion::{expand_query, Domain};
use crate::daemon::watcher::FileWatcher;
use crate::index::CodebaseIndex;
use crate::intelligence::api_surface::extract_api_surface;
use crate::parser::LanguageRegistry;
use crate::scanner::Scanner;
use crate::schema::EdgeType;
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
type SharedSnapshot = Arc<RwLock<Option<crate::auto_context::diff::ContextSnapshot>>>;

fn matches_focus(path: &str, focus: Option<&str>) -> bool {
    match focus {
        Some(f) => path.starts_with(f),
        None => true,
    }
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

    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
    index.conventions = crate::conventions::build_convention_profile(&index, path);
    // Propagate co-change data from git_health into the top-level index field
    index.co_changes = index.conventions.git_health.co_changes.clone();
    Ok(index)
}

type SharedPath = Arc<std::path::PathBuf>;

#[derive(Clone)]
struct AppState {
    index: SharedIndex,
    repo_path: SharedPath,
    snapshot: SharedSnapshot,
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

impl axum::extract::FromRef<AppState> for SharedSnapshot {
    fn from_ref(state: &AppState) -> Self {
        state.snapshot.clone()
    }
}

/// Build the axum Router for the HTTP server.
fn build_router(shared: SharedIndex, repo_path: SharedPath) -> Router {
    let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
    let state = AppState {
        index: shared,
        repo_path,
        snapshot,
    };
    Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/overview", get(overview_handler))
        .route("/trace", get(trace_handler))
        .route("/diff", get(diff_handler))
        .route("/search", axum::routing::post(search_handler))
        .route("/blast_radius", axum::routing::post(blast_radius_handler))
        .route("/api_surface", get(api_surface_handler))
        .route("/auto_context", axum::routing::post(auto_context_handler))
        .route("/context_diff", get(context_diff_handler))
        .route("/health_score", get(health_score_handler))
        .route("/risks", get(risks_handler))
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

#[derive(Deserialize)]
struct BlastRadiusParams {
    files: Vec<String>,
    depth: Option<usize>,
    focus: Option<String>,
}

async fn blast_radius_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<BlastRadiusParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if params.files.is_empty() {
        return Ok(Json(
            json!({"error": "files is required and must not be empty"}),
        ));
    }

    let files: Vec<&str> = params.files.iter().map(|s| s.as_str()).collect();
    let depth = params.depth.unwrap_or(3);
    let focus = params.focus.as_deref();

    let result = crate::intelligence::blast_radius::compute_blast_radius(
        &files,
        &idx.graph,
        &idx.pagerank,
        &idx.test_map,
        depth,
        focus,
    );

    Ok(Json(serde_json::to_value(&result).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}

#[derive(Deserialize)]
struct ApiSurfaceParams {
    focus: Option<String>,
    include: Option<String>,
    tokens: Option<String>,
}

async fn api_surface_handler(
    State(index): State<SharedIndex>,
    Query(params): Query<ApiSurfaceParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let focus = params.focus.as_deref();
    let include = params.include.as_deref().unwrap_or("all");
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(20_000);

    let surface = extract_api_surface(&idx, focus, include, token_budget);
    Ok(Json(serde_json::to_value(&surface).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}

// --- HTTP handlers for auto_context and context_diff ---

#[derive(Deserialize)]
struct AutoContextParams {
    task: String,
    tokens: Option<String>,
    focus: Option<String>,
    include_tests: Option<bool>,
    include_blast_radius: Option<bool>,
    mode: Option<String>,
}

async fn auto_context_handler(
    State(index): State<SharedIndex>,
    State(snapshot): State<SharedSnapshot>,
    Json(params): Json<AutoContextParams>,
) -> Result<Json<Value>, StatusCode> {
    if params.task.is_empty() {
        return Ok(Json(
            json!({"error": "task is required and must not be empty"}),
        ));
    }
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);
    let opts = crate::auto_context::AutoContextOpts {
        tokens: token_budget,
        focus: params.focus,
        include_tests: params.include_tests.unwrap_or(true),
        include_blast_radius: params.include_blast_radius.unwrap_or(true),
        mode: params.mode.unwrap_or_else(|| "full".to_string()),
    };
    let result = crate::auto_context::auto_context(&params.task, &idx, &opts);
    // Store snapshot for subsequent context_diff calls.
    if let Ok(mut snap) = snapshot.write() {
        *snap = Some(crate::auto_context::diff::create_snapshot(&idx));
    }
    Ok(Json(serde_json::to_value(&result).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}

#[derive(Deserialize)]
struct ContextDiffParams {
    #[allow(dead_code)]
    since: Option<String>,
}

async fn context_diff_handler(
    State(index): State<SharedIndex>,
    State(snapshot): State<SharedSnapshot>,
    Query(_params): Query<ContextDiffParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let snap_guard = snapshot
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let delta = match snap_guard.as_ref() {
        None => crate::auto_context::diff::no_snapshot_recommendation(),
        Some(snap) => crate::auto_context::diff::compute_diff(snap, &idx),
    };
    Ok(Json(serde_json::to_value(&delta).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}

#[derive(Deserialize)]
struct RisksParams {
    limit: Option<usize>,
}

async fn health_score_handler(State(index): State<SharedIndex>) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let health = crate::intelligence::health::compute_health(&idx);
    Ok(Json(serde_json::to_value(&health).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}

async fn risks_handler(
    State(index): State<SharedIndex>,
    Query(params): Query<RisksParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let limit = params.limit.unwrap_or(10);
    let all_risks = crate::intelligence::risk::compute_risk_ranking(&idx);
    let risks: Vec<crate::intelligence::risk::RiskEntry> =
        all_risks.into_iter().take(limit).collect();
    Ok(Json(serde_json::to_value(&risks).unwrap_or_else(
        |_| json!({"error": "serialisation failed"}),
    )))
}

// --- MCP server mode (JSON-RPC over stdio) ---

pub fn run_mcp(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let index = match build_index(path) {
        Ok(idx) => idx,
        Err(e) => {
            eprintln!("cxpak: warning: could not index {}: {e}", path.display());
            eprintln!("cxpak: starting MCP server with empty index");
            CodebaseIndex::empty()
        }
    };

    eprintln!(
        "cxpak: MCP server ready ({} files indexed, {} tokens)",
        index.total_files, index.total_tokens
    );

    let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
    mcp_stdio_loop(path, &index, &snapshot)
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
    snapshot: &SharedSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    mcp_stdio_loop_with_io(repo_path, index, snapshot, stdin.lock(), &mut stdout.lock())
}

fn mcp_stdio_loop_with_io(
    repo_path: &Path,
    index: &CodebaseIndex,
    snapshot: &SharedSnapshot,
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
                            "name": "cxpak_auto_context",
                            "description": "One-call optimal context for any task. Automatically selects, ranks, filters, packs, and annotates the best context from the entire codebase. Start here — use other tools only if you need finer control.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "task": { "type": "string", "description": "Natural language task description" },
                                    "tokens": { "type": "string", "description": "Token budget (default '50k')", "default": "50k" },
                                    "focus": { "type": "string", "description": "Path prefix to scope" },
                                    "include_tests": { "type": "boolean", "description": "Include mapped test files (default true)", "default": true },
                                    "include_blast_radius": { "type": "boolean", "description": "Include blast radius analysis (default true)", "default": true }
                                },
                                "required": ["task"]
                            }
                        },
                        {
                            "name": "cxpak_context_diff",
                            "description": "Show what changed since the last auto_context call. Lightweight delta for long sessions.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "since": { "type": "string", "description": "What to diff against: 'last_call' (default) or a git ref", "default": "last_call" },
                                    "focus": { "type": "string", "description": "Path prefix to scope results (e.g. 'src/', 'tests/')" }
                                }
                            }
                        },
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
                                    "include_tests": { "type": "boolean", "description": "Auto-include test files for packed source files (default true)", "default": true },
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
                        },
                        {
                            "name": "cxpak_blast_radius",
                            "description": "Analyze the impact of changing specified files. Returns affected files categorized by impact type with risk scores.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "files": { "type": "array", "items": { "type": "string" }, "description": "File paths that are changing" },
                                    "depth": { "type": "number", "description": "Max dependency hops to follow (default 3)", "default": 3 },
                                    "focus": { "type": "string", "description": "Path prefix to scope results" }
                                },
                                "required": ["files"]
                            }
                        },
                        {
                            "name": "cxpak_api_surface",
                            "description": "Extract the public API surface: public symbols with signatures, HTTP routes, gRPC services, GraphQL types.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "focus": { "type": "string", "description": "Path prefix to scope" },
                                    "include": { "type": "string", "description": "What to include: 'all', 'symbols', 'routes' (default 'all')", "default": "all" },
                                    "tokens": { "type": "string", "description": "Token budget (default '20k')", "default": "20k" }
                                }
                            }
                        },
                        {
                            "name": "cxpak_verify",
                            "description": "Verify code changes against the codebase's observed conventions. Reports deviations with evidence, severity, and suggested fixes. Only flags violations in changed lines, not pre-existing debt.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "ref": { "type": "string", "description": "Git ref to diff against (default: auto-detect uncommitted changes vs HEAD)" },
                                    "focus": { "type": "string", "description": "Path prefix to scope verification" }
                                }
                            }
                        },
                        {
                            "name": "cxpak_conventions",
                            "description": "Return the full convention profile for the codebase. Shows all detected patterns with counts, percentages, strength labels, and exceptions.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "category": { "type": "string", "description": "Filter: 'naming', 'imports', 'errors', 'dependencies', 'testing', 'visibility', 'functions', 'git_health', or 'all' (default 'all')", "default": "all" },
                                    "strength": { "type": "string", "description": "Minimum strength: 'convention', 'trend', 'mixed', or 'all' (default 'all')", "default": "all" },
                                    "focus": { "type": "string", "description": "Path prefix — recompute stats scoped to this directory" }
                                }
                            }
                        },
                        {
                            "name": "cxpak_health",
                            "description": "Returns the codebase health score — a composite metric across 6 dimensions: convention adherence, test coverage, churn stability, module coupling, circular dependencies, and dead code (null until v1.3.0). Use this to understand the overall quality state before making structural changes.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "focus": {
                                        "type": "string",
                                        "description": "Optional path prefix to scope the analysis (e.g. 'src/api/')"
                                    }
                                }
                            }
                        },
                        {
                            "name": "cxpak_risks",
                            "description": "Returns the top risky files ranked by a composite of churn rate, blast radius, and test coverage gap. Use this to identify where to focus refactoring or additional testing.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "limit": {
                                        "type": "number",
                                        "description": "Maximum number of risk entries to return (default 10)",
                                        "default": 10
                                    },
                                    "focus": {
                                        "type": "string",
                                        "description": "Optional path prefix to scope the analysis (e.g. 'src/api/')"
                                    }
                                }
                            }
                        },
                        {
                            "name": "cxpak_briefing",
                            "description": "Returns a compact briefing: file manifest with scores and signals, health score, top risks, and architecture map — but no file content. Ideal for orientation at the start of a task when you need structure, not code.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "task": { "type": "string", "description": "Natural language task description" },
                                    "tokens": { "type": "string", "description": "Token budget (default '50k')", "default": "50k" },
                                    "focus": { "type": "string", "description": "Path prefix to scope" }
                                },
                                "required": ["task"]
                            }
                        }
                    ]
                }),
            ),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                handle_tool_call(id, tool_name, &arguments, index, repo_path, snapshot)
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
    snapshot: &SharedSnapshot,
) -> Value {
    match tool_name {
        "cxpak_auto_context" => {
            let task = args.get("task").and_then(|t| t.as_str()).unwrap_or("");
            if task.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'task' argument is required and must not be empty",
                );
            }
            let token_budget = args
                .get("tokens")
                .and_then(|t| t.as_str())
                .and_then(|t| crate::cli::parse_token_count(t).ok())
                .unwrap_or(50_000);
            let focus = args.get("focus").and_then(|f| f.as_str()).map(String::from);
            let include_tests = args
                .get("include_tests")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let include_blast_radius = args
                .get("include_blast_radius")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let mode = args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("full")
                .to_string();
            let opts = crate::auto_context::AutoContextOpts {
                tokens: token_budget,
                focus,
                include_tests,
                include_blast_radius,
                mode,
            };
            let result = crate::auto_context::auto_context(task, index, &opts);
            // Store snapshot for subsequent context_diff calls.
            if let Ok(mut snap) = snapshot.write() {
                *snap = Some(crate::auto_context::diff::create_snapshot(index));
            }
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "cxpak_context_diff" => {
            let snap_guard = snapshot.read();
            let delta = match snap_guard {
                Ok(guard) => match guard.as_ref() {
                    None => crate::auto_context::diff::no_snapshot_recommendation(),
                    Some(snap) => crate::auto_context::diff::compute_diff(snap, index),
                },
                Err(_) => crate::auto_context::diff::no_snapshot_recommendation(),
            };
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&delta).unwrap_or_default(),
            )
        }
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

            let mut expanded_tokens = expand_query(task, &index.domains);
            // When the Database domain is active and schema is indexed, add table
            // and column names as additional expansion terms so that files referencing
            // those identifiers score higher.
            if index.domains.contains(&Domain::Database) {
                if let Some(schema) = &index.schema {
                    for (name, table) in &schema.tables {
                        expanded_tokens.insert(name.to_lowercase());
                        for col in &table.columns {
                            expanded_tokens.insert(col.name.to_lowercase());
                        }
                    }
                }
            }
            let scorer = crate::relevance::MultiSignalScorer::new().with_expansion(expanded_tokens);
            let all_scored = scorer.score_all(task, index);
            let seeds = crate::relevance::seed::select_seeds_with_graph(
                &all_scored,
                index,
                crate::relevance::seed::SEED_THRESHOLD,
                limit,
                Some(&index.graph),
            );
            let candidates: Vec<Value> = seeds
                .iter()
                .filter(|s| matches_focus(&s.path, focus))
                .map(|s| {
                    let deps: Vec<&str> = index
                        .graph
                        .dependencies(&s.path)
                        .map(|d| d.iter().map(|e| e.target.as_str()).collect())
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
            let include_tests = args
                .get("include_tests")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let focus = args.get("focus").and_then(|f| f.as_str());

            // Build a lookup map from path -> index position for O(1) access.
            let index_map: HashMap<&str, usize> = index
                .files
                .iter()
                .enumerate()
                .map(|(i, f)| (f.relative_path.as_str(), i))
                .collect();

            // Track which paths came from user selection vs. dependency expansion,
            // the file that pulled each dependency in, and the edge type used.
            let mut target_files: Vec<(String, FileRole, Option<String>, Option<EdgeType>)> =
                vec![];
            let mut seen: HashSet<String> = HashSet::new();
            let graph = if include_deps {
                Some(&index.graph)
            } else {
                None
            };

            for path in &files {
                if !matches_focus(path, focus) {
                    continue;
                }
                if seen.insert(path.clone()) {
                    target_files.push((path.clone(), FileRole::Selected, None, None));
                }
                if let Some(g) = &graph {
                    if let Some(deps) = g.dependencies(path) {
                        for dep in deps {
                            if seen.insert(dep.target.clone()) {
                                target_files.push((
                                    dep.target.clone(),
                                    FileRole::Dependency,
                                    Some(path.clone()),
                                    Some(dep.edge_type.clone()),
                                ));
                            }
                        }
                    }
                }
            }

            // Auto-include test files for selected source files.
            if include_tests {
                let test_additions: Vec<(String, FileRole, Option<String>, Option<EdgeType>)> =
                    target_files
                        .iter()
                        .filter(|(_, role, _, _)| matches!(role, FileRole::Selected))
                        .filter_map(|(path, _, _, _)| {
                            index.test_map.get(path).map(|tests| {
                                tests
                                    .iter()
                                    .filter_map(|t| {
                                        if seen.insert(t.path.clone()) {
                                            Some((
                                                t.path.clone(),
                                                FileRole::Dependency,
                                                Some(path.clone()),
                                                None,
                                            ))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .flatten()
                        .collect();
                target_files.extend(test_additions);
            }

            // Separate found vs. not-found.
            type PackTarget<'a> = (
                &'a crate::index::IndexedFile,
                FileRole,
                f64,
                Option<String>,
                Option<EdgeType>,
            );
            let mut not_found: Vec<Value> = vec![];
            let mut indexed_targets: Vec<PackTarget<'_>> = vec![];

            for (path, role, parent, edge_type) in &target_files {
                match index_map.get(path.as_str()) {
                    Some(&idx) => {
                        // Selected files get a high relevance score; dependencies lower.
                        let score = match role {
                            FileRole::Selected => 1.0,
                            FileRole::Dependency => 0.5,
                        };
                        indexed_targets.push((
                            &index.files[idx],
                            *role,
                            score,
                            parent.clone(),
                            edge_type.clone(),
                        ));
                    }
                    None => {
                        not_found.push(json!({ "path": path }));
                    }
                }
            }

            // Allocate budget with progressive degradation.
            let alloc_inputs: Vec<(&crate::index::IndexedFile, FileRole, f64)> = indexed_targets
                .iter()
                .map(|(f, role, score, _, _)| (*f, *role, *score))
                .collect();
            let allocated =
                allocate_with_degradation(&alloc_inputs, token_budget, Some(&index.pagerank));

            // Render annotated output per file.
            let mut packed: Vec<Value> = vec![];
            let mut total_tokens = 0usize;

            for (alloc, (indexed_file, role, _score, parent, edge_type)) in
                allocated.iter().zip(indexed_targets.iter())
            {
                let rendered_tokens: usize = alloc.symbols.iter().map(|s| s.rendered_tokens).sum();
                // For files with no parsed symbols (binary, unrecognised language, etc.)
                // fall back to raw content token count so the annotation is still accurate.
                let effective_tokens = if rendered_tokens > 0 {
                    rendered_tokens
                } else {
                    indexed_file.token_count
                };

                // Build the annotation parent string: for non-Import edges, append the edge type.
                let annotation_parent = parent.as_ref().map(|p| match edge_type {
                    Some(et) if *et != EdgeType::Import => {
                        let label = match et {
                            EdgeType::ForeignKey => "foreign_key",
                            EdgeType::ViewReference => "view_reference",
                            EdgeType::TriggerTarget => "trigger_target",
                            EdgeType::IndexTarget => "index_target",
                            EdgeType::FunctionReference => "function_reference",
                            EdgeType::EmbeddedSql => "embedded_sql",
                            EdgeType::OrmModel => "orm_model",
                            EdgeType::MigrationSequence => "migration_sequence",
                            EdgeType::Import => unreachable!(),
                        };
                        format!("{p} (via: {label})")
                    }
                    _ => p.clone(),
                });

                let annotation_ctx = AnnotationContext {
                    path: indexed_file.relative_path.clone(),
                    language: indexed_file.language.clone().unwrap_or_default(),
                    score: match role {
                        FileRole::Selected => 1.0,
                        FileRole::Dependency => 0.5,
                    },
                    role: *role,
                    parent: annotation_parent,
                    signals: vec![],
                    detail_level: alloc.level,
                    tokens: effective_tokens,
                };
                let annotation = annotate_file(&annotation_ctx);

                // Build the content: annotation header + rendered symbols (if any),
                // otherwise annotation header + raw file content.
                let content = if alloc.symbols.is_empty() {
                    format!("{annotation}\n{}", indexed_file.content)
                } else {
                    let body: String = alloc
                        .symbols
                        .iter()
                        .map(|s| s.rendered.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    format!("{annotation}\n{body}")
                };

                let detail_level_str = match alloc.level {
                    crate::context_quality::degradation::DetailLevel::Full => "full",
                    crate::context_quality::degradation::DetailLevel::Trimmed => "trimmed",
                    crate::context_quality::degradation::DetailLevel::Documented => "documented",
                    crate::context_quality::degradation::DetailLevel::Signature => "signature",
                    crate::context_quality::degradation::DetailLevel::Stub => "stub",
                };

                let included_as = match role {
                    FileRole::Selected => "selected",
                    FileRole::Dependency => "dependency",
                };

                total_tokens += effective_tokens;
                packed.push(json!({
                    "path": &indexed_file.relative_path,
                    "tokens": effective_tokens,
                    "detail_level": detail_level_str,
                    "included_as": included_as,
                    "content": content,
                }));
            }

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "packed_files": packed.len(),
                    "total_tokens": total_tokens,
                    "budget": token_budget,
                    "files": packed,
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
        "cxpak_blast_radius" => {
            let files: Vec<&str> = args
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if files.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'files' argument is required and must not be empty",
                );
            }
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let focus = args.get("focus").and_then(|v| v.as_str());
            let result = crate::intelligence::blast_radius::compute_blast_radius(
                &files,
                &index.graph,
                &index.pagerank,
                &index.test_map,
                depth,
                focus,
            );
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "cxpak_api_surface" => {
            let focus = args.get("focus").and_then(|f| f.as_str());
            let include = args
                .get("include")
                .and_then(|v| v.as_str())
                .unwrap_or("all");
            let token_budget = args
                .get("tokens")
                .and_then(|t| t.as_str())
                .and_then(|t| crate::cli::parse_token_count(t).ok())
                .unwrap_or(20_000);

            let surface = extract_api_surface(index, focus, include, token_budget);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&surface).unwrap_or_default(),
            )
        }
        "cxpak_verify" => {
            let git_ref = args.get("ref").and_then(|r| r.as_str());
            let focus = args.get("focus").and_then(|f| f.as_str());

            let changed =
                match crate::conventions::verify::get_changed_lines(repo_path, git_ref, focus) {
                    Ok(c) => c,
                    Err(e) => return mcp_tool_result(id, &format!("Error: {e}")),
                };

            if changed.is_empty() {
                return mcp_tool_result(
                    id,
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "files_checked": 0,
                        "lines_checked": 0,
                        "violations": [],
                        "passed": ["No changes detected"],
                        "summary": {"high": 0, "medium": 0, "low": 0}
                    }))
                    .unwrap_or_default(),
                );
            }

            let result = crate::conventions::verify::verify_changes(&changed, index, repo_path);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&serde_json::to_value(&result).unwrap_or_default())
                    .unwrap_or_default(),
            )
        }
        "cxpak_conventions" => {
            let category = args
                .get("category")
                .and_then(|c| c.as_str())
                .unwrap_or("all");
            let _strength = args
                .get("strength")
                .and_then(|s| s.as_str())
                .unwrap_or("all");
            let _focus = args.get("focus").and_then(|f| f.as_str());

            let profile = &index.conventions;

            let result = match category {
                "naming" => serde_json::to_value(&profile.naming).unwrap_or_default(),
                "imports" => serde_json::to_value(&profile.imports).unwrap_or_default(),
                "errors" => serde_json::to_value(&profile.errors).unwrap_or_default(),
                "dependencies" => serde_json::to_value(&profile.dependencies).unwrap_or_default(),
                "testing" => serde_json::to_value(&profile.testing).unwrap_or_default(),
                "visibility" => serde_json::to_value(&profile.visibility).unwrap_or_default(),
                "functions" => serde_json::to_value(&profile.functions).unwrap_or_default(),
                "git_health" => serde_json::to_value(&profile.git_health).unwrap_or_default(),
                _ => serde_json::to_value(profile).unwrap_or_default(),
            };

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "cxpak_health" => {
            let health = crate::intelligence::health::compute_health(index);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&health).unwrap_or_default(),
            )
        }
        "cxpak_risks" => {
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;
            let all_risks = crate::intelligence::risk::compute_risk_ranking(index);
            let risks: Vec<&crate::intelligence::risk::RiskEntry> =
                all_risks.iter().take(limit).collect();
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&risks).unwrap_or_default(),
            )
        }
        "cxpak_briefing" => {
            let task = args.get("task").and_then(|t| t.as_str()).unwrap_or("");
            if task.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'task' argument is required and must not be empty",
                );
            }
            let token_budget = args
                .get("tokens")
                .and_then(|t| t.as_str())
                .and_then(|t| crate::cli::parse_token_count(t).ok())
                .unwrap_or(50_000);
            let focus = args.get("focus").and_then(|f| f.as_str()).map(String::from);
            let opts = crate::auto_context::AutoContextOpts {
                tokens: token_budget,
                focus,
                include_tests: true,
                include_blast_radius: true,
                mode: "briefing".to_string(),
            };
            let result = crate::auto_context::auto_context(task, index, &opts);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
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
            {
                let mod_vec: Vec<String> = modified_paths.iter().cloned().collect();
                let rem_vec: Vec<String> = removed_paths.iter().cloned().collect();
                let mut conventions = std::mem::take(&mut idx.conventions);
                crate::conventions::update_conventions_incremental(
                    &mut conventions,
                    &mod_vec,
                    &rem_vec,
                    &idx,
                );
                idx.conventions = conventions;
            }
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

    fn make_shared_snapshot() -> SharedSnapshot {
        Arc::new(RwLock::new(None))
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_stats",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(2)),
            "cxpak_overview",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["total_files"], 2);
        assert!(parsed["languages"].as_array().is_some());
    }

    #[test]
    fn test_handle_tool_call_trace_found() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(3)),
            "cxpak_trace",
            &json!({"target": "main"}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(4)),
            "cxpak_trace",
            &json!({"target": "hello"}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(5)),
            "cxpak_trace",
            &json!({"target": "nonexistent_xyz"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["found"], false);
    }

    #[test]
    fn test_handle_tool_call_trace_empty_target() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(6)),
            "cxpak_trace",
            &json!({"target": ""}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("required"));
    }

    #[test]
    fn test_handle_tool_call_trace_missing_target_arg() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(7)),
            "cxpak_trace",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("required"));
    }

    #[test]
    fn test_handle_tool_call_unknown_tool() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(8)),
            "unknown_tool",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 16);
    }

    #[test]
    fn test_mcp_stdio_loop_tool_call() {
        let index = make_test_index();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_stats","arguments":{}}}"#;
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_empty_lines_skipped() {
        let index = make_test_index();
        let input = "\n\n\n".to_string();
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_mcp_stdio_loop_invalid_json_skipped() {
        let index = make_test_index();
        let input = "not json\n".to_string();
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = text.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
        let resp1: Value = serde_json::from_str(lines[0]).unwrap();
        let resp2: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(resp1["id"], 1);
        assert_eq!(resp2["id"], 2);
    }

    // --- blast_radius MCP round-trip ---

    #[test]
    fn test_mcp_blast_radius_round_trip() {
        // Build an index with two files where main.rs imports lib.rs, so
        // blast_radius can find a real direct dependent.
        let counter = crate::budget::counter::TokenCounter::new();
        use crate::parser::language::{Import, ParseResult};

        let files = vec![
            crate::scanner::ScannedFile {
                relative_path: "src/lib.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/lib.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
            crate::scanner::ScannedFile {
                relative_path: "src/main.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/main.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/main.rs".to_string(),
            ParseResult {
                symbols: vec![],
                // main.rs imports lib.rs ("src::lib" resolves to src/lib.rs)
                imports: vec![Import {
                    source: "src::lib".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/lib.rs".to_string(),
            ParseResult {
                symbols: vec![],
                imports: vec![],
                exports: vec![],
            },
        );

        let mut content_map = HashMap::new();
        content_map.insert("src/lib.rs".to_string(), "pub fn helper() {}".to_string());
        content_map.insert("src/main.rs".to_string(), "fn main() {}".to_string());

        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

        // MCP round-trip: call cxpak_blast_radius for src/lib.rs
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","#,
            r#""params":{"name":"cxpak_blast_radius","arguments":{"files":["src/lib.rs"],"depth":3}}}"#
        );
        let input = format!("{input}\n");
        let cursor = std::io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            Path::new("/tmp"),
            &index,
            &make_shared_snapshot(),
            cursor,
            &mut output,
        )
        .unwrap();

        let line = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).unwrap();

        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 42);

        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        assert!(
            parsed["changed_files"].as_array().is_some(),
            "result must have changed_files"
        );
        assert!(
            parsed["total_affected"].is_number(),
            "result must have total_affected"
        );
        assert!(
            parsed["categories"].is_object(),
            "result must have categories"
        );
        assert!(
            parsed["risk_summary"].is_object(),
            "result must have risk_summary"
        );

        // src/main.rs imports src/lib.rs -> should appear as a direct dependent
        let direct = parsed["categories"]["direct_dependents"]
            .as_array()
            .expect("direct_dependents must be an array");
        let main_rs = direct.iter().find(|f| f["path"] == "src/main.rs");
        assert!(
            main_rs.is_some(),
            "src/main.rs should appear as a direct dependent of src/lib.rs"
        );
    }

    // --- blast_radius HTTP endpoint ---

    #[test]
    fn test_axum_blast_radius_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
            let body = serde_json::to_vec(&json!({"files": ["src/main.rs"]})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/blast_radius")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(json["changed_files"].as_array().is_some());
            assert!(json["total_affected"].is_number());
            assert!(json["categories"].is_object());
            assert!(json["risk_summary"].is_object());
        });
    }

    #[test]
    fn test_axum_blast_radius_empty_files_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
            let body = serde_json::to_vec(&json!({"files": []})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/blast_radius")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(json["error"].as_str().is_some());
        });
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            16,
            "should have 16 tools (13 existing + 1 health + 1 risks + 1 briefing)"
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert!(result["packed_files"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_mcp_pack_context_budget_overflow() {
        // With a very small budget, degradation kicks in but all files are still returned
        // (degraded to stub level rather than dropped entirely).
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs","src/lib.rs"],"tokens":"1"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        // The response should be well-formed and contain a budget field.
        assert_eq!(result["budget"].as_u64().unwrap(), 1);
        // All requested files are returned (degraded, not omitted).
        assert!(result["packed_files"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_mcp_pack_context_missing_files() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["nonexistent.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input1.as_bytes(),
            &mut output1,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input2.as_bytes(),
            &mut output2,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_overview",
            &json!({"focus": "src/main"}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_stats",
            &json!({"focus": "src/lib"}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_stats",
            &json!({"focus": "nonexistent/"}),
            &index,
            Path::new("/tmp"),
            &snap,
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
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
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

    // --- Task 15: pack_context with degradation + annotations ---

    #[test]
    fn test_pack_context_response_includes_detail_level() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let files = result["files"].as_array().unwrap();
        assert!(!files.is_empty(), "should have at least one packed file");
        // Each file entry must now include a detail_level field.
        for file in files {
            assert!(
                file["detail_level"].is_string(),
                "each file should have a detail_level field"
            );
            let level = file["detail_level"].as_str().unwrap();
            assert!(
                ["full", "trimmed", "documented", "signature", "stub"].contains(&level),
                "detail_level should be a valid level name, got: {level}"
            );
        }
    }

    #[test]
    fn test_pack_context_response_content_contains_annotation_header() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let files = result["files"].as_array().unwrap();
        let main_file = files
            .iter()
            .find(|f| f["path"] == "src/main.rs")
            .expect("src/main.rs should be in the pack");
        let file_content = main_file["content"].as_str().unwrap();
        // The annotation header must contain the [cxpak] marker.
        assert!(
            file_content.contains("[cxpak]"),
            "content should start with annotation header containing [cxpak], got:\n{file_content}"
        );
        // The annotation header should include the file path.
        assert!(
            file_content.contains("src/main.rs"),
            "annotation should include the file path"
        );
        // The annotation should include a detail_level line.
        assert!(
            file_content.contains("detail_level:"),
            "annotation should include a detail_level line"
        );
    }

    #[test]
    fn test_pack_context_selected_role_annotation() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/main.rs"],"tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let files = result["files"].as_array().unwrap();
        let main_file = files
            .iter()
            .find(|f| f["path"] == "src/main.rs")
            .expect("src/main.rs should be in the pack");
        // Selected files should be marked as "selected" in included_as.
        assert_eq!(main_file["included_as"], "selected");
        // The annotation should note the role.
        let file_content = main_file["content"].as_str().unwrap();
        assert!(
            file_content.contains("selected"),
            "annotation should mention 'selected' role"
        );
    }

    // --- Task 10: include_tests in pack_context ---

    #[test]
    fn test_pack_context_include_tests_true_auto_includes_test_files() {
        // Build an index with a source file and its corresponding test file.
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/util.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/util.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
            ScannedFile {
                relative_path: "tests/util_test.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/tests/util_test.rs"),
                language: Some("rust".to_string()),
                size_bytes: 60,
            },
        ];
        let mut content_map = HashMap::new();
        content_map.insert(
            "src/util.rs".to_string(),
            "pub fn add(a: u32, b: u32) -> u32 { a + b }".to_string(),
        );
        content_map.insert(
            "tests/util_test.rs".to_string(),
            "use crate::util; #[test] fn test_add() { assert_eq!(util::add(1,2),3); }".to_string(),
        );

        use crate::parser::language::{Import, ParseResult as PR};
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "tests/util_test.rs".to_string(),
            PR {
                symbols: vec![],
                imports: vec![Import {
                    source: "crate::util".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        let repo_path = std::path::Path::new("/tmp");

        // include_tests defaults to true, so test file should be auto-included.
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/util.rs"],"tokens":"50k","include_tests":true}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let file_paths: Vec<&str> = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(
            file_paths.contains(&"src/util.rs"),
            "selected source file must be present"
        );
        assert!(
            file_paths.contains(&"tests/util_test.rs"),
            "test file should be auto-included when include_tests=true, got paths: {file_paths:?}"
        );
        // The test file should be marked as a dependency.
        let test_entry = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "tests/util_test.rs")
            .expect("tests/util_test.rs must be in packed files");
        assert_eq!(
            test_entry["included_as"], "dependency",
            "auto-included test file should have included_as=dependency"
        );
    }

    #[test]
    fn test_pack_context_include_tests_false_does_not_include_test_files() {
        // Same setup as above but with include_tests=false.
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/util.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/util.rs"),
                language: Some("rust".to_string()),
                size_bytes: 50,
            },
            ScannedFile {
                relative_path: "tests/util_test.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/tests/util_test.rs"),
                language: Some("rust".to_string()),
                size_bytes: 60,
            },
        ];
        let mut content_map = HashMap::new();
        content_map.insert(
            "src/util.rs".to_string(),
            "pub fn add(a: u32, b: u32) -> u32 { a + b }".to_string(),
        );
        content_map.insert(
            "tests/util_test.rs".to_string(),
            "use crate::util; #[test] fn test_add() { assert_eq!(util::add(1,2),3); }".to_string(),
        );

        use crate::parser::language::{Import, ParseResult as PR};
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "tests/util_test.rs".to_string(),
            PR {
                symbols: vec![],
                imports: vec![Import {
                    source: "crate::util".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );

        let index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);
        let repo_path = std::path::Path::new("/tmp");

        // include_tests=false — test file must NOT be auto-included.
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_pack_context","arguments":{"files":["src/util.rs"],"tokens":"50k","include_tests":false}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let file_paths: Vec<&str> = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(
            file_paths.contains(&"src/util.rs"),
            "selected source file must be present"
        );
        assert!(
            !file_paths.contains(&"tests/util_test.rs"),
            "test file must NOT be included when include_tests=false, got paths: {file_paths:?}"
        );
    }

    // --- Task 16: context_for_task with query expansion ---

    #[test]
    fn test_context_for_task_uses_expansion_for_auth_terms() {
        // Build an index that contains an "auth" file so expansion works.
        let counter = TokenCounter::new();
        let files = vec![
            crate::scanner::ScannedFile {
                relative_path: "src/auth/login.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/auth/login.rs"),
                language: Some("rust".to_string()),
                size_bytes: 120,
            },
            crate::scanner::ScannedFile {
                relative_path: "src/api/handler.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/api/handler.rs"),
                language: Some("rust".to_string()),
                size_bytes: 80,
            },
        ];
        let mut content_map = std::collections::HashMap::new();
        content_map.insert(
            "src/auth/login.rs".to_string(),
            "pub fn authenticate(credential: &str) -> bool { true }".to_string(),
        );
        content_map.insert(
            "src/api/handler.rs".to_string(),
            "pub fn handle_request(req: Request) -> Response { todo!() }".to_string(),
        );
        let index = CodebaseIndex::build_with_content(
            files,
            std::collections::HashMap::new(),
            &counter,
            content_map,
        );

        let repo_path = std::path::Path::new("/tmp");
        // Query "auth" should expand to synonyms like "authentication", "login", "credential".
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"auth","limit":5}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let candidates = result["candidates"].as_array().unwrap();
        assert!(!candidates.is_empty(), "should find candidates");
        // The auth file should be ranked at or near the top.
        let top_path = candidates[0]["path"].as_str().unwrap();
        assert!(
            top_path.contains("auth"),
            "auth-related file should be top candidate when querying 'auth', got: {top_path}"
        );
    }

    #[test]
    fn test_context_for_task_expansion_synonym_boosts_score() {
        // Verify that query expansion actually influences scoring.
        // We create two files: one matching the literal query term, one matching
        // only an expanded synonym. Both should appear in candidates.
        let counter = TokenCounter::new();
        let files = vec![
            crate::scanner::ScannedFile {
                relative_path: "src/db/schema.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/db/schema.rs"),
                language: Some("rust".to_string()),
                size_bytes: 100,
            },
            crate::scanner::ScannedFile {
                relative_path: "src/api/route.rs".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/src/api/route.rs"),
                language: Some("rust".to_string()),
                size_bytes: 80,
            },
        ];
        let mut content_map = std::collections::HashMap::new();
        // This file contains "migration" which is an expansion of "db"
        content_map.insert(
            "src/db/schema.rs".to_string(),
            "// migration schema definition\npub struct User { id: u64 }".to_string(),
        );
        content_map.insert(
            "src/api/route.rs".to_string(),
            "pub fn get_users() -> Vec<User> { vec![] }".to_string(),
        );
        let index = CodebaseIndex::build_with_content(
            files,
            std::collections::HashMap::new(),
            &counter,
            content_map,
        );

        let repo_path = std::path::Path::new("/tmp");
        // Query "db" expands to: database, query, sql, migration, schema, table, model, orm, repository
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_for_task","arguments":{"task":"db","limit":10}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_bytes(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        let candidates = result["candidates"].as_array().unwrap();
        // The db/schema.rs file should rank above api/route.rs because it matches
        // "schema" and "migration" (expansion synonyms for "db").
        if candidates.len() >= 2 {
            let top_score = candidates[0]["score"].as_f64().unwrap_or(0.0);
            let db_candidate = candidates
                .iter()
                .find(|c| c["path"].as_str().unwrap_or("").contains("schema"));
            let route_candidate = candidates
                .iter()
                .find(|c| c["path"].as_str().unwrap_or("").contains("route"));
            if let (Some(db), Some(route)) = (db_candidate, route_candidate) {
                let db_score = db["score"].as_f64().unwrap_or(0.0);
                let route_score = route["score"].as_f64().unwrap_or(0.0);
                assert!(
                    db_score >= route_score,
                    "db/schema.rs (score {db_score:.4}) should score >= api/route.rs (score {route_score:.4}) when querying 'db'"
                );
            }
            let _ = top_score; // used above indirectly
        }
        assert!(!candidates.is_empty(), "should return candidates");
    }

    // --- Task 17: Pipeline integration tests ---

    /// Helper: build a minimal CodebaseIndex directly (without disk I/O) with
    /// a SchemaIndex attached, ready for use in pipeline tests.
    fn make_sql_index_with_fk() -> CodebaseIndex {
        use crate::schema::{ColumnSchema, ForeignKeyRef, SchemaIndex, TableSchema};

        let counter = TokenCounter::new();
        let files = vec![
            crate::scanner::ScannedFile {
                relative_path: "schema/users.sql".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/schema/users.sql"),
                language: Some("sql".to_string()),
                size_bytes: 100,
            },
            crate::scanner::ScannedFile {
                relative_path: "schema/orders.sql".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/schema/orders.sql"),
                language: Some("sql".to_string()),
                size_bytes: 120,
            },
        ];
        let content_map = HashMap::from([
            (
                "schema/users.sql".to_string(),
                "CREATE TABLE users (id INT PRIMARY KEY, name TEXT);".to_string(),
            ),
            (
                "schema/orders.sql".to_string(),
                "CREATE TABLE orders (id INT PRIMARY KEY, user_id INT REFERENCES users(id));"
                    .to_string(),
            ),
        ]);

        let users_table = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "INT".to_string(),
                nullable: false,
                default: None,
                constraints: vec![],
                foreign_key: None,
            }],
            primary_key: Some(vec!["id".to_string()]),
            indexes: vec![],
            file_path: "schema/users.sql".to_string(),
            start_line: 1,
        };
        let orders_table = TableSchema {
            name: "orders".to_string(),
            columns: vec![ColumnSchema {
                name: "user_id".to_string(),
                data_type: "INT".to_string(),
                nullable: true,
                default: None,
                constraints: vec![],
                foreign_key: Some(ForeignKeyRef {
                    target_table: "users".to_string(),
                    target_column: "id".to_string(),
                }),
            }],
            primary_key: Some(vec!["id".to_string()]),
            indexes: vec![],
            file_path: "schema/orders.sql".to_string(),
            start_line: 1,
        };

        let mut schema = SchemaIndex::empty();
        schema.tables.insert("users".to_string(), users_table);
        schema.tables.insert("orders".to_string(), orders_table);

        let mut index =
            CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        index.schema = Some(schema);
        index.rebuild_graph();
        index
    }

    #[test]
    fn test_pipeline_sql_repo_produces_fk_edges() {
        let index = make_sql_index_with_fk();
        let graph = &index.graph;

        // orders.sql → users.sql via ForeignKey
        let deps = graph
            .dependencies("schema/orders.sql")
            .expect("orders.sql should have deps");
        assert!(
            deps.iter().any(|e| e.target == "schema/users.sql"
                && e.edge_type == crate::schema::EdgeType::ForeignKey),
            "orders.sql should have a ForeignKey edge to users.sql, got: {:?}",
            deps
        );
    }

    #[test]
    fn test_pipeline_python_embedded_sql_produces_embedded_sql_edge() {
        use crate::schema::{ColumnSchema, SchemaIndex, TableSchema};

        let counter = TokenCounter::new();
        let files = vec![
            crate::scanner::ScannedFile {
                relative_path: "schema/products.sql".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/schema/products.sql"),
                language: Some("sql".to_string()),
                size_bytes: 60,
            },
            crate::scanner::ScannedFile {
                relative_path: "api/orders.py".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/api/orders.py"),
                language: Some("python".to_string()),
                size_bytes: 100,
            },
        ];
        let content_map = HashMap::from([
            (
                "schema/products.sql".to_string(),
                "CREATE TABLE products (id INT, name TEXT);".to_string(),
            ),
            (
                "api/orders.py".to_string(),
                "def list_products(db):\n    return db.execute('SELECT id FROM products WHERE active = 1')".to_string(),
            ),
        ]);

        let products_table = TableSchema {
            name: "products".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "INT".to_string(),
                nullable: false,
                default: None,
                constraints: vec![],
                foreign_key: None,
            }],
            primary_key: None,
            indexes: vec![],
            file_path: "schema/products.sql".to_string(),
            start_line: 1,
        };

        let mut schema = SchemaIndex::empty();
        schema.tables.insert("products".to_string(), products_table);

        let mut index =
            CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        index.schema = Some(schema);
        index.rebuild_graph();

        let graph = &index.graph;
        let deps = graph
            .dependencies("api/orders.py")
            .expect("api/orders.py should have deps");
        assert!(
            deps.iter().any(|e| e.target == "schema/products.sql"
                && e.edge_type == crate::schema::EdgeType::EmbeddedSql),
            "api/orders.py should have an EmbeddedSql edge to schema/products.sql, got: {:?}",
            deps
        );
    }

    #[test]
    fn test_pipeline_plain_rust_repo_has_no_schema() {
        let index = make_test_index();
        assert!(
            index.schema.is_none(),
            "a plain Rust repo should have schema: None"
        );
    }

    #[test]
    fn test_pipeline_orm_model_produces_orm_edge() {
        use crate::schema::{ColumnSchema, OrmFramework, OrmModelSchema, SchemaIndex, TableSchema};

        let counter = TokenCounter::new();
        let files = vec![
            crate::scanner::ScannedFile {
                relative_path: "schema/users.sql".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/schema/users.sql"),
                language: Some("sql".to_string()),
                size_bytes: 80,
            },
            crate::scanner::ScannedFile {
                relative_path: "app/models.py".to_string(),
                absolute_path: std::path::PathBuf::from("/tmp/app/models.py"),
                language: Some("python".to_string()),
                size_bytes: 150,
            },
        ];
        let content_map = HashMap::from([
            (
                "schema/users.sql".to_string(),
                "CREATE TABLE users (id INT PRIMARY KEY, name TEXT);".to_string(),
            ),
            (
                "app/models.py".to_string(),
                "class User(models.Model):\n    class Meta:\n        db_table = 'users'"
                    .to_string(),
            ),
        ]);

        let users_table = TableSchema {
            name: "users".to_string(),
            columns: vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "INT".to_string(),
                nullable: false,
                default: None,
                constraints: vec![],
                foreign_key: None,
            }],
            primary_key: Some(vec!["id".to_string()]),
            indexes: vec![],
            file_path: "schema/users.sql".to_string(),
            start_line: 1,
        };

        let mut schema = SchemaIndex::empty();
        schema.tables.insert("users".to_string(), users_table);
        schema.orm_models.insert(
            "User".to_string(),
            OrmModelSchema {
                class_name: "User".to_string(),
                table_name: "users".to_string(),
                framework: OrmFramework::Django,
                file_path: "app/models.py".to_string(),
                fields: vec![],
            },
        );

        let mut index =
            CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content_map);
        index.schema = Some(schema);
        index.rebuild_graph();

        let graph = &index.graph;
        let deps = graph
            .dependencies("app/models.py")
            .expect("app/models.py should have deps");
        assert!(
            deps.iter().any(|e| e.target == "schema/users.sql"
                && e.edge_type == crate::schema::EdgeType::OrmModel),
            "app/models.py should have an OrmModel edge to schema/users.sql, got: {:?}",
            deps
        );
    }

    // --- cxpak_auto_context MCP tool ---

    #[test]
    fn test_mcp_auto_context_happy_path() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{"task":"main function","tokens":"50k"}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert_eq!(result["task"], "main function");
        // Snapshot should have been populated after the call.
        assert!(
            snap.read().unwrap().is_some(),
            "snapshot should be populated after cxpak_auto_context call"
        );
    }

    #[test]
    fn test_mcp_auto_context_empty_task_returns_error() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{"task":""}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("Error") || content.contains("error"),
            "empty task should return error"
        );
    }

    #[test]
    fn test_mcp_auto_context_missing_task_returns_error() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("Error") || content.contains("error"),
            "missing task should return error"
        );
    }

    #[test]
    fn test_mcp_auto_context_first_in_tools_list() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools[0]["name"], "cxpak_auto_context",
            "cxpak_auto_context must be first in the tools list"
        );
    }

    // --- cxpak_context_diff MCP tool ---

    #[test]
    fn test_mcp_context_diff_no_snapshot_returns_recommendation() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_context_diff","arguments":{}}}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let result: Value = serde_json::from_str(content).unwrap();
        assert!(
            result["recommendation"]
                .as_str()
                .unwrap()
                .contains("cxpak_auto_context"),
            "no-snapshot recommendation should mention cxpak_auto_context"
        );
    }

    #[test]
    fn test_mcp_context_diff_after_auto_context_shows_no_changes() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        // Use a persistent snapshot shared across both calls.
        let snap = make_shared_snapshot();

        // First: call auto_context to establish a snapshot.
        let request1 = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cxpak_auto_context","arguments":{"task":"main function"}}}"#;
        let input1 = format!("{request1}\n");
        let mut output1 = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input1.as_bytes(), &mut output1).unwrap();
        assert!(snap.read().unwrap().is_some(), "snapshot must be set");

        // Second: call context_diff — should show no changes since index is unchanged.
        let request2 = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"cxpak_context_diff","arguments":{}}}"#;
        let input2 = format!("{request2}\n");
        let mut output2 = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input2.as_bytes(), &mut output2).unwrap();
        let response2: Value = serde_json::from_slice(&output2).unwrap();
        let content2 = response2["result"]["content"][0]["text"].as_str().unwrap();
        let result2: Value = serde_json::from_str(content2).unwrap();
        assert!(
            result2["recommendation"]
                .as_str()
                .unwrap()
                .contains("No changes"),
            "diff against identical index should report no changes, got: {}",
            result2["recommendation"]
        );
    }

    #[test]
    fn test_mcp_tools_list_includes_auto_context_and_diff() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(
            tool_names.contains(&"cxpak_auto_context"),
            "tools/list should include cxpak_auto_context"
        );
        assert!(
            tool_names.contains(&"cxpak_context_diff"),
            "tools/list should include cxpak_context_diff"
        );
        assert_eq!(tools.len(), 16, "total tool count should be 16");
    }

    // --- Task 11: cxpak_health MCP tool ---

    #[test]
    fn test_mcp_health_tool() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_health",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed["composite"].is_number(),
            "health result should have composite score"
        );
        assert!(
            parsed["conventions"].is_number(),
            "health result should have conventions score"
        );
        assert!(
            parsed["test_coverage"].is_number(),
            "health result should have test_coverage score"
        );
        assert!(
            parsed["churn_stability"].is_number(),
            "health result should have churn_stability score"
        );
        assert!(
            parsed["coupling"].is_number(),
            "health result should have coupling score"
        );
        assert!(
            parsed["cycles"].is_number(),
            "health result should have cycles score"
        );
        // composite should be in [0, 10]
        let composite = parsed["composite"].as_f64().unwrap();
        assert!(
            (0.0..=10.0).contains(&composite),
            "composite should be in [0, 10], got {composite}"
        );
    }

    // --- Task 12: cxpak_risks MCP tool ---

    #[test]
    fn test_mcp_risks_tool_returns_array() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(2)),
            "cxpak_risks",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed.as_array().is_some(),
            "risks result should be an array, got: {parsed}"
        );
        let risks = parsed.as_array().unwrap();
        // Risks count should be <= default limit of 10
        assert!(risks.len() <= 10, "risks should be capped at 10 by default");
        // Each entry should have the required fields
        for entry in risks {
            assert!(entry["path"].is_string(), "risk entry should have path");
            assert!(
                entry["risk_score"].is_number(),
                "risk entry should have risk_score"
            );
        }
    }

    #[test]
    fn test_mcp_risks_tool_with_custom_limit() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(3)),
            "cxpak_risks",
            &json!({"limit": 1}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        let risks = parsed.as_array().unwrap();
        assert!(
            risks.len() <= 1,
            "risks should be capped at custom limit of 1"
        );
    }

    // --- Task 13: cxpak_briefing MCP tool ---

    #[test]
    fn test_mcp_briefing_tool_returns_auto_context_result() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(4)),
            "cxpak_briefing",
            &json!({"task": "main function"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        // Should have auto_context result fields
        assert!(
            parsed["health"].is_object(),
            "briefing should have health object"
        );
        assert!(
            parsed["sections"].is_object(),
            "briefing should have sections object"
        );
        // In briefing mode, target_files entries should have null content
        let target_files = parsed["sections"]["target_files"]["files"]
            .as_array()
            .unwrap();
        for file in target_files {
            assert!(
                file["content"].is_null(),
                "briefing mode file content should be null for path: {}",
                file["path"]
            );
        }
    }

    #[test]
    fn test_mcp_briefing_tool_empty_task_errors() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(5)),
            "cxpak_briefing",
            &json!({"task": ""}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Error") || text.contains("error"),
            "empty task should return error"
        );
    }

    // --- Task 14: HTTP endpoints for health_score and risks ---

    #[test]
    fn test_axum_health_score_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/health_score")
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
            assert!(
                json["composite"].is_number(),
                "health_score endpoint should return composite"
            );
            let composite = json["composite"].as_f64().unwrap();
            assert!(
                (0.0..=10.0).contains(&composite),
                "composite should be in [0, 10], got {composite}"
            );
        });
    }

    #[test]
    fn test_axum_risks_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/risks")
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
            assert!(
                json.as_array().is_some(),
                "risks endpoint should return an array"
            );
            let risks = json.as_array().unwrap();
            assert!(risks.len() <= 10, "default limit is 10");
        });
    }

    #[test]
    fn test_axum_risks_endpoint_with_limit() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")));
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/risks?limit=1")
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
            let risks = json.as_array().unwrap();
            assert!(risks.len() <= 1, "limit=1 should return at most 1 entry");
        });
    }
}
