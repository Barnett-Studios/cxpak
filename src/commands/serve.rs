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
use crate::schema::{EdgeConfidence, EdgeType};
use axum::{
    extract::{DefaultBodyLimit, Query, State},
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
use subtle::ConstantTimeEq;

/// Shared, atomically-swappable handle to the active CodebaseIndex.
///
/// The double-Arc pattern (`Arc<RwLock<Arc<CodebaseIndex>>>`) lets readers
/// take a snapshot of the inner `Arc<CodebaseIndex>` in O(1) (a single
/// atomic refcount bump), drop the read lock, and then run arbitrarily
/// long handlers against the snapshot.  Writers (the watcher) build a
/// new `CodebaseIndex` off a clone of the previous snapshot, then swap
/// the inner Arc atomically — readers in flight continue to see the
/// pre-swap snapshot, never blocked, never returning torn state.
///
/// Without the inner Arc, the existing `RwLock<CodebaseIndex>` forced
/// long-running LSP handlers (predict, drift, securitySurface) to hold
/// the read guard for seconds, starving every concurrent watcher write.
pub type SharedIndex = Arc<RwLock<Arc<CodebaseIndex>>>;
pub type SharedSnapshot = Arc<RwLock<Option<crate::auto_context::diff::ContextSnapshot>>>;

/// Readiness of the MCP server's background index build (Task R0, ADR-0185).
///
/// `cxpak serve --mcp` answers the `initialize` handshake *immediately* and
/// builds the index on a background `std::thread`, publishing the outcome into a
/// [`SharedReadiness`] cell. A `tools/call` snapshots the cell: `Ready` runs
/// against the base index exactly as a synchronous build would (byte-identical
/// results); `Building`/`Failed` return a graceful JSON-RPC tool `result` — a
/// retry hint or a failure status — never a session-killing protocol error.
///
/// Deliberately an enum, not a bare bool, so Phase R-E1 can append a *second*
/// background phase (embedding enrichment layered on top of the ready base
/// index) as an additional state — e.g. a `ReadyEnriched` variant — without
/// reshaping the handshake path or the gating logic.
#[derive(Clone)]
pub enum IndexReadiness {
    /// Background base-index build in progress; not yet queryable.
    Building,
    /// The base index is ready. Cloning bumps the inner `Arc` refcount (O(1)).
    Ready(Arc<CodebaseIndex>),
    /// The base index is ready **and** enriched with an embedding index
    /// (similarity signal #7). Published by the R-E1 phase-2 background enrich
    /// swap (opt-in via `.cxpak.json`; ADR-0186). Snapshotted identically to
    /// [`Ready`][IndexReadiness::Ready] — the only difference is the attached
    /// `embedding_index`, so tool calls transparently pick up the 7-signal
    /// weight vector once the swap lands.
    ReadyEnriched(Arc<CodebaseIndex>),
    /// The background build failed; the message is surfaced on tool calls.
    Failed(String),
}

/// Shared, atomically-updated readiness cell for the background MCP index build.
///
/// Same double-Arc discipline as [`SharedIndex`]: readers clone the inner
/// `Arc<CodebaseIndex>` under a brief read lock (O(1)) and drop the lock before
/// running the tool handler; the background thread swaps the whole enum under a
/// brief write lock once its locally-built index is ready.
pub type SharedReadiness = Arc<RwLock<IndexReadiness>>;

/// Status returned for a `tools/call` that arrives before the background index
/// build has finished (Task R0). A normal tool `result` — not a protocol error —
/// so the MCP session stays alive and the client can simply retry.
const INDEXING_IN_PROGRESS_MESSAGE: &str =
    "cxpak: indexing in progress — the codebase is still being analyzed in the \
     background. Retry this call in a few seconds.";

/// Maximum allowed regex pattern length in the search endpoint.
/// Patterns beyond this limit risk catastrophic backtracking (ReDoS).
const MAX_PATTERN_LEN: usize = 1000;

fn matches_focus(path: &str, focus: Option<&str>) -> bool {
    match focus {
        Some(f) => path.starts_with(f),
        None => true,
    }
}

/// One auto_context target file: its path, role (selected vs. dependency), the
/// file that pulled it in (for dependencies), the edge type that linked them,
/// and that edge's [`EdgeConfidence`]. The confidence rides alongside the edge
/// type so the dependency annotation can flag heuristically inferred edges.
type AutoContextTarget = (
    String,
    FileRole,
    Option<String>,
    Option<EdgeType>,
    EdgeConfidence,
);

/// Render the `parent` annotation for an auto_context dependency.
///
/// Import edges (the common case) render the bare parent path. Every other
/// edge type appends `(via: <label>)`, and edges that were heuristically
/// [`Inferred`][EdgeConfidence::Inferred] (embedded-SQL regex, cross-language
/// bridges, heuristic column refs) additionally carry an `inferred` tag so the
/// reader knows the dependency was pattern-matched, not structurally extracted.
fn format_dependency_parent(
    parent: &str,
    edge_type: &EdgeType,
    confidence: EdgeConfidence,
) -> String {
    if *edge_type == EdgeType::Import {
        return parent.to_string();
    }
    let label = if confidence.is_inferred() {
        format!("{}, inferred", edge_type.label())
    } else {
        edge_type.label()
    };
    format!("{parent} (via: {label})")
}

/// Remove PatternObservation entries from a serialized convention JSON value
/// whose `percentage` field is below `min_pct`.
fn filter_observations_by_strength(val: &mut Value, min_pct: f64) {
    match val {
        Value::Object(map) => {
            for v in map.values_mut() {
                // If this value looks like a PatternObservation (has "percentage"),
                // check against the threshold and nullify if below.
                if let Some(pct) = v.get("percentage").and_then(|p| p.as_f64()) {
                    if pct < min_pct {
                        *v = Value::Null;
                    }
                } else {
                    filter_observations_by_strength(v, min_pct);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                filter_observations_by_strength(v, min_pct);
            }
            arr.retain(|v| !v.is_null());
        }
        _ => {}
    }
}

/// Remove entries from `file_contributions` objects in a serialized convention
/// value whose key does not start with `focus_prefix`.
fn filter_contributions_by_focus(val: &mut Value, focus_prefix: &str) {
    if let Some(map) = val.as_object_mut() {
        if let Some(contributions) = map.get_mut("file_contributions") {
            if let Some(contrib_map) = contributions.as_object_mut() {
                contrib_map.retain(|k, _| k.starts_with(focus_prefix));
            }
        }
        // Recurse into nested objects
        for v in map.values_mut() {
            if v.is_object() {
                filter_contributions_by_focus(v, focus_prefix);
            }
        }
    }
}

/// Scan and parse all files in a path, returning a fully built CodebaseIndex.
pub fn build_index(path: &Path) -> Result<CodebaseIndex, Box<dyn std::error::Error>> {
    build_index_with_workspace(path, None)
}

/// Scan and parse files in a path, optionally scoped to a workspace prefix.
pub fn build_index_with_workspace(
    path: &Path,
    workspace: Option<&str>,
) -> Result<CodebaseIndex, Box<dyn std::error::Error>> {
    let counter = TokenCounter::new();
    let registry = LanguageRegistry::new();

    let scanner = Scanner::new(path)?;
    let files = scanner.scan_workspace(workspace)?;

    let mut parse_results = HashMap::new();
    let mut content_map = HashMap::new();
    // Capture mtime_ns + size_bytes per file before `files` is consumed by
    // build_with_content.  These feed the stat-index to skip re-hashing
    // files whose (mtime_ns, size_bytes) are unchanged (Task 0.2).
    let mut file_stats: HashMap<String, (u64, u64)> = HashMap::new();
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
        let mtime_ns = crate::cache::file_mtime_ns(&file.absolute_path);
        file_stats.insert(file.relative_path.clone(), (mtime_ns, file.size_bytes));
        content_map.insert(file.relative_path.clone(), source);
    }

    let mut index = CodebaseIndex::build_with_content(files, parse_results, &counter, content_map);

    // Derived-index cache (ADR-0167): on a content+HEAD fingerprint hit, restore
    // the derived analysis — crucially skipping the expensive git-mined
    // conventions/co-changes recompute — instead of re-mining history. The
    // fingerprint is content-based, so the cache is portable across clones/CI
    // and never serves stale data on a same-size edit. Fail-closed: any miss /
    // corruption falls through to a full rebuild.
    let cache_dir = path.join(cache_namespace(path, workspace));
    let head_oid = git_head_oid(path);

    // Build file list with mtime_ns + size_bytes for the stat-index fast-path.
    // Files not present in file_stats (shouldn't happen — metadata read failed)
    // fall back to (0, 0), a degenerate value that will only hit a stat-index
    // entry previously stored with the same (0, 0) key; in practice this means
    // a failed-metadata file is re-hashed on every build, which is safe.
    let fp_files: Vec<(String, String, u64, u64)> = index
        .files
        .iter()
        .map(|f| {
            let (mtime_ns, size_bytes) =
                file_stats.get(&f.relative_path).copied().unwrap_or((0, 0));
            (
                f.relative_path.clone(),
                f.content.clone(),
                mtime_ns,
                size_bytes,
            )
        })
        .collect();

    // Load the stat-index (fail-closed: empty on any error).
    let mut stat_index = crate::cache::StatIndex::load(&cache_dir);

    // Production content hasher: strips markdown frontmatter, then SHA-256.
    let fingerprint = crate::cache::content_fingerprint_with_stat_index(
        &fp_files,
        &head_oid,
        &mut stat_index,
        |content| {
            use sha2::{Digest, Sha256};
            let body = crate::cache::strip_md_frontmatter(content);
            format!("{:x}", Sha256::digest(body.as_bytes()))
        },
    );

    // Persist the updated stat-index (best-effort; a write failure must not
    // fail indexing).
    let _ = stat_index.save(&cache_dir);

    match crate::cache::DerivedCache::load(&cache_dir, &fingerprint) {
        Some(derived) => {
            index.graph = derived.graph;
            index.pagerank = derived.pagerank;
            index.call_graph = derived.call_graph;
            index.conventions = derived.conventions;
            index.co_changes = derived.co_changes;
        }
        None => {
            index.conventions = crate::conventions::build_convention_profile(&index, path);
            index.co_changes = index.conventions.git_health.co_changes.clone();
            // Stamp the HEAD SHA this analysis was built at so the next
            // post-commit edge-delta can validate its base (ADR-0179). The stamp
            // means "graph == committed tree at this SHA", so it is recorded ONLY
            // when the working tree is CLEAN vs HEAD. `overview`/`auto_context`
            // routinely run on a dirty tree (uncommitted edits); stamping HEAD
            // there would mislabel a graph built from uncommitted content as the
            // clean base, letting a later commit delta onto it (silent
            // corruption). An empty oid (no git repo / unborn HEAD), a dirty tree,
            // or any status error → `None`.
            let base_commit = if head_oid.is_empty() {
                None
            } else {
                match git2::Repository::discover(path) {
                    Ok(repo) if working_tree_clean(&repo) => Some(head_oid.clone()),
                    _ => None,
                }
            };
            let derived = crate::cache::DerivedCache::new(
                fingerprint,
                index.graph.clone(),
                index.call_graph.clone(),
                index.pagerank.clone(),
                index.conventions.clone(),
                index.co_changes.clone(),
                base_commit,
            );
            // Persisting is best-effort; a write failure must not fail indexing.
            let _ = derived.save(&cache_dir);
        }
    }
    Ok(index)
}

/// Current git HEAD commit oid as a hex string, or `""` when `path` is not a
/// git repository / has no commits. Part of the derived-cache fingerprint so a
/// HEAD move invalidates history-derived data (conventions, co-changes).
///
/// Public so the post-commit rebuild (`commands::hook`) computes the SAME
/// content fingerprint as `build_index`, keeping the shared derived cache it
/// writes a valid warm hit for a later `overview` (ADR-0179).
pub fn git_head_oid(path: &Path) -> String {
    let Ok(repo) = git2::Repository::discover(path) else {
        return String::new();
    };
    repo.head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| oid.to_string())
        .unwrap_or_default()
}

/// Whether the repo's working tree is CLEAN versus HEAD — i.e. no tracked-file
/// modifications, staged changes, or deletions. Untracked and ignored files are
/// deliberately excluded (a new scratch file does not make the committed tree
/// stale). Any status error degrades to "dirty" (`false`), fail-closed.
///
/// This is the truth condition behind a `base_commit = Some(HEAD)` stamp: the
/// stamp promises "graph == committed tree at this SHA", which only holds when
/// the working tree the graph was built from equals HEAD's tree (ADR-0179).
pub(crate) fn working_tree_clean(repo: &git2::Repository) -> bool {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false)
        .include_ignored(false)
        .exclude_submodules(true);
    match repo.statuses(Some(&mut opts)) {
        // With untracked + ignored excluded, any remaining entry is a tracked
        // modification/staged/deleted change → the tree differs from HEAD.
        Ok(statuses) => statuses.iter().all(|entry| entry.status().is_empty()),
        Err(_) => false,
    }
}

/// Returns a cache directory name scoped to the given workspace.
///
/// When workspace is None: ".cxpak/cache/root"
/// When workspace is Some("packages/api"): ".cxpak/cache/packages_api"
#[allow(dead_code)]
pub fn cache_namespace(repo_root: &std::path::Path, workspace: Option<&str>) -> String {
    let _ = repo_root;
    match workspace {
        None => ".cxpak/cache/root".to_string(),
        Some(ws) => format!(".cxpak/cache/{}", ws.replace('/', "_")),
    }
}

type SharedPath = Arc<std::path::PathBuf>;

#[derive(Clone)]
struct AppState {
    index: SharedIndex,
    repo_path: SharedPath,
    snapshot: SharedSnapshot,
    expected_token: Option<String>,
    #[allow(dead_code)]
    workspace_root: Arc<std::path::PathBuf>,
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

pub fn validate_workspace_path(
    workspace: &std::path::Path,
    requested: &str,
) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(requested);
    if p.is_absolute() {
        return Err(format!("absolute paths rejected: {requested}"));
    }

    // Canonicalize the workspace base (must exist).
    let ws_canon = workspace
        .canonicalize()
        .map_err(|e| format!("workspace canonicalize failed: {e}"))?;
    let ws_depth = ws_canon.components().count();

    // Build a lexically-normalized candidate by tracking a component stack.
    // This resolves `..` components without touching the filesystem, which
    // prevents traversal even when intermediate directories do not exist.
    let mut stack: Vec<std::ffi::OsString> = ws_canon
        .components()
        .map(|c| c.as_os_str().to_os_string())
        .collect();
    let mut traversal_attempt = false;

    for component in p.components() {
        match component {
            std::path::Component::ParentDir => {
                if stack.len() > ws_depth {
                    stack.pop();
                } else {
                    // The `..` would go above the workspace root — flag this
                    // as a traversal attempt regardless of where the path ends.
                    traversal_attempt = true;
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(seg) => {
                stack.push(seg.to_os_string());
            }
            // RootDir / Prefix cannot appear in a relative path; already
            // rejected absolute paths above.
            _ => {}
        }
    }

    if traversal_attempt {
        return Err(format!("path traversal rejected: {requested}"));
    }

    let resolved: std::path::PathBuf = stack.iter().collect();

    // If the resolved path exists on disk, canonicalize it to catch any
    // symlink-based escapes. For non-existent paths the lexical check is
    // sufficient.
    let final_path = if resolved.exists() {
        resolved
            .canonicalize()
            .map_err(|e| format!("path canonicalize failed: {e}"))?
    } else {
        resolved
    };

    if !final_path.starts_with(&ws_canon) {
        return Err(format!("path escapes workspace: {requested}"));
    }
    Ok(final_path)
}

pub fn extract_bearer_token(header: &str) -> Option<&str> {
    header.strip_prefix("Bearer ")
}

/// Compare the provided Bearer token against the expected token in constant time.
///
/// Uses `subtle::ConstantTimeEq` to prevent timing side-channel attacks.
/// Tokens of different byte lengths are rejected immediately before the
/// constant-time comparison so that the length itself leaks no additional
/// information beyond what is already visible (the comparison fails).
pub fn check_auth(expected: Option<&str>, provided: Option<&str>) -> bool {
    match expected {
        None => true,
        Some(tok) => match provided {
            None => false,
            Some(prov) => {
                // Reject length mismatch first; both branches are falsy so no
                // useful timing information is revealed.
                if tok.len() != prov.len() {
                    return false;
                }
                bool::from(tok.as_bytes().ct_eq(prov.as_bytes()))
            }
        },
    }
}

/// Public test helper: builds a router with the given shared state.
/// Used by integration tests that cannot access the private `build_router`.
pub fn build_router_for_test(shared: SharedIndex, repo_path: SharedPath) -> Router {
    build_router(shared, repo_path, None)
}

/// Public test helper: builds a router with optional auth token.
pub fn build_router_for_test_with_token(
    shared: SharedIndex,
    repo_path: SharedPath,
    token: Option<String>,
) -> Router {
    build_router(shared, repo_path, token)
}

/// Build the axum Router for the HTTP server.
/// Inject defense-in-depth security headers on every response.
///
/// `cxpak serve` returns only JSON; no inline scripts, no third-party
/// resources, no framing.  A strict CSP costs nothing here and gives
/// browsers a clear signal that any unexpected script execution is
/// disallowed if a response is ever mis-typed as HTML.
async fn security_headers_layer(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    // No script, no styles, no images, no frames, nothing.  cxpak/serve
    // emits JSON exclusively; tightening to `'none'` makes a wrong-MIME
    // response inert in a browser.
    headers.insert(
        "Content-Security-Policy",
        axum::http::HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    headers.insert(
        "X-Content-Type-Options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "Referrer-Policy",
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "X-Frame-Options",
        axum::http::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        "Cross-Origin-Resource-Policy",
        axum::http::HeaderValue::from_static("same-origin"),
    );
    response
}

fn build_router(shared: SharedIndex, repo_path: SharedPath, token: Option<String>) -> Router {
    let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
    let state = AppState {
        index: shared,
        repo_path: repo_path.clone(),
        snapshot,
        expected_token: token,
        workspace_root: repo_path,
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
        .route("/call_graph", axum::routing::post(call_graph_handler))
        .route("/dead_code", axum::routing::post(dead_code_handler))
        .route("/architecture", axum::routing::post(architecture_handler))
        .route("/predict", axum::routing::post(predict_handler))
        .route("/drift", axum::routing::post(drift_handler))
        .route(
            "/security_surface",
            axum::routing::post(security_surface_handler),
        )
        .route("/data_flow", axum::routing::post(data_flow_handler))
        .route("/cross_lang", get(cross_lang_handler))
        .merge(build_v1_router(state.clone()))
        // Security layer applied LAST so it wraps both legacy + v1 routes.
        // Tower layers wrap outside-in: layers added later sit at the
        // outermost edge and run for every request that reaches a child
        // route, including everything pulled in via `.merge`.
        .layer(axum::middleware::from_fn(security_headers_layer))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(state)
}

fn build_v1_router(state: AppState) -> Router<AppState> {
    use axum::extract::Request;
    use axum::middleware::{self, Next};
    use axum::response::Response;

    async fn auth_layer(
        axum::extract::State(state): axum::extract::State<AppState>,
        req: Request,
        next: Next,
    ) -> Result<Response, StatusCode> {
        let provided = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(extract_bearer_token);
        if check_auth(state.expected_token.as_deref(), provided) {
            Ok(next.run(req).await)
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }

    let repo_path = state.repo_path.clone();
    Router::new()
        .route("/v1/health", axum::routing::get(v1_health_handler))
        .route("/v1/risks", axum::routing::post(v1_risks_handler))
        .route(
            "/v1/architecture",
            axum::routing::post(v1_architecture_handler),
        )
        .route("/v1/call_graph", axum::routing::post(v1_call_graph_handler))
        .route("/v1/graph", axum::routing::post(v1_graph_handler))
        .route("/v1/retrieval", axum::routing::post(v1_retrieval_handler))
        .route("/v1/dead_code", axum::routing::post(v1_dead_code_handler))
        .route("/v1/predict", axum::routing::post(v1_predict_handler))
        .route("/v1/drift", axum::routing::post(v1_drift_handler))
        .route(
            "/v1/security_surface",
            axum::routing::post(v1_security_surface_handler),
        )
        .route("/v1/data_flow", axum::routing::post(v1_data_flow_handler))
        .route("/v1/cross_lang", axum::routing::post(v1_cross_lang_handler))
        .route(
            "/v1/conventions",
            axum::routing::post(v1_conventions_handler),
        )
        .route("/v1/briefing", axum::routing::post(v1_briefing_handler))
        .layer(axum::Extension(repo_path))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_layer))
        .with_state(state)
}

#[derive(Deserialize)]
struct V1FocusParams {
    focus: Option<String>,
    workspace: Option<String>,
}

/// Optional request body for `POST /v1/conventions`.
/// All fields are optional; an empty `{}` body (or a body with unknown keys)
/// deserialises without error, keeping backward compatibility.
#[derive(Deserialize, Default)]
struct V1ConventionsParams {
    /// Token budget for the response; defaults to `MAX_MCP_CONVENTIONS_TOKENS`.
    tokens: Option<usize>,
    /// Category filter: "all" | "naming" | "imports" | "errors" | "dependencies"
    ///                  | "testing" | "visibility" | "functions" | "git_health".
    category: Option<String>,
    /// Minimum pattern strength: "all" | "mixed" | "trend" | "convention".
    strength: Option<String>,
    /// Path prefix to filter `file_contributions` entries.
    focus: Option<String>,
}

#[derive(serde::Deserialize)]
struct V1PredictParams {
    files: Option<Vec<String>>,
    depth: Option<usize>,
    focus: Option<String>,
    workspace: Option<String>,
}

#[derive(serde::Deserialize)]
struct V1DataFlowParams {
    symbol: Option<String>,
    depth: Option<usize>,
    focus: Option<String>,
    workspace: Option<String>,
}

#[derive(serde::Deserialize)]
struct V1CallGraphParams {
    target: Option<String>,
    focus: Option<String>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
struct V1BriefingParams {
    task: String,
    tokens: Option<usize>,
    focus: Option<String>,
    /// Opt-in: model name for a USD cost estimate in the efficiency report.
    cost_model: Option<String>,
}

pub fn v1_error(
    status: StatusCode,
    code: &'static str,
    msg: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error": code, "message": msg.into()})))
}

pub fn normalize_path_param(value: &str) -> Result<String, (StatusCode, Json<Value>)> {
    if value.len() > 1024 {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "param_too_long",
            "path exceeds 1024 chars",
        ));
    }
    if value.contains('\0') || value.contains('\\') {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "invalid_param",
            "illegal character",
        ));
    }
    if value.split('/').any(|seg| seg == "..") {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "invalid_param",
            "path traversal segment",
        ));
    }
    if value.starts_with('/') {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "invalid_param",
            "absolute path",
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
    {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "invalid_param",
            "illegal character class",
        ));
    }
    Ok(value.to_string())
}

pub fn normalize_symbol_param(value: &str) -> Result<String, (StatusCode, Json<Value>)> {
    if value.len() > 512 {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "param_too_long",
            "symbol exceeds 512 chars",
        ));
    }
    if value.contains('\0') {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "invalid_param",
            "null byte",
        ));
    }
    if value
        .chars()
        .any(|c| c.is_control() || matches!(c, '/' | '\\' | '`' | '$' | ';' | '|'))
    {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "invalid_param",
            "illegal character",
        ));
    }
    Ok(value.to_string())
}

pub fn validate_visual_type_slug(s: &str) -> Result<&'static str, String> {
    match s {
        "dashboard" => Ok("dashboard"),
        "architecture" => Ok("architecture"),
        "risk" => Ok("risk"),
        "flow" => Ok("flow"),
        "timeline" => Ok("timeline"),
        "diff" => Ok("diff"),
        "all" => Ok("all"),
        _ => Err(format!("invalid_type: {s}")),
    }
}

async fn v1_health_handler(State(index): State<SharedIndex>) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // Cached so polling /v1/health doesn't re-run the 5 scoring passes
    // every request.  Cache is invalidated by process_watcher_changes.
    let health = idx.health_cached();
    Ok(Json(serde_json::json!({
        "total_files": idx.total_files,
        "total_tokens": idx.total_tokens,
        "composite": health.composite,
        "dimensions": {
            "conventions": health.conventions,
            "test_coverage": health.test_coverage,
            "churn_stability": health.churn_stability,
            "coupling": health.coupling,
            "cycles": health.cycles,
            "dead_code": health.dead_code,
        },
    })))
}

async fn v1_conventions_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<V1ConventionsParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let profile = &idx.conventions;
    let category = params.category.as_deref().unwrap_or("all");
    let strength_filter = params.strength.as_deref().unwrap_or("all");
    let focus = params.focus.as_deref();
    let token_budget = params
        .tokens
        .unwrap_or(crate::conventions::render::MAX_MCP_CONVENTIONS_TOKENS);

    let mut result = match category {
        "naming" => serde_json::to_value(&profile.naming),
        "imports" => serde_json::to_value(&profile.imports),
        "errors" => serde_json::to_value(&profile.errors),
        "dependencies" => serde_json::to_value(&profile.dependencies),
        "testing" => serde_json::to_value(&profile.testing),
        "visibility" => serde_json::to_value(&profile.visibility),
        "functions" => serde_json::to_value(&profile.functions),
        "git_health" => serde_json::to_value(&profile.git_health),
        _ => serde_json::to_value(profile),
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let min_pct: f64 = match strength_filter {
        "convention" => 90.0,
        "trend" => 70.0,
        "mixed" => 50.0,
        _ => 0.0,
    };
    if min_pct > 0.0 {
        filter_observations_by_strength(&mut result, min_pct);
    }
    if let Some(focus_prefix) = focus {
        filter_contributions_by_focus(&mut result, focus_prefix);
    }

    let result = crate::conventions::render::render_budgeted_conventions(result, token_budget);
    Ok(Json(result))
}

async fn v1_briefing_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<V1BriefingParams>,
) -> Result<Json<Value>, StatusCode> {
    // Clone the index out before releasing the read lock so the lock is not held
    // across the auto_context computation, which would starve the watcher thread.
    let idx = {
        let guard = index
            .read()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        guard.clone()
    };
    let opts = crate::auto_context::AutoContextOpts {
        cost_model: params.cost_model,
        tokens: params.tokens.unwrap_or(50_000),
        focus: params.focus,
        include_tests: true,
        include_blast_radius: true,
        mode: "briefing".to_string(),
    };
    let result = crate::auto_context::auto_context(&params.task, &idx, &opts);
    serde_json::to_value(&result)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn v1_risks_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params
        .focus
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let mut risks = crate::intelligence::risk::compute_risk_ranking(&idx);
    if let Some(ref prefix) = focus {
        risks.retain(|r| r.path.starts_with(prefix));
    }
    Ok(axum::Json(serde_json::json!({"risks": risks})))
}

async fn v1_architecture_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params
        .focus
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let mut map = crate::intelligence::architecture::build_architecture_map(&idx, 2);
    if let Some(ref prefix) = focus {
        map.modules.retain(|m| m.prefix.starts_with(prefix));
    }
    Ok(axum::Json(
        serde_json::json!({"modules": map.modules, "circular_deps": map.circular_deps}),
    ))
}

async fn v1_call_graph_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1CallGraphParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let target = match params
        .target
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(t) => Some(normalize_path_param(t)?),
        None => None,
    };
    let focus = match params
        .focus
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let cg = &idx.call_graph;
    let filtered: Vec<_> = cg
        .edges
        .iter()
        .filter(|e| {
            let t_match = target
                .as_ref()
                .map(|t| {
                    e.caller_file.contains(t.as_str())
                        || e.callee_file.contains(t.as_str())
                        || e.caller_symbol.contains(t.as_str())
                        || e.callee_symbol.contains(t.as_str())
                })
                .unwrap_or(true);
            let f_match = focus
                .as_ref()
                .map(|f| {
                    e.caller_file.starts_with(f.as_str()) || e.callee_file.starts_with(f.as_str())
                })
                .unwrap_or(true);
            t_match && f_match
        })
        .collect();
    Ok(axum::Json(
        serde_json::json!({"edges": filtered, "total": cg.edges.len()}),
    ))
}

async fn v1_dead_code_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params
        .focus
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let dead = crate::intelligence::dead_code::detect_dead_code(&idx, focus.as_deref());
    let total = dead.len();
    Ok(axum::Json(
        serde_json::json!({"dead_symbols": dead, "total": total}),
    ))
}

async fn v1_predict_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1PredictParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let files = params
        .files
        .as_ref()
        .ok_or_else(|| v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "files"))?;
    if files.is_empty() {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "missing_required_param",
            "files must be non-empty",
        ));
    }
    if files.len() > 100 {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "param_too_long",
            "max 100 files",
        ));
    }
    let mut normalized: Vec<String> = Vec::with_capacity(files.len());
    for f in files {
        normalized.push(normalize_path_param(f)?);
    }
    let depth = params.depth.unwrap_or(3);
    if depth > 10 {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "depth_exceeds_max",
            "max depth 10",
        ));
    }
    if let Some(f) = params.focus.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(f)?;
    }
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let refs: Vec<&str> = normalized.iter().map(|s| s.as_str()).collect();
    let result = crate::intelligence::predict::predict(
        &refs,
        &idx.graph,
        &idx.pagerank,
        &idx.co_changes,
        &idx.test_map,
        depth,
    );
    Ok(axum::Json(serde_json::to_value(result).unwrap()))
}

/// `POST /v1/graph` — deterministic graph-query (cxpak 3.0.0 Task B1).
///
/// The request body is `{ "op": "node"|"neighbors"|"path"|"subgraph", ... }`;
/// the remaining fields are the op's params. The body is passed straight to the
/// single core [`crate::intelligence::graph_query::execute`] — this surface only
/// adapts transport, it does not re-derive. Malformed requests (missing `op`,
/// missing a required param, bad direction, unknown op) map to `400`.
async fn v1_graph_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let op = body
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "op"))?
        .to_string();
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    crate::intelligence::graph_query::execute(&idx.graph, &op, &body)
        .map(axum::Json)
        .map_err(|e| v1_error(StatusCode::BAD_REQUEST, "invalid_request", e.to_string()))
}

/// Deterministic iterative retrieval over cxpak's own index (cxpak 3.0.0 Task
/// C1, ADR-0180). Body: `{ "op": "search"|"references"|"expand", ... }`, passed
/// straight to the single core `retrieval::execute` — the same byte-deterministic
/// JSON the CLI/LSP/MCP surfaces return. Read-only.
async fn v1_retrieval_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let op = body
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "op"))?
        .to_string();
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    crate::intelligence::retrieval::execute(&idx, &op, &body)
        .map(axum::Json)
        .map_err(|e| v1_error(StatusCode::BAD_REQUEST, "invalid_request", e.to_string()))
}

async fn v1_drift_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::extract::Extension(repo): axum::extract::Extension<SharedPath>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params
        .focus
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let mut report = crate::intelligence::drift::build_drift_report(&idx, &repo, false);
    if let Some(ref prefix) = focus {
        report.hotspots.retain(|h| h.module.starts_with(prefix));
    }
    Ok(axum::Json(serde_json::to_value(report).unwrap()))
}

async fn v1_security_surface_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus_owned =
        match params
            .focus
            .as_deref()
            .and_then(|s| if s.is_empty() { None } else { Some(s) })
        {
            Some(f) => Some(normalize_path_param(f)?),
            None => None,
        };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let result = crate::intelligence::security::build_security_surface(
        &idx,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        focus_owned.as_deref(),
    );
    Ok(axum::Json(serde_json::to_value(result).unwrap()))
}

async fn v1_data_flow_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1DataFlowParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let symbol = params
        .symbol
        .as_deref()
        .ok_or_else(|| v1_error(StatusCode::BAD_REQUEST, "missing_required_param", "symbol"))?;
    let symbol = normalize_symbol_param(symbol)?;
    let depth = params.depth.unwrap_or(6);
    if depth > 10 {
        return Err(v1_error(
            StatusCode::BAD_REQUEST,
            "depth_exceeds_max",
            "max depth 10",
        ));
    }
    if let Some(f) = params.focus.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(f)?;
    }
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let result = crate::intelligence::data_flow::trace_data_flow(&symbol, None, depth, &idx);
    Ok(axum::Json(serde_json::to_value(result).unwrap()))
}

async fn v1_cross_lang_handler(
    axum::extract::State(index): axum::extract::State<SharedIndex>,
    axum::Json(params): axum::Json<V1FocusParams>,
) -> Result<axum::Json<serde_json::Value>, (StatusCode, axum::Json<serde_json::Value>)> {
    let focus = match params
        .focus
        .as_deref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        Some(f) => Some(normalize_path_param(f)?),
        None => None,
    };
    if let Some(ws) = params.workspace.as_deref().filter(|s| !s.is_empty()) {
        normalize_path_param(ws)?;
    }
    let idx = index.read().map_err(|_| {
        v1_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "lock poisoned",
        )
    })?;
    let edges: Vec<_> = if let Some(ref prefix) = focus {
        idx.cross_lang_edges
            .iter()
            .filter(|e| {
                e.source_file.contains(prefix.as_str()) || e.target_file.contains(prefix.as_str())
            })
            .cloned()
            .collect()
    } else {
        idx.cross_lang_edges.clone()
    };
    Ok(axum::Json(serde_json::json!({"edges": edges})))
}

/// Reject startup configurations that would expose the API to other
/// hosts without authentication.
///
/// `cxpak serve` returns full source-bearing intelligence responses;
/// without this guard, `cxpak serve --bind 0.0.0.0` exposes the entire
/// codebase content to any host that can reach the port.  Loopback
/// (127.0.0.1, ::1) is permitted token-less because it already requires
/// local OS access.
///
/// Public for adversarial testing — the rule is too important to leave
/// untestable inside `run()` (which would otherwise have to bind a
/// real socket to be exercised).
pub fn validate_bind_security(
    addr: &std::net::SocketAddr,
    token: Option<&str>,
) -> Result<(), String> {
    // Treat the empty string as no-token at startup (fail-fast) rather
    // than relying on the per-request bearer middleware to reject it
    // later.  An operator who launches `cxpak serve --bind 0.0.0.0
    // --token ""` is almost certainly mistaken about what they typed; a
    // startup error tells them at the moment they can fix it.
    let effective_token = token.filter(|t| !t.is_empty());
    if !addr.ip().is_loopback() && effective_token.is_none() {
        return Err(format!(
            "refusing to bind {addr} without --token: a non-loopback listener \
             is reachable by other hosts and MUST be authenticated with a \
             non-empty bearer token. Either set --token <secret> or bind to \
             127.0.0.1 / ::1."
        ));
    }
    Ok(())
}

pub fn run(
    path: &Path,
    port: u16,
    bind: &str,
    token: Option<&str>,
    _token_budget: usize,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: std::net::SocketAddr = format!("{bind}:{port}")
        .parse()
        .map_err(|e| format!("invalid bind address '{bind}:{port}': {e}"))?;

    validate_bind_security(&addr, token)?;

    let index = build_index(path)?;

    eprintln!(
        "cxpak: serving {} ({} files indexed, {} tokens) on {addr}",
        path.display(),
        index.total_files,
        index.total_tokens,
    );

    // Wrap in inner Arc so the lock guards a cheap-to-clone handle, not
    // the full CodebaseIndex.  Readers snapshot in O(1); writers swap
    // atomically (see SharedIndex docs).
    let shared = Arc::new(RwLock::new(Arc::new(index)));
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

    // Treat empty-string token consistently with `validate_bind_security`:
    // if the operator typed `--token ""` (loopback bind path that survived
    // the security guard), the token must NOT be installed — otherwise the
    // bearer middleware would silently accept clients sending an empty
    // `Authorization: Bearer` header, contradicting the validate_bind
    // semantics that empty == not configured.
    let app = build_router(
        shared,
        shared_path,
        token.filter(|s| !s.is_empty()).map(|s| s.to_string()),
    );

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        eprintln!("cxpak: listening on http://{addr}");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let shutdown = async {
            // Listen for SIGTERM in addition to Ctrl-C: containerised
            // environments (kubectl, systemd, docker stop) and most CI process
            // killers send SIGTERM, not SIGINT.  Without this branch the
            // process is force-killed by the kernel after the grace period
            // and any in-flight request is dropped mid-write.
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                match signal(SignalKind::terminate()) {
                    Ok(mut term) => {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {},
                            _ = term.recv() => {},
                        }
                    }
                    Err(_) => {
                        tokio::signal::ctrl_c().await.ok();
                    }
                }
            }
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c().await.ok();
            }
            eprintln!("cxpak: shutting down gracefully...");
        };
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
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
) -> (StatusCode, Json<Value>) {
    let target = match params.target {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing required query parameter: target"})),
            );
        }
    };

    let idx = match index.read() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "index lock poisoned"})),
            );
        }
    };
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);

    let found =
        !idx.find_symbol(&target).is_empty() || !idx.find_content_matches(&target).is_empty();

    (
        StatusCode::OK,
        Json(json!({
            "target": target,
            "token_budget": token_budget,
            "found": found,
            "total_files": idx.total_files,
            "total_tokens": idx.total_tokens,
        })),
    )
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
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);

    let changes = crate::commands::diff::extract_changes(&repo_path, git_ref)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Approximate 1 token ≈ 4 chars (English text avg).  Pack files until
    // the budget is reached, then signal truncation in the response so the
    // caller can decide to retry with a larger budget or paginate.
    let counter = crate::budget::counter::TokenCounter::new();
    let total_changed = changes.len();
    let mut files: Vec<Value> = Vec::new();
    let mut tokens_used = 0usize;
    let mut truncated = false;
    for c in &changes {
        let tokens = counter.count(&c.diff_text);
        if tokens_used + tokens > token_budget && !files.is_empty() {
            truncated = true;
            break;
        }
        tokens_used = tokens_used.saturating_add(tokens);
        files.push(json!({
            "path": c.path,
            "diff": c.diff_text,
        }));
    }

    Ok(Json(json!({
        "git_ref": git_ref.unwrap_or("working tree"),
        "changed_files": total_changed,
        "showing": files.len(),
        "truncated": truncated,
        "tokens_used": tokens_used,
        "token_budget": token_budget,
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
) -> (StatusCode, Json<Value>) {
    if params.pattern.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "pattern is required and must not be empty"})),
        );
    }

    if params.pattern.len() > MAX_PATTERN_LEN {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "pattern length {} exceeds maximum allowed length {}",
                    params.pattern.len(),
                    MAX_PATTERN_LEN
                )
            })),
        );
    }

    let re = match regex::Regex::new(&params.pattern) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid regex: {e}")})),
            )
        }
    };

    let idx = match index.read() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "index lock poisoned"})),
            );
        }
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

    (
        StatusCode::OK,
        Json(json!({
            "pattern": params.pattern,
            "matches": matches_vec,
            "total_matches": total_matches,
            "files_searched": files_searched,
            "truncated": total_matches > limit,
        })),
    )
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
    if params.files.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
    /// Opt-in: model name for a USD cost estimate in the efficiency report.
    cost_model: Option<String>,
}

async fn auto_context_handler(
    State(index): State<SharedIndex>,
    State(snapshot): State<SharedSnapshot>,
    Json(params): Json<AutoContextParams>,
) -> Result<Json<Value>, StatusCode> {
    if params.task.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Clone the index out from behind the read lock so the lock is not held
    // across the (potentially slow) auto_context computation, which would
    // starve the background watcher thread waiting on the write lock.
    let idx = {
        let guard = index
            .read()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        guard.clone()
    };
    let token_budget = params
        .tokens
        .as_deref()
        .and_then(|t| crate::cli::parse_token_count(t).ok())
        .unwrap_or(50_000);
    let opts = crate::auto_context::AutoContextOpts {
        cost_model: params.cost_model,
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
    /// `since` is an ISO-8601-ish timestamp threshold.  When set, the
    /// stored snapshot is rejected if its `generated_at` is older than
    /// the threshold and the response says "snapshot too old; refresh
    /// auto_context to capture a new baseline".  Lets clients ignore
    /// stale baselines without first probing.
    since: Option<String>,
}

async fn context_diff_handler(
    State(index): State<SharedIndex>,
    State(snapshot): State<SharedSnapshot>,
    Query(params): Query<ContextDiffParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let snap_guard = snapshot
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let delta = match snap_guard.as_ref() {
        None => crate::auto_context::diff::no_snapshot_recommendation(),
        Some(snap) => {
            // Honour `since`: lexicographic comparison works for
            // ISO-8601-ish timestamps (the snapshot.generated_at field
            // uses RFC-3339 strings which sort the same way as their
            // wall-clock order).
            if let Some(threshold) = params.since.as_deref() {
                if snap.generated_at.as_str() < threshold {
                    let mut rec = crate::auto_context::diff::no_snapshot_recommendation();
                    rec.recommendation = format!(
                        "snapshot generated_at {} predates `since` threshold {}; \
                         call /auto_context to refresh the baseline before diffing",
                        snap.generated_at, threshold
                    );
                    return Ok(Json(
                        serde_json::to_value(&rec)
                            .unwrap_or_else(|_| json!({"error": "serialisation failed"})),
                    ));
                }
            }
            crate::auto_context::diff::compute_diff(snap, &idx)
        }
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

// --- v1.3.0 MCP endpoints: call_graph, dead_code, architecture ---

#[derive(Deserialize)]
struct CallGraphParams {
    target: Option<String>,
    /// Maximum hops from `target` to include in the response. Default
    /// 3, capped at 8.  Without this filter a full-codebase query
    /// returns the entire call_graph (megabytes on real repos).
    depth: Option<usize>,
    focus: Option<String>,
    /// MCP-style alias for `focus` — the cxpak_call_graph MCP tool
    /// accepts `workspace` as a workspace-prefix filter; the v1 path
    /// now honours the same name so clients can use one parameter
    /// shape across both transports.
    workspace: Option<String>,
}

async fn call_graph_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<CallGraphParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let cg = &idx.call_graph;
    let depth_cap = params.depth.unwrap_or(3).min(8);
    // Workspace acts as a focus prefix when focus is not set (matches MCP).
    let effective_focus = params.focus.as_deref().or(params.workspace.as_deref());

    let filtered_edges: Vec<&crate::intelligence::call_graph::CallEdge> =
        if let Some(target) = params.target.as_deref() {
            // Initial set: edges that mention the target.
            let mut frontier: std::collections::HashSet<String> = std::collections::HashSet::new();
            for e in cg.edges.iter().filter(|e| {
                e.caller_file.contains(target)
                    || e.callee_file.contains(target)
                    || e.caller_symbol.contains(target)
                    || e.callee_symbol.contains(target)
            }) {
                frontier.insert(e.caller_file.clone());
                frontier.insert(e.callee_file.clone());
            }
            // BFS up to depth_cap hops away from the frontier files.
            for _ in 0..depth_cap {
                let mut next = frontier.clone();
                for e in cg.edges.iter() {
                    if frontier.contains(&e.caller_file) {
                        next.insert(e.callee_file.clone());
                    }
                    if frontier.contains(&e.callee_file) {
                        next.insert(e.caller_file.clone());
                    }
                }
                if next.len() == frontier.len() {
                    break;
                }
                frontier = next;
            }
            cg.edges
                .iter()
                .filter(|e| frontier.contains(&e.caller_file) || frontier.contains(&e.callee_file))
                .collect()
        } else {
            cg.edges.iter().collect()
        };

    let edges: Vec<&crate::intelligence::call_graph::CallEdge> =
        if let Some(focus) = effective_focus {
            filtered_edges
                .into_iter()
                .filter(|e| e.caller_file.starts_with(focus) || e.callee_file.starts_with(focus))
                .collect()
        } else {
            filtered_edges
        };

    Ok(Json(json!({
        "edges": edges,
        "unresolved": cg.unresolved,
        "total_edges": cg.edges.len(),
    })))
}

#[derive(Deserialize)]
struct DeadCodeParams {
    focus: Option<String>,
    limit: Option<usize>,
    /// MCP-style workspace alias for `focus`.  Cross-transport parity:
    /// `cxpak_dead_code` MCP tool accepts `workspace` as a workspace
    /// prefix; v1 honours the same name.
    workspace: Option<String>,
}

async fn dead_code_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<DeadCodeParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let limit = params.limit.unwrap_or(50);
    let focus = params.focus.as_deref().or(params.workspace.as_deref());

    let dead = crate::intelligence::dead_code::detect_dead_code(&idx, focus);
    let total_count = dead.len();
    let limited: Vec<_> = dead.into_iter().take(limit).collect();
    let showing = limited.len();

    Ok(Json(json!({
        "dead_symbols": limited,
        "total_count": total_count,
        "showing": showing,
    })))
}

#[derive(Deserialize)]
struct ArchitectureParams {
    focus: Option<String>,
    /// MCP-style workspace alias for `focus`.
    workspace: Option<String>,
}

async fn architecture_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<ArchitectureParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let map = crate::intelligence::architecture::build_architecture_map(&idx, 2);
    let effective_focus = params.focus.as_deref().or(params.workspace.as_deref());

    let modules = if let Some(focus) = effective_focus {
        map.modules
            .into_iter()
            .filter(|m| m.prefix.starts_with(focus))
            .collect::<Vec<_>>()
    } else {
        map.modules
    };

    Ok(Json(json!({
        "modules": modules,
        "circular_deps": map.circular_deps,
    })))
}

// --- v1.4.0 handlers: predict, drift, security_surface ---

#[derive(Deserialize)]
struct PredictParams {
    files: Option<Vec<String>>,
    /// Restrict predictions to test files whose path starts with this
    /// prefix.  Lets a CI integration ask "which tests in `tests/auth/`
    /// should I run for these changes" without parsing the full output.
    focus: Option<String>,
    depth: Option<usize>,
}

async fn predict_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<PredictParams>,
) -> Result<Json<Value>, StatusCode> {
    let files = match params.files {
        Some(f) if !f.is_empty() => f,
        _ => {
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    let depth = params.depth.unwrap_or(3);

    let mut result = crate::intelligence::predict::predict(
        &file_refs,
        &idx.graph,
        &idx.pagerank,
        &idx.co_changes,
        &idx.test_map,
        depth,
    );
    if let Some(prefix) = params.focus.as_deref() {
        result
            .test_impact
            .retain(|t| t.test_file.starts_with(prefix));
    }

    Ok(Json(serde_json::to_value(&result).unwrap_or_else(
        |_| json!({"error": "serialization failed"}),
    )))
}

#[derive(Deserialize)]
struct DriftParams {
    save_baseline: Option<bool>,
    /// Restrict the drift report's `hotspots` to module prefixes
    /// matching this filter.  Useful for per-team drift dashboards.
    focus: Option<String>,
}

async fn drift_handler(
    State(index): State<SharedIndex>,
    State(repo_path): State<SharedPath>,
    Json(params): Json<DriftParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let save_baseline = params.save_baseline.unwrap_or(false);
    let mut report =
        crate::intelligence::drift::build_drift_report(&idx, &repo_path, save_baseline);
    if let Some(prefix) = params.focus.as_deref() {
        report.hotspots.retain(|h| h.module.starts_with(prefix));
    }
    Ok(Json(serde_json::to_value(&report).unwrap_or_else(
        |_| json!({"error": "serialization failed"}),
    )))
}

#[derive(Deserialize)]
struct SecuritySurfaceParams {
    focus: Option<String>,
}

async fn security_surface_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<SecuritySurfaceParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let surface = crate::intelligence::security::build_security_surface(
        &idx,
        crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
        params.focus.as_deref(),
    );
    Ok(Json(serde_json::to_value(&surface).unwrap_or_else(
        |_| json!({"error": "serialization failed"}),
    )))
}

// v1.5.0: data flow and cross-language HTTP handlers

#[derive(Deserialize)]
struct DataFlowParams {
    symbol: Option<String>,
    sink: Option<String>,
    depth: Option<usize>,
    /// Restrict result paths to those whose origin or sink file starts
    /// with this prefix. Lets per-area data-flow audits skip irrelevant
    /// paths in the response without re-tracing.
    focus: Option<String>,
}

async fn data_flow_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<DataFlowParams>,
) -> Result<Json<Value>, StatusCode> {
    let Some(symbol) = params.symbol.filter(|s| !s.is_empty()) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let depth = params
        .depth
        .unwrap_or(crate::intelligence::data_flow::MAX_DEPTH)
        .min(crate::intelligence::data_flow::MAX_DEPTH);
    let mut result = crate::intelligence::data_flow::trace_data_flow(
        &symbol,
        params.sink.as_deref(),
        depth,
        &idx,
    );
    if let Some(prefix) = params.focus.as_deref() {
        // Keep only paths whose source OR sink file lives under the focus
        // prefix.  An empty result is meaningful — the caller asked for
        // a specific area and got nothing.
        result
            .paths
            .retain(|p| p.nodes.iter().any(|n| n.file.starts_with(prefix)));
    }
    Ok(Json(serde_json::to_value(&result).unwrap_or_else(
        |_| json!({"error": "serialization failed"}),
    )))
}

#[derive(Deserialize)]
struct CrossLangParams {
    file: Option<String>,
    focus: Option<String>,
}

async fn cross_lang_handler(
    State(index): State<SharedIndex>,
    Query(params): Query<CrossLangParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index
        .read()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let filtered: Vec<&crate::intelligence::cross_lang::CrossLangEdge> = idx
        .cross_lang_edges
        .iter()
        .filter(|e| match &params.file {
            Some(f) => &e.source_file == f || &e.target_file == f,
            None => true,
        })
        .filter(|e| match &params.focus {
            Some(p) => e.source_file.starts_with(p) || e.target_file.starts_with(p),
            None => true,
        })
        .collect();
    Ok(Json(json!({
        "edges": filtered,
        "total": filtered.len(),
    })))
}

// --- MCP server mode (JSON-RPC over stdio) ---

pub fn run_mcp(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Non-blocking startup (Task R0, ADR-0185): the full index build is 13–34s
    // on a large repo, which straddles Claude Code's ~30s MCP timeout when done
    // synchronously before the `initialize` handshake. Instead, publish a
    // `Building` readiness cell, kick the build onto a background thread (it
    // loads the ADR-0167 derived cache on a fingerprint hit), and enter the
    // stdio loop straight away so `initialize`/`tools/list` answer instantly.
    let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
    eprintln!(
        "cxpak: MCP server accepting connections; indexing {} in background",
        path.display()
    );
    let build_handle = spawn_mcp_index_build(path, Arc::clone(&readiness));

    let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
    let result = mcp_stdio_loop(path, &readiness, &snapshot);

    // Reclaim the background thread before returning (stdin closed / EOF).
    // Best-effort: a panicked build thread must not turn a clean shutdown into
    // an error, so the join result is intentionally discarded.
    let _ = build_handle.join();
    result
}

/// Map the outcome of a [`std::panic::catch_unwind`]-wrapped [`build_index`]
/// call to the appropriate [`IndexReadiness`] variant (Task R0 panic fix).
///
/// Extracted as a pure function so all three branches — success, build `Err`,
/// and panic — can be unit-tested without spawning threads or triggering real
/// panics in the test harness.
///
/// `path` is used only for the stderr diagnostic in the error/panic branches.
fn classify_build_outcome(
    path: &Path,
    outcome: Result<
        Result<CodebaseIndex, Box<dyn std::error::Error>>,
        Box<dyn std::any::Any + Send>,
    >,
) -> IndexReadiness {
    match outcome {
        Ok(Ok(idx)) => {
            eprintln!(
                "cxpak: MCP index ready ({} files indexed, {} tokens)",
                idx.total_files, idx.total_tokens
            );
            IndexReadiness::Ready(Arc::new(idx))
        }
        Ok(Err(e)) => {
            eprintln!("cxpak: warning: could not index {}: {e}", path.display());
            IndexReadiness::Failed(e.to_string())
        }
        Err(panic_payload) => {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic_payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic payload");
            eprintln!(
                "cxpak: warning: could not index {}: indexing panicked: {msg}",
                path.display()
            );
            IndexReadiness::Failed(format!("indexing panicked: {msg}"))
        }
    }
}

/// Spawn the background base-index build for the MCP server (Task R0).
///
/// Runs [`build_index`] off the handshake path and publishes the outcome into
/// `readiness`. Snapshot-then-swap lock discipline: the long build runs in a
/// thread-local `next`, and only the O(1) enum publish holds the write lock — so
/// a concurrent `tools/call` never blocks on (nor is blocked by) the build. A
/// build error **or panic** is captured as [`IndexReadiness::Failed`] via
/// [`std::panic::catch_unwind`] + [`classify_build_outcome`] so the cell never
/// stays `Building` forever — every tool call surfaces a clear status either way.
///
/// Extension point (Phase R-E1): after publishing `Ready`, a second background
/// phase will enrich the *same* cell with embeddings (an added readiness state)
/// without touching the handshake path or this function's contract.
#[doc(hidden)]
pub fn spawn_mcp_index_build(
    path: &Path,
    readiness: SharedReadiness,
) -> std::thread::JoinHandle<()> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_index(&path)));
        let next = classify_build_outcome(&path, outcome);
        // Capture the ready base *before* the move-consuming publish so phase 2
        // can enrich it without re-reading the cell. Only `Ready` is enrichable.
        #[cfg(feature = "embeddings")]
        let ready_base = match &next {
            IndexReadiness::Ready(idx) => Some(Arc::clone(idx)),
            _ => None,
        };
        // Brief write lock: `next` is already built, so this is an O(1) swap.
        // Recover from a poisoned lock (a reader panicked) and still publish, so
        // tool calls stop reporting "Building" once the build has completed.
        match readiness.write() {
            Ok(mut g) => *g = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
        // Phase 2 (R-E1, ADR-0186): once the base index is published `Ready`,
        // build the embedding index in this same background thread — off the
        // handshake, off the base-ready path — and swap in `ReadyEnriched`. This
        // is opt-in (only when `.cxpak.json` declares an `"embeddings"` section)
        // and strictly non-fatal: any failure or panic leaves the already-ready
        // base untouched (6-signal), never `Failed`, never a hang.
        #[cfg(feature = "embeddings")]
        if let Some(base) = ready_base {
            enrich_ready_with_embeddings(&readiness, &base, &path);
        }
    })
}

/// Phase 2 of the MCP background build (R-E1, ADR-0186): build the embedding
/// index for an already-`Ready` base and publish `ReadyEnriched`.
///
/// Opt-in gate: returns early (leaving the base `Ready`, 6-signal) unless
/// `.cxpak.json` declares an `"embeddings"` section — so the default no-config
/// path never downloads the MiniLM model and stays byte-identical.
///
/// Runs `build_embedding_index` under [`std::panic::catch_unwind`] so a
/// panicking provider or model loader can neither wedge the server nor downgrade
/// the ready base — it just leaves the index un-enriched. The relevance mode is
/// the current [`DEFAULT_RELEVANCE_MODE`][crate::relevance::DEFAULT_RELEVANCE_MODE]
/// (presently `Inert`), *not* a hard-coded `Active`; the Inert→Active flip is a
/// later task (R-D1) and embeddings are rebuilt on each serve start, so no stale
/// embedding survives that flip.
#[cfg(feature = "embeddings")]
fn enrich_ready_with_embeddings(
    readiness: &SharedReadiness,
    base: &Arc<CodebaseIndex>,
    path: &Path,
) {
    // Opt-in gate — no `.cxpak.json` embeddings section ⇒ no enrichment, no
    // model download, no network. Base stays `Ready`; golden byte-identical.
    if crate::embeddings::EmbeddingConfig::from_repo_root_if_configured(path).is_none() {
        return;
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::index::build_embedding_index(base, path, crate::relevance::DEFAULT_RELEVANCE_MODE)
    }));
    match outcome {
        Ok(Some(emb)) => publish_ready_enriched(readiness, base, emb),
        Ok(None) => {
            eprintln!(
                "cxpak: embedding index unavailable (provider/network/model) — \
                 serving on 6 signals"
            );
        }
        Err(_) => {
            eprintln!("cxpak: embedding build panicked — serving on 6 signals");
        }
    }
}

/// Swap an enriched `CodebaseIndex` (base + embedding index) into the readiness
/// cell as [`IndexReadiness::ReadyEnriched`] (R-E1).
///
/// The enriched index is built into a local *before* the lock is taken, so the
/// write lock is held only for the O(1) `Arc` swap — no lock is held across the
/// clone/build. A `tools/call` racing this swap snapshots either the pre-swap
/// `Ready` (6-signal) or the `ReadyEnriched` (7-signal), never a torn state.
/// Poison recovery matches R0's discipline: a panicked reader cannot wedge the
/// publish.
#[cfg(feature = "embeddings")]
fn publish_ready_enriched(
    readiness: &SharedReadiness,
    base: &Arc<CodebaseIndex>,
    emb: crate::embeddings::EmbeddingIndex,
) {
    let mut enriched = (**base).clone();
    enriched.embedding_index = Some(emb);
    let next = IndexReadiness::ReadyEnriched(Arc::new(enriched));
    match readiness.write() {
        Ok(mut g) => *g = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

/// Snapshot the base index if the background build has finished, else return the
/// human-readable status to surface as a tool `result` (Task R0).
///
/// Cloning the `Ready` `Arc` under a brief read lock is O(1); the lock is
/// released before the (possibly long-running) tool handler runs, so tool calls
/// never hold the readiness lock across a handler and never starve the
/// background publish. A poisoned lock is recovered rather than propagated so a
/// panicked reader cannot wedge the server.
fn snapshot_ready_index(readiness: &SharedReadiness) -> Result<Arc<CodebaseIndex>, String> {
    let guard = match readiness.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    match &*guard {
        // `ReadyEnriched` (R-E1) serves exactly like `Ready`; the attached
        // embedding index rides on the snapshotted `CodebaseIndex`.
        IndexReadiness::Ready(idx) | IndexReadiness::ReadyEnriched(idx) => Ok(Arc::clone(idx)),
        IndexReadiness::Building => Err(INDEXING_IN_PROGRESS_MESSAGE.to_string()),
        IndexReadiness::Failed(msg) => Err(format!(
            "cxpak: indexing failed and no context is available: {msg}. \
             Check the cxpak server logs, then restart the MCP server to retry."
        )),
    }
}

/// Run the MCP stdio loop against a background-built index (Task R0).
///
/// The index is published asynchronously into `readiness` by
/// [`spawn_mcp_index_build`]; the loop itself never blocks on the build.
/// Connections are typically short-lived (one task ≈ one connection); a
/// long-lived session would keep the first ready index (no periodic rebuild).
fn mcp_stdio_loop(
    repo_path: &Path,
    readiness: &SharedReadiness,
    snapshot: &SharedSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    mcp_stdio_loop_readiness(
        repo_path,
        readiness,
        snapshot,
        stdin.lock(),
        &mut stdout.lock(),
    )
}

/// Run the MCP JSON-RPC loop reading newline-delimited requests from
/// `reader` and writing newline-delimited responses to `writer`.
///
/// Public-with-`#[doc(hidden)]` so integration tests can drive the real
/// stdio framing path without spawning a subprocess.  Closes the gap the
/// prior round catalogued (in-process MCP tests bypass framing entirely).
#[doc(hidden)]
pub fn mcp_stdio_loop_with_io(
    repo_path: &Path,
    index: &CodebaseIndex,
    snapshot: &SharedSnapshot,
    reader: impl std::io::BufRead,
    writer: &mut impl std::io::Write,
) -> Result<(), Box<dyn std::error::Error>> {
    // Compatibility + test entry point: the caller already holds a fully built
    // index, so publish it as `Ready` and drive the readiness-gated loop. The
    // lazy/background path (`run_mcp`) constructs a `Building` cell instead and
    // lets `spawn_mcp_index_build` publish `Ready` when the build finishes.
    let readiness: SharedReadiness =
        Arc::new(RwLock::new(IndexReadiness::Ready(Arc::new(index.clone()))));
    mcp_stdio_loop_readiness(repo_path, &readiness, snapshot, reader, writer)
}

/// Readiness-gated MCP JSON-RPC loop (Task R0). Identical framing and dispatch
/// to the pre-R0 loop, except `tools/call` first snapshots `readiness`:
/// `initialize`, `tools/list`, and notifications always answer instantly
/// regardless of index state, while a tool call before the base index is ready
/// returns the graceful "indexing"/"failed" status via [`snapshot_ready_index`].
#[doc(hidden)]
pub fn mcp_stdio_loop_readiness(
    repo_path: &Path,
    readiness: &SharedReadiness,
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
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {e}")}
                });
                serde_json::to_writer(&mut *writer, &err)?;
                writer.write_all(b"\n")?;
                writer.flush()?;
                continue;
            }
        };

        // Reject JSON-RPC batch requests (arrays): not supported per MCP spec.
        if request.is_array() {
            let err = json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32600, "message": "Batch requests are not supported"}
            });
            serde_json::to_writer(&mut *writer, &err)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            continue;
        }

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
                    // C3 (ADR-0182): the live MCP surface is the capability
                    // catalog's ≤8 intent-tool projection. The 26 hand-rolled
                    // tool schemas were removed; every former tool is now an
                    // `op` under one of the five `cxpak_<intent>` tools.
                    "tools": crate::capability::adapter::mcp_tool_schemas()
                }),
            ),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                // Gate on background-build readiness (Task R0): once ready, run
                // against the base index exactly as before; before ready, return
                // a graceful tool `result` (retry/failed status), never an error.
                match snapshot_ready_index(readiness) {
                    Ok(idx) => {
                        handle_tool_call(id, tool_name, &arguments, &idx, repo_path, snapshot)
                    }
                    Err(status) => mcp_tool_result(id, &status),
                }
            }
            _ => mcp_error_response(id, -32601, "Method not found"),
        };

        serde_json::to_writer(&mut *writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }

    Ok(())
}

/// MCP `tools/call` entry point — the public router (C3, ADR-0182).
///
/// The advertised MCP surface is the five `cxpak_<intent>` tools projected from
/// the capability catalog (`tools/list` = [`crate::capability::adapter::mcp_tool_schemas`]);
/// a capability is selected by the `op` argument. For backward compatibility the
/// 26 removed tool NAMES are also accepted as deprecated aliases — they are not
/// discoverable (absent from `tools/list`) but still route to the same
/// capability core (removed in a future release; see `docs/MIGRATION-3.0.md`).
///
/// Public-with-`#[doc(hidden)]` so integration tests in
/// `tests/cross_channel_consistency.rs` can drive the MCP channel directly and
/// assert byte-identical parity with v1 / LSP / SPA. Not a stable public API.
#[doc(hidden)]
pub fn handle_tool_call(
    id: Option<Value>,
    tool_name: &str,
    args: &Value,
    index: &CodebaseIndex,
    repo_path: &Path,
    snapshot: &SharedSnapshot,
) -> Value {
    let op = match resolve_capability_op(tool_name, args) {
        Ok(op) => op,
        Err(OpResolution::InvalidOp(msg)) => return mcp_tool_result(id, &msg),
        Err(OpResolution::UnknownTool) => {
            return mcp_error_response(id, -32601, &format!("Unknown tool: {tool_name}"))
        }
    };
    dispatch_capability_op(id, &op, args, index, repo_path, snapshot)
}

/// Outcome of mapping a `tools/call` name (+ args) to a capability op.
enum OpResolution {
    /// The name is a known intent-tool but the `op` arg is missing/not hosted.
    InvalidOp(String),
    /// The name is neither an intent-tool nor a known legacy alias.
    UnknownTool,
}

/// Resolve a `tools/call` name to a capability `op` id.
///
/// * An intent-tool (`cxpak_context`/`cxpak_graph`/`cxpak_data`/`cxpak_review`/
///   `cxpak_insight`) requires an `op` arg the catalog hosts under it.
/// * A legacy `cxpak_*` tool name maps to its capability id (deprecated alias).
fn resolve_capability_op(tool_name: &str, args: &Value) -> Result<String, OpResolution> {
    if let Some(ops) = intent_tool_ops(tool_name) {
        return match args.get("op").and_then(|v| v.as_str()) {
            Some(op) if ops.iter().any(|o| o == op) => Ok(op.to_string()),
            Some(op) => Err(OpResolution::InvalidOp(format!(
                "Error: op '{op}' is not valid for {tool_name}. Valid ops: {}",
                ops.join(", ")
            ))),
            None => Err(OpResolution::InvalidOp(format!(
                "Error: '{tool_name}' requires an 'op' argument. Valid ops: {}",
                ops.join(", ")
            ))),
        };
    }
    match legacy_alias_to_op(tool_name) {
        Some(op) => Ok(op.to_string()),
        None => Err(OpResolution::UnknownTool),
    }
}

/// The capability ops hosted by an intent-tool, from the catalog adapter, or
/// `None` if `tool_name` is not one of the five intent-tools. Deterministic.
fn intent_tool_ops(tool_name: &str) -> Option<Vec<String>> {
    crate::capability::adapter::mcp_catalog_tools()
        .into_iter()
        .find(|t| t.name == tool_name)
        .map(|t| t.ops)
}

/// Map a legacy `cxpak_*` MCP tool name to its capability `op` id (C3 deprecated
/// alias). Only the 26 former tool names resolve; anything else is `None`.
fn legacy_alias_to_op(tool_name: &str) -> Option<&'static str> {
    Some(match tool_name {
        "cxpak_auto_context" => "context",
        "cxpak_context_diff" => "review",
        "cxpak_overview" => "overview",
        "cxpak_trace" => "trace",
        "cxpak_diff" => "diff",
        "cxpak_stats" => "stats",
        "cxpak_context_for_task" => "context_for_task",
        "cxpak_pack_context" => "pack_context",
        "cxpak_search" => "search",
        "cxpak_blast_radius" => "blast_radius",
        "cxpak_api_surface" => "api_surface",
        "cxpak_verify" => "verify",
        "cxpak_conventions" => "conventions",
        "cxpak_health" => "health",
        "cxpak_risks" => "risks",
        "cxpak_briefing" => "briefing",
        "cxpak_call_graph" => "call_graph",
        "cxpak_dead_code" => "dead_code",
        "cxpak_architecture" => "architecture",
        "cxpak_predict" => "predict",
        "cxpak_drift" => "drift",
        "cxpak_security_surface" => "security_surface",
        "cxpak_data_flow" => "data_flow",
        "cxpak_cross_lang" => "cross_lang",
        "cxpak_visual" => "visual",
        "cxpak_onboard" => "onboard",
        _ => return None,
    })
}

/// Build a deterministic JSON summary of the indexed data layer (`SchemaIndex`)
/// for the `data` capability (C3 / B1 M2). Tables, views, ORM models and
/// migration chains are emitted in sorted order so the output is byte-stable.
fn build_data_summary(index: &CodebaseIndex, focus: Option<&str>) -> Value {
    let in_focus = |path: &str| matches_focus(path, focus);
    let schema = match index.schema.as_ref() {
        None => {
            return json!({
                "indexed": false,
                "tables": [],
                "views": [],
                "orm_models": [],
                "migrations": [],
            })
        }
        Some(s) => s,
    };

    let mut tables: Vec<&crate::schema::TableSchema> = schema
        .tables
        .values()
        .filter(|t| in_focus(&t.file_path))
        .collect();
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    let tables_json: Vec<Value> = tables
        .iter()
        .map(|t| {
            let columns: Vec<Value> = t
                .columns
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "data_type": c.data_type,
                        "nullable": c.nullable,
                        "foreign_key": c.foreign_key.as_ref().map(|fk| {
                            json!({"target_table": fk.target_table, "target_column": fk.target_column})
                        }),
                    })
                })
                .collect();
            json!({
                "name": t.name,
                "file_path": t.file_path,
                "primary_key": t.primary_key,
                "columns": columns,
            })
        })
        .collect();

    let mut views: Vec<&crate::schema::ViewSchema> = schema
        .views
        .values()
        .filter(|v| in_focus(&v.file_path))
        .collect();
    views.sort_by(|a, b| a.name.cmp(&b.name));
    let views_json: Vec<Value> = views
        .iter()
        .map(
            |v| json!({"name": v.name, "file_path": v.file_path, "source_tables": v.source_tables}),
        )
        .collect();

    let mut models: Vec<&crate::schema::OrmModelSchema> = schema
        .orm_models
        .values()
        .filter(|m| in_focus(&m.file_path))
        .collect();
    models.sort_by(|a, b| {
        a.class_name
            .cmp(&b.class_name)
            .then_with(|| a.file_path.cmp(&b.file_path))
    });
    let models_json: Vec<Value> = models
        .iter()
        .map(|m| {
            json!({
                "class_name": m.class_name,
                "table_name": m.table_name,
                "file_path": m.file_path,
                "fields": m.fields.len(),
            })
        })
        .collect();

    let mut migrations: Vec<Value> = schema
        .migrations
        .iter()
        .filter(|chain| in_focus(&chain.directory))
        .map(|chain| json!({"directory": chain.directory, "steps": chain.migrations.len()}))
        .collect();
    migrations.sort_by(|a, b| {
        a["directory"]
            .as_str()
            .unwrap_or("")
            .cmp(b["directory"].as_str().unwrap_or(""))
    });

    json!({
        "indexed": true,
        "tables": tables_json,
        "views": views_json,
        "orm_models": models_json,
        "migrations": migrations,
    })
}

/// Dispatch a resolved capability `op` to its core. Each arm below is the former
/// `cxpak_*` tool body, re-keyed to the catalog capability id (C3, ADR-0182);
/// `graph`/`retrieval`/`data` are the newly MCP-surfaced cores.
fn dispatch_capability_op(
    id: Option<Value>,
    op: &str,
    args: &Value,
    index: &CodebaseIndex,
    repo_path: &Path,
    snapshot: &SharedSnapshot,
) -> Value {
    match op {
        // ---- Newly MCP-surfaced cores (B1 graph, C1 retrieval, C3 data) -----
        "graph" => {
            // `graph_op` selects node/neighbors/path/subgraph — renamed so it
            // does not collide with the intent-tool `op` discriminator. The
            // neighbors/path/subgraph output carries per-edge `edge_type` +
            // `confidence` (`inferred`) — A3 (ADR-0175) edge confidence.
            let sub = args
                .get("graph_op")
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("node");
            match crate::intelligence::graph_query::execute(&index.graph, sub, args) {
                Ok(v) => mcp_tool_result(id, &serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(e) => mcp_tool_result(id, &format!("Error: {e}")),
            }
        }
        "retrieval" => {
            // `retrieval_op` selects search|references|expand (C1, ADR-0180).
            let sub = args
                .get("retrieval_op")
                .and_then(|v| v.as_str())
                .unwrap_or("search");
            match crate::intelligence::retrieval::execute(index, sub, args) {
                Ok(v) => mcp_tool_result(id, &serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(e) => mcp_tool_result(id, &format!("Error: {e}")),
            }
        }
        "data" => {
            let result = build_data_summary(index, args.get("focus").and_then(|f| f.as_str()));
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "context" => {
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
            let cost_model = args
                .get("cost_model")
                .and_then(|v| v.as_str())
                .map(String::from);
            let opts = crate::auto_context::AutoContextOpts {
                cost_model,
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
        "review" => {
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
        "stats" => {
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
        "overview" => {
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
        "trace" => {
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
        "diff" => {
            let git_ref = args.get("git_ref").and_then(|r| r.as_str());
            let focus = args.get("focus").and_then(|f| f.as_str());
            let review = args
                .get("review")
                .and_then(|r| r.as_bool())
                .unwrap_or(false);
            let token_budget = args
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

                    let counter = TokenCounter::new();
                    let mut total_tokens = 0usize;
                    let mut files: Vec<Value> = Vec::new();

                    for c in &filtered {
                        let diff_tokens = counter.count(&c.diff_text);
                        if total_tokens + diff_tokens > token_budget {
                            // Budget exhausted — truncate remaining entries
                            break;
                        }
                        total_tokens += diff_tokens;
                        files.push(json!({
                            "path": c.path,
                            "diff": c.diff_text,
                        }));
                    }

                    let truncated = files.len() < filtered.len();

                    let mut result = json!({
                        "git_ref": git_ref.unwrap_or("working tree"),
                        "changed_files": filtered.len(),
                        "files_shown": files.len(),
                        "total_tokens": total_tokens,
                        "token_budget": token_budget,
                        "truncated": truncated,
                        "files": files,
                    });
                    if let Some(f) = focus {
                        result["focus"] = json!(f);
                    }

                    // --review: attach the structured change-impact bundle
                    // (blast radius, impacted tests, convention + security
                    // deltas, and expected-but-absent changes). Self-contained:
                    // build_review_bundle recomputes pagerank/test_map locally.
                    if review {
                        match crate::commands::diff::build_review_bundle(index, repo_path, git_ref)
                        {
                            Ok(bundle) => {
                                result["review"] =
                                    serde_json::to_value(&bundle).unwrap_or(Value::Null);
                            }
                            Err(e) => {
                                result["review_error"] = json!(e);
                            }
                        }
                    }

                    mcp_tool_result(
                        id,
                        &serde_json::to_string_pretty(&result).unwrap_or_default(),
                    )
                }
                Err(e) => mcp_tool_result(id, &format!("Error: {e}")),
            }
        }
        "context_for_task" => {
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
        "pack_context" => {
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
            // the file that pulled each dependency in, the edge type used, and
            // whether that edge was structurally extracted or heuristically
            // inferred (so the annotation can flag inferred dependencies).
            let mut target_files: Vec<AutoContextTarget> = vec![];
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
                    target_files.push((
                        path.clone(),
                        FileRole::Selected,
                        None,
                        None,
                        EdgeConfidence::Extracted,
                    ));
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
                                    dep.confidence,
                                ));
                            }
                        }
                    }
                }
            }

            // Auto-include test files for selected source files.
            if include_tests {
                let test_additions: Vec<AutoContextTarget> = target_files
                    .iter()
                    .filter(|(_, role, _, _, _)| matches!(role, FileRole::Selected))
                    .filter_map(|(path, _, _, _, _)| {
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
                                            EdgeConfidence::Extracted,
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
                EdgeConfidence,
            );
            let mut not_found: Vec<Value> = vec![];
            let mut indexed_targets: Vec<PackTarget<'_>> = vec![];

            for (path, role, parent, edge_type, confidence) in &target_files {
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
                            *confidence,
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
                .map(|(f, role, score, _, _, _)| (*f, *role, *score))
                .collect();
            let allocated =
                allocate_with_degradation(&alloc_inputs, token_budget, Some(&index.pagerank));

            // Render annotated output per file.
            let mut packed: Vec<Value> = vec![];
            let mut total_tokens = 0usize;

            for (alloc, (indexed_file, role, _score, parent, edge_type, confidence)) in
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

                // Build the annotation parent string: for non-Import edges,
                // append the edge type and flag heuristically inferred edges.
                let annotation_parent = parent.as_ref().map(|p| match edge_type {
                    Some(et) => format_dependency_parent(p, et, *confidence),
                    None => p.clone(),
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
        "search" => {
            let pattern = args.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
            if pattern.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'pattern' argument is required and must not be empty",
                );
            }
            if pattern.len() > MAX_PATTERN_LEN {
                return mcp_tool_result(
                    id,
                    &format!(
                        "Error: pattern exceeds maximum length of {MAX_PATTERN_LEN} characters"
                    ),
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
        "blast_radius" => {
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
        "api_surface" => {
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
        "verify" => {
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
        "conventions" => {
            let category = args
                .get("category")
                .and_then(|c| c.as_str())
                .unwrap_or("all");
            let strength_filter = args
                .get("strength")
                .and_then(|s| s.as_str())
                .unwrap_or("all");
            let focus = args.get("focus").and_then(|f| f.as_str());
            let token_budget = args
                .get("tokens")
                .and_then(|t| t.as_str())
                .and_then(|t| crate::cli::parse_token_count(t).ok())
                .or_else(|| {
                    args.get("tokens")
                        .and_then(|t| t.as_u64())
                        .map(|n| n as usize)
                })
                .unwrap_or(crate::conventions::render::MAX_MCP_CONVENTIONS_TOKENS);

            let profile = &index.conventions;

            let mut result = match category {
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

            // Apply strength filter: remove observations whose strength is below threshold.
            // Valid values: "convention" (≥90%), "trend" (≥70%), "mixed" (≥50%), "all".
            let min_pct: f64 = match strength_filter {
                "convention" => 90.0,
                "trend" => 70.0,
                "mixed" => 50.0,
                _ => 0.0, // "all" — no filtering
            };
            if min_pct > 0.0 {
                filter_observations_by_strength(&mut result, min_pct);
            }

            // Apply focus filter: prune file_contributions (if present) to entries
            // whose path matches the prefix.
            if let Some(focus_prefix) = focus {
                filter_contributions_by_focus(&mut result, focus_prefix);
            }

            // Apply token budget AFTER category/strength/focus filters.
            // A narrow result already under budget returns in full with no omission marker.
            let result =
                crate::conventions::render::render_budgeted_conventions(result, token_budget);

            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "health" => {
            let health = crate::intelligence::health::compute_health(index);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&health).unwrap_or_default(),
            )
        }
        "risks" => {
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20) as usize;
            let focus = args.get("focus").and_then(|f| f.as_str());
            let mut all_risks = crate::intelligence::risk::compute_risk_ranking(index);
            if let Some(f) = focus {
                all_risks.retain(|r| r.path.starts_with(f));
            }
            let risks: Vec<&crate::intelligence::risk::RiskEntry> =
                all_risks.iter().take(limit).collect();
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&risks).unwrap_or_default(),
            )
        }
        "briefing" => {
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
            let cost_model = args
                .get("cost_model")
                .and_then(|v| v.as_str())
                .map(String::from);
            let opts = crate::auto_context::AutoContextOpts {
                cost_model,
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
        "call_graph" => {
            let target = args.get("target").and_then(|t| t.as_str());
            let focus = args.get("focus").and_then(|f| f.as_str());
            let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(1) as usize;
            let workspace = args.get("workspace").and_then(|w| w.as_str());
            let cg = &index.call_graph;

            // BFS from the seed files/symbols up to `depth` hops.
            // If no target is given, all edges are considered seeds (depth=1 returns all).
            let seed_edges: Vec<&crate::intelligence::call_graph::CallEdge> = cg
                .edges
                .iter()
                .filter(|e| {
                    if let Some(t) = target {
                        e.caller_file.contains(t)
                            || e.callee_file.contains(t)
                            || e.caller_symbol.contains(t)
                            || e.callee_symbol.contains(t)
                    } else {
                        true
                    }
                })
                .collect();

            // BFS: collect files reachable within `depth` hops from seed edges.
            let filtered: Vec<&crate::intelligence::call_graph::CallEdge> = if target.is_some() {
                let mut reachable: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let mut frontier: Vec<String> = seed_edges
                    .iter()
                    .flat_map(|e| [e.caller_file.clone(), e.callee_file.clone()])
                    .collect();
                for f in &frontier {
                    reachable.insert(f.clone());
                }
                for _ in 0..depth.saturating_sub(1) {
                    let next: Vec<String> = cg
                        .edges
                        .iter()
                        .filter(|e| {
                            reachable.contains(&e.caller_file) || reachable.contains(&e.callee_file)
                        })
                        .flat_map(|e| [e.caller_file.clone(), e.callee_file.clone()])
                        .filter(|f| !reachable.contains(f))
                        .collect();
                    if next.is_empty() {
                        break;
                    }
                    frontier = next;
                    for f in &frontier {
                        reachable.insert(f.clone());
                    }
                }
                cg.edges
                    .iter()
                    .filter(|e| {
                        reachable.contains(&e.caller_file) && reachable.contains(&e.callee_file)
                    })
                    .collect()
            } else {
                seed_edges
            };

            // Apply focus filter
            let filtered: Vec<&crate::intelligence::call_graph::CallEdge> = filtered
                .into_iter()
                .filter(|e| {
                    if let Some(f) = focus {
                        e.caller_file.starts_with(f) || e.callee_file.starts_with(f)
                    } else {
                        true
                    }
                })
                .collect();

            // Apply workspace filter: both caller and callee must be in workspace prefix
            let filtered: Vec<&crate::intelligence::call_graph::CallEdge> = filtered
                .into_iter()
                .filter(|e| {
                    if let Some(ws) = workspace {
                        e.caller_file.starts_with(ws) && e.callee_file.starts_with(ws)
                    } else {
                        true
                    }
                })
                .collect();

            let result = json!({
                "edges": filtered,
                "unresolved": cg.unresolved,
                "total_edges": cg.edges.len(),
            });
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "dead_code" => {
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;
            let focus = args.get("focus").and_then(|f| f.as_str());
            let workspace = args.get("workspace").and_then(|w| w.as_str());
            // workspace acts as a focus prefix when focus is not set
            let effective_focus = focus.or(workspace);
            // Use the shared cache when no focus is requested — matches the
            // LSP / dashboard / health-score paths and saves the full
            // O(F·S·C) re-scan that an unfocused MCP poll would otherwise
            // pay on every call.  When focus IS set, fall through to a
            // direct call so the focus filter is honoured.
            let dead: Vec<_> = match effective_focus {
                None => index.dead_code_cached().to_vec(),
                Some(f) => crate::intelligence::dead_code::detect_dead_code(index, Some(f)),
            };
            let total = dead.len();
            let limited: Vec<_> = dead.into_iter().take(limit).collect();
            let showing = limited.len();
            // Cross-channel parity: v1/dead_code, LSP cxpak/deadCode, and MCP
            // cxpak_dead_code all expose `total` and `dead_symbols`.  MCP
            // additionally reports `showing` so a client that passed `limit`
            // can detect truncation; v1/LSP are non-paginated.
            let result = json!({
                "dead_symbols": limited,
                "total": total,
                "showing": showing,
            });
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "architecture" => {
            let focus = args.get("focus").and_then(|f| f.as_str());
            let workspace = args.get("workspace").and_then(|w| w.as_str());
            // workspace acts as a module prefix filter when focus is not set
            let effective_prefix = focus.or(workspace);
            let map = crate::intelligence::architecture::build_architecture_map(index, 2);
            let modules: Vec<_> = if let Some(prefix) = effective_prefix {
                map.modules
                    .into_iter()
                    .filter(|m| m.prefix.starts_with(prefix))
                    .collect()
            } else {
                map.modules
            };
            let result = json!({
                "modules": modules,
                "circular_deps": map.circular_deps,
            });
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "predict" => {
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
            let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
            let depth = args.get("depth").and_then(|d| d.as_u64()).unwrap_or(3) as usize;
            let focus = args.get("focus").and_then(|f| f.as_str());
            let mut result = crate::intelligence::predict::predict(
                &file_refs,
                &index.graph,
                &index.pagerank,
                &index.co_changes,
                &index.test_map,
                depth,
            );
            // Apply focus filter: keep only impact entries whose path starts with prefix
            if let Some(focus_prefix) = focus {
                result
                    .structural_impact
                    .retain(|entry| entry.path.starts_with(focus_prefix));
                result
                    .historical_impact
                    .retain(|entry| entry.path.starts_with(focus_prefix));
                result
                    .call_impact
                    .retain(|entry| entry.path.starts_with(focus_prefix));
                result
                    .test_impact
                    .retain(|tp| tp.test_file.starts_with(focus_prefix));
            }
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "drift" => {
            let save_baseline = args
                .get("save_baseline")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let report =
                crate::intelligence::drift::build_drift_report(index, repo_path, save_baseline);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&report).unwrap_or_default(),
            )
        }
        "security_surface" => {
            let focus = args.get("focus").and_then(|f| f.as_str());
            let surface = crate::intelligence::security::build_security_surface(
                index,
                crate::intelligence::security::DEFAULT_AUTH_PATTERNS,
                focus,
            );
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&surface).unwrap_or_default(),
            )
        }
        "data_flow" => {
            let symbol = args.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
            if symbol.is_empty() {
                return mcp_tool_result(
                    id,
                    "Error: 'symbol' argument is required and must not be empty",
                );
            }
            let sink = args.get("sink").and_then(|s| s.as_str());
            let depth = args
                .get("depth")
                .and_then(|d| d.as_u64())
                .map(|d| d as usize)
                .unwrap_or(10)
                .min(crate::intelligence::data_flow::MAX_DEPTH);
            let result =
                crate::intelligence::data_flow::trace_data_flow(symbol, sink, depth, index);
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            )
        }
        "cross_lang" => {
            let file_filter = args.get("file").and_then(|s| s.as_str());
            let focus = args.get("focus").and_then(|s| s.as_str());
            let filtered: Vec<&crate::intelligence::cross_lang::CrossLangEdge> = index
                .cross_lang_edges
                .iter()
                .filter(|e| match file_filter {
                    Some(f) => e.source_file == f || e.target_file == f,
                    None => true,
                })
                .filter(|e| match focus {
                    Some(p) => e.source_file.starts_with(p) || e.target_file.starts_with(p),
                    None => true,
                })
                .collect();
            mcp_tool_result(
                id,
                &serde_json::to_string_pretty(&json!({
                    "edges": filtered,
                    "total": filtered.len(),
                }))
                .unwrap_or_default(),
            )
        }
        "visual" => {
            let visual_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("dashboard");
            let format = args
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("html");
            let focus = args.get("focus").and_then(|v| v.as_str());
            let symbol = args.get("symbol").and_then(|v| v.as_str());
            let files_arg = args.get("files").and_then(|v| v.as_str());

            // Validate parameter requirements before rendering.
            // Per MCP spec, tool parameter errors use mcp_tool_result with isError,
            // not the JSON-RPC error response used for protocol-level errors.
            if visual_type == "flow" && symbol.is_none() {
                return mcp_tool_result(id, "Error: symbol is required when type=flow");
            }
            if visual_type == "diff" && files_arg.is_none() {
                return mcp_tool_result(id, "Error: files is required when type=diff");
            }

            #[cfg(feature = "visual")]
            {
                use crate::visual::export;
                use crate::visual::layout::{self, LayoutConfig};
                use crate::visual::render::{self};

                let _ = focus; // focus reserved for future scoped rendering

                // Single source of truth — call commands::visual::make_metadata
                // directly so the SPA/standalone renderers and this MCP path
                // can never drift on any field (repo_name canonicalization,
                // health_score wiring, edge_count helper, version).  Pre-fix
                // this site was an inline copy of make_metadata's body, which
                // is exactly what hid the original `health_score: null` bug
                // (parity broke when one branch was updated and the other
                // wasn't).
                let metadata = crate::commands::visual::make_metadata(index, repo_path);
                let config = LayoutConfig::default();

                let html_result: Result<String, Box<dyn std::error::Error>> = match visual_type {
                    "architecture" => {
                        render::render_architecture_explorer(index, &metadata).map_err(|e| e.into())
                    }
                    "risk" => Ok(render::render_risk_heatmap(index, &metadata)),
                    "flow" => {
                        let sym = symbol.unwrap_or("main");
                        let flow_result =
                            crate::intelligence::data_flow::trace_data_flow(sym, None, 6, index);
                        render::render_flow_diagram(&flow_result, index, &metadata)
                            .map_err(|e| e.into())
                    }
                    "timeline" => {
                        let snapshots = crate::visual::timeline::load_cached_snapshots(repo_path)
                            .unwrap_or_default();
                        render::render_time_machine(snapshots, &metadata, &config)
                            .map_err(|e| e.into())
                    }
                    "diff" => {
                        let changed: Vec<String> = files_arg
                            .unwrap_or("")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        render::render_diff_view(index, &changed, &metadata, &config)
                            .map_err(|e| e.into())
                    }
                    _ => Ok(render::render_dashboard(index, &metadata)),
                };

                let html = match html_result {
                    Ok(h) => h,
                    Err(e) => return mcp_tool_result(id, &format!("Error: {e}")),
                };

                let content =
                    match format {
                        "mermaid" => {
                            let computed = layout::build_module_layout(index, &config)
                                .unwrap_or_else(|_| crate::visual::layout::ComputedLayout {
                                    nodes: vec![],
                                    edges: vec![],
                                    width: 0.0,
                                    height: 0.0,
                                    layers: vec![],
                                });
                            export::to_mermaid(&computed)
                        }
                        "svg" => {
                            let computed = layout::build_module_layout(index, &config)
                                .unwrap_or_else(|_| crate::visual::layout::ComputedLayout {
                                    nodes: vec![],
                                    edges: vec![],
                                    width: 0.0,
                                    height: 0.0,
                                    layers: vec![],
                                });
                            export::to_svg(&computed, &metadata)
                        }
                        "c4" => {
                            let computed = layout::build_module_layout(index, &config)
                                .unwrap_or_else(|_| crate::visual::layout::ComputedLayout {
                                    nodes: vec![],
                                    edges: vec![],
                                    width: 0.0,
                                    height: 0.0,
                                    layers: vec![],
                                });
                            export::to_c4(&computed, &metadata)
                        }
                        "json" => {
                            let computed = layout::build_module_layout(index, &config)
                                .unwrap_or_else(|_| crate::visual::layout::ComputedLayout {
                                    nodes: vec![],
                                    edges: vec![],
                                    width: 0.0,
                                    height: 0.0,
                                    layers: vec![],
                                });
                            export::to_json(&computed)
                        }
                        "cypher" => export::to_cypher(&index.graph, &metadata.repo_name),
                        "graphml" => export::to_graphml(&index.graph, &metadata.repo_name),
                        _ => html, // html is the default
                    };

                const MCP_INLINE_LIMIT: usize = 1_048_576; // 1 MB
                if format == "html" && content.len() > MCP_INLINE_LIMIT {
                    let validated_slug = match validate_visual_type_slug(visual_type) {
                        Ok(s) => s,
                        Err(e) => return mcp_tool_result(id, &format!("Error: {e}")),
                    };
                    let visual_dir = repo_path.join(".cxpak/visual");
                    std::fs::create_dir_all(&visual_dir).ok();
                    let filepath = visual_dir.join(format!("cxpak-{validated_slug}.html"));
                    let canon_dir = match visual_dir.canonicalize() {
                        Ok(d) => d,
                        Err(e) => return mcp_tool_result(id, &format!("canonicalize failed: {e}")),
                    };
                    let canon_file = match filepath.parent().unwrap().canonicalize() {
                        Ok(p) => p,
                        Err(e) => return mcp_tool_result(id, &format!("canonicalize failed: {e}")),
                    };
                    if !canon_file.starts_with(&canon_dir) {
                        return mcp_tool_result(id, "Error: path escape detected");
                    }
                    match std::fs::write(&filepath, &content) {
                        Ok(()) => mcp_tool_result(
                            id,
                            &format!(
                                "Output written to {} ({} bytes)",
                                filepath.display(),
                                content.len()
                            ),
                        ),
                        Err(e) => mcp_tool_result(id, &format!("Error writing output: {e}")),
                    }
                } else {
                    mcp_tool_result(id, &content)
                }
            }
            #[cfg(not(feature = "visual"))]
            {
                let _ = (visual_type, format, focus, symbol, files_arg);
                mcp_tool_result(
                    id,
                    "Error: cxpak_visual requires the 'visual' feature flag. Rebuild with: cargo build --features visual",
                )
            }
        }
        "onboard" => {
            let focus = args.get("focus").and_then(|v| v.as_str());
            let format = args
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("json");

            #[cfg(feature = "visual")]
            {
                let map = crate::visual::onboard::compute_onboarding_map(index, focus);
                let content = if format == "markdown" {
                    crate::visual::onboard::render_onboarding_markdown(&map)
                } else {
                    crate::visual::onboard::render_onboarding_json(&map)
                };
                mcp_tool_result(id, &content)
            }
            #[cfg(not(feature = "visual"))]
            {
                let _ = (focus, format);
                mcp_tool_result(
                    id,
                    "Error: cxpak_onboard requires the 'visual' feature flag. Rebuild with: cargo build --features visual",
                )
            }
        }
        // Unreachable in practice: `resolve_capability_op` only yields ops that
        // the catalog hosts (or the 26 legacy aliases), all of which have an arm
        // above. Kept as a defensive fallback.
        _ => mcp_error_response(id, -32601, &format!("Unknown capability op: {op}")),
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
///
/// Public so integration tests can drive the watcher invalidation contract
/// without owning a real `notify` watcher process.  Hidden from the rendered
/// docs because it's not part of the stable public API surface.
#[doc(hidden)]
pub fn process_watcher_changes(
    changes: &[crate::daemon::watcher::FileChange],
    base_path: &Path,
    shared: &SharedIndex,
) {
    let (modified_paths, removed_paths) = classify_changes(changes, base_path);

    // Snapshot-then-swap: clone the inner Arc under the read lock (O(1)),
    // drop the lock, build the new index off a deep clone of the snapshot,
    // then take the write lock briefly to swap in the new Arc.  Long-running
    // readers in flight continue to see the pre-swap snapshot — never
    // blocked, never returning torn state.
    let snapshot: Arc<CodebaseIndex> = match shared.read() {
        Ok(g) => Arc::clone(&*g),
        Err(_) => return, // poisoned lock — nothing to do
    };
    let mut next: CodebaseIndex = (*snapshot).clone();
    drop(snapshot);

    let update_count =
        apply_incremental_update(&mut next, base_path, &modified_paths, &removed_paths);
    if update_count == 0 {
        return;
    }
    // Edge-delta graph rebuild + warm-started PageRank (ADR-0165/0166): work
    // proportional to the change for the common content-edit case, falling back
    // to a full rebuild + cold start on structural (add/remove) or schema
    // changes.  The prior ranks seed the new iteration; PageRank's stationary
    // distribution is unique per graph, so this is bit-identical to a cold
    // recompute (proven by tests/parity.rs) while converging in fewer passes.
    let prior_pagerank = std::mem::take(&mut next.pagerank);
    next.rebuild_graph_delta(&modified_paths, &removed_paths);
    next.pagerank = crate::intelligence::pagerank::compute_pagerank_seeded(
        &next.graph,
        0.85,
        100,
        &prior_pagerank,
    );
    let paths: std::collections::HashSet<String> =
        next.files.iter().map(|f| f.relative_path.clone()).collect();
    next.test_map = crate::intelligence::test_map::build_test_map(&next.files, &paths);
    {
        let mod_vec: Vec<String> = modified_paths.iter().cloned().collect();
        let rem_vec: Vec<String> = removed_paths.iter().cloned().collect();
        let mut conventions = std::mem::take(&mut next.conventions);
        crate::conventions::update_conventions_incremental(
            &mut conventions,
            &mod_vec,
            &rem_vec,
            &next,
        );
        next.conventions = conventions;
    }
    // Fresh OnceLocks — we built a new CodebaseIndex; the prior caches
    // are bound to the prior state and would lie if carried forward.
    next.dead_code_cache = std::sync::Arc::new(std::sync::OnceLock::new());
    next.health_cache = std::sync::Arc::new(std::sync::OnceLock::new());

    let total_files = next.total_files;
    let total_tokens = next.total_tokens;
    let new_arc = Arc::new(next);
    if let Ok(mut g) = shared.write() {
        *g = new_arc;
    }
    eprintln!(
        "cxpak: updated {update_count} file(s), {total_files} files / {total_tokens} tokens total"
    );
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
        Arc::new(RwLock::new(Arc::new(make_test_index())))
    }

    fn make_shared_snapshot() -> SharedSnapshot {
        Arc::new(RwLock::new(None))
    }

    // --- R0 panic fix: classify_build_outcome covers all three arms ---

    /// Success arm: `Ok(Ok(idx))` → `Ready` carrying the built index.
    #[test]
    fn classify_build_outcome_ok_yields_ready() {
        let idx = make_test_index();
        let outcome: Result<
            Result<CodebaseIndex, Box<dyn std::error::Error>>,
            Box<dyn std::any::Any + Send>,
        > = Ok(Ok(idx));
        let result = classify_build_outcome(Path::new("/tmp"), outcome);
        assert!(matches!(result, IndexReadiness::Ready(_)));
    }

    /// Error arm: `Ok(Err(e))` → `Failed` carrying the error message.
    #[test]
    fn classify_build_outcome_err_yields_failed() {
        let e: Box<dyn std::error::Error> = "scanner failed".into();
        let outcome: Result<
            Result<CodebaseIndex, Box<dyn std::error::Error>>,
            Box<dyn std::any::Any + Send>,
        > = Ok(Err(e));
        let result = classify_build_outcome(Path::new("/tmp"), outcome);
        match result {
            IndexReadiness::Failed(msg) => assert!(msg.contains("scanner failed"), "msg: {msg}"),
            IndexReadiness::Building => panic!("expected Failed, got Building"),
            IndexReadiness::Ready(_) => panic!("expected Failed, got Ready"),
            IndexReadiness::ReadyEnriched(_) => panic!("expected Failed, got ReadyEnriched"),
        }
    }

    /// Panic arm: `Err(payload)` → `Failed("indexing panicked: …")`.
    ///
    /// This is the R0 fix: a panicking `build_index` call must never leave the
    /// shared readiness cell at `Building` forever. `classify_build_outcome`
    /// is the single place that converts a panic payload to `Failed`.
    #[test]
    fn classify_build_outcome_panic_yields_failed() {
        let panic_payload: Box<dyn std::any::Any + Send> = Box::new("tree-sitter exploded");
        let outcome: Result<
            Result<CodebaseIndex, Box<dyn std::error::Error>>,
            Box<dyn std::any::Any + Send>,
        > = Err(panic_payload);
        let result = classify_build_outcome(Path::new("/tmp"), outcome);
        match result {
            IndexReadiness::Failed(msg) => {
                assert!(msg.contains("indexing panicked"), "msg: {msg}");
                assert!(msg.contains("tree-sitter exploded"), "msg: {msg}");
            }
            IndexReadiness::Building => panic!("expected Failed, got Building"),
            IndexReadiness::Ready(_) => panic!("expected Failed, got Ready"),
            IndexReadiness::ReadyEnriched(_) => panic!("expected Failed, got ReadyEnriched"),
        }
    }

    /// Integration: a thread that panics during indexing publishes `Failed` into
    /// the shared cell — the cell must never stay `Building`.
    ///
    /// Exercises the full `catch_unwind` + `classify_build_outcome` + write-lock
    /// path that `spawn_mcp_index_build` uses in production.
    #[test]
    fn panicking_build_thread_publishes_failed_not_building() {
        let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
        let readiness_clone = Arc::clone(&readiness);

        let handle = std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<CodebaseIndex, Box<dyn std::error::Error>> {
                    panic!("deliberate panic in indexing thread");
                },
            ));
            let next = classify_build_outcome(Path::new("/tmp"), outcome);
            match readiness_clone.write() {
                Ok(mut g) => *g = next,
                Err(poisoned) => *poisoned.into_inner() = next,
            }
        });
        handle.join().expect("thread must not propagate the panic");

        let guard = readiness.read().unwrap();
        match &*guard {
            IndexReadiness::Failed(msg) => {
                assert!(msg.contains("indexing panicked"), "msg: {msg}");
                assert!(
                    msg.contains("deliberate panic in indexing thread"),
                    "msg: {msg}"
                );
            }
            IndexReadiness::Building => {
                panic!("cell must not stay Building after a panicking build")
            }
            IndexReadiness::Ready(_) => panic!("panicking build must not yield Ready"),
            IndexReadiness::ReadyEnriched(_) => {
                panic!("panicking build must not yield ReadyEnriched")
            }
        }
    }

    // --- R-E1: background embedding enrichment (phase 2, opt-in) ---

    /// `publish_ready_enriched` swaps the cell to `ReadyEnriched` carrying a
    /// `CodebaseIndex` whose `embedding_index` is now `Some`. `snapshot_ready_index`
    /// treats it exactly like `Ready` (returns the index), and the snapshot now
    /// reports `has_embedding_index()` — the signal-#7 activation this task wires.
    #[cfg(feature = "embeddings")]
    #[test]
    fn publish_ready_enriched_attaches_embedding_index() {
        let base = Arc::new(make_test_index());
        assert!(
            !base.has_embedding_index(),
            "base must start without an embedding index"
        );
        let readiness: SharedReadiness =
            Arc::new(RwLock::new(IndexReadiness::Ready(Arc::clone(&base))));

        // A tiny in-crate embedding index — no provider, no network, no model.
        let mut emb = crate::embeddings::EmbeddingIndex::new(3);
        emb.add("src/main.rs".to_string(), vec![0.1, 0.2, 0.3]);
        publish_ready_enriched(&readiness, &base, emb);

        // Cell is now ReadyEnriched, and snapshots (Ready|ReadyEnriched) carry it.
        assert!(matches!(
            &*readiness.read().unwrap(),
            IndexReadiness::ReadyEnriched(_)
        ));
        let snap = snapshot_ready_index(&readiness).expect("ReadyEnriched must yield the index");
        assert_eq!(snap.total_files, 2);
        assert!(
            snap.has_embedding_index(),
            "the enriched snapshot must expose the embedding index (signal #7)"
        );
    }

    /// Opt-in gate at the serve layer: a repo with **no** `.cxpak.json` leaves the
    /// base `Ready` (never `ReadyEnriched`), builds no embedding index, downloads
    /// no model, and hits no network. This keeps the default path byte-identical.
    #[cfg(feature = "embeddings")]
    #[test]
    fn enrich_skips_and_stays_ready_when_not_configured() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = Arc::new(make_test_index());
        let readiness: SharedReadiness =
            Arc::new(RwLock::new(IndexReadiness::Ready(Arc::clone(&base))));

        enrich_ready_with_embeddings(&readiness, &base, dir.path());

        assert!(
            matches!(&*readiness.read().unwrap(), IndexReadiness::Ready(_)),
            "no .cxpak.json ⇒ cell must stay Ready (6-signal), never ReadyEnriched"
        );
        let snap = snapshot_ready_index(&readiness).expect("ready");
        assert!(!snap.has_embedding_index());
    }

    /// Graceful fallback: a repo configured for a *remote* provider whose API-key
    /// env var is unset fails provider construction (no network) — `build_embedding_index`
    /// returns `None`, so the cell stays `Ready` (6-signal), never `Failed`, never
    /// a hang, and `has_embedding_index()` stays false.
    #[cfg(feature = "embeddings")]
    #[test]
    fn enrich_falls_back_gracefully_on_failing_provider() {
        let key_env = "CXPAK_TEST_UNSET_EMBED_KEY_R_E1";
        std::env::remove_var(key_env);
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".cxpak.json"),
            format!(r#"{{"embeddings": {{"provider": "openai", "api_key_env": "{key_env}"}}}}"#),
        )
        .unwrap();
        let base = Arc::new(make_test_index());
        let readiness: SharedReadiness =
            Arc::new(RwLock::new(IndexReadiness::Ready(Arc::clone(&base))));

        // Configured, but the provider cannot construct (unset key) — no network.
        enrich_ready_with_embeddings(&readiness, &base, dir.path());

        assert!(
            matches!(&*readiness.read().unwrap(), IndexReadiness::Ready(_)),
            "a failing provider must leave the base Ready (6-signal), not Failed"
        );
        let snap = snapshot_ready_index(&readiness).expect("base must still be queryable");
        assert!(!snap.has_embedding_index());
        assert_eq!(snap.total_files, 2);
    }

    /// Concurrency: while the phase-2 enrich swap runs on one thread, concurrent
    /// snapshots on another always observe a valid, queryable index — either the
    /// pre-swap `Ready` (6-signal) or the post-swap `ReadyEnriched` (7-signal) —
    /// never a torn or panicking state. After the swap lands, the cell is enriched.
    #[cfg(feature = "embeddings")]
    #[test]
    fn enrich_swap_never_races_base_ready() {
        let base = Arc::new(make_test_index());
        let readiness: SharedReadiness =
            Arc::new(RwLock::new(IndexReadiness::Ready(Arc::clone(&base))));

        let reader = {
            let readiness = Arc::clone(&readiness);
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    let idx = snapshot_ready_index(&readiness)
                        .expect("Ready|ReadyEnriched must always yield an index");
                    // Never torn: the file count is stable across the swap.
                    assert_eq!(idx.total_files, 2);
                }
            })
        };

        let mut emb = crate::embeddings::EmbeddingIndex::new(3);
        emb.add("src/main.rs".to_string(), vec![0.4, 0.5, 0.6]);
        publish_ready_enriched(&readiness, &base, emb);

        reader
            .join()
            .expect("reader must not observe a torn/panicking state");
        assert!(
            snapshot_ready_index(&readiness)
                .unwrap()
                .has_embedding_index(),
            "after the swap the cell must be enriched"
        );
    }

    // --- R0: non-blocking MCP startup / readiness gating ---

    /// `Ready` yields the base index (an O(1) Arc snapshot); the returned index
    /// is the one published into the cell (same file count).
    #[test]
    fn readiness_ready_snapshots_the_index() {
        let idx = Arc::new(make_test_index());
        let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Ready(idx)));
        let got = snapshot_ready_index(&readiness).expect("ready must yield the index");
        assert_eq!(got.total_files, 2);
    }

    /// `Building` yields the graceful retry status — not an index, not an error.
    #[test]
    fn readiness_building_yields_retry_status() {
        let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
        let status =
            snapshot_ready_index(&readiness).expect_err("building must not yield an index");
        assert_eq!(status, INDEXING_IN_PROGRESS_MESSAGE);
        assert!(status.contains("indexing in progress"));
    }

    /// `Failed` surfaces the build error text as a clear status.
    #[test]
    fn readiness_failed_yields_failure_status() {
        let readiness: SharedReadiness =
            Arc::new(RwLock::new(IndexReadiness::Failed("boom".to_string())));
        let status = snapshot_ready_index(&readiness).expect_err("failed must not yield an index");
        assert!(status.contains("indexing failed"));
        assert!(status.contains("boom"));
    }

    /// A tool call BEFORE the index is ready returns a JSON-RPC `result`
    /// (graceful status), never a session-killing `error`. `initialize` and
    /// `tools/list` still answer instantly regardless of readiness.
    #[test]
    fn stdio_loop_before_ready_returns_status_not_error() {
        let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
        let snapshot = make_shared_snapshot();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cxpak_context","arguments":{"op":"stats"}}}"#,
            "\n",
        );
        let start = std::time::Instant::now();
        let mut out: Vec<u8> = Vec::new();
        mcp_stdio_loop_readiness(
            std::path::Path::new("/tmp"),
            &readiness,
            &snapshot,
            std::io::Cursor::new(input.as_bytes()),
            &mut out,
        )
        .expect("loop");
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "handshake path must not block on indexing (took {elapsed:?})"
        );
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        // initialize: well-formed capabilities, index-independent.
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[0]["result"]["serverInfo"]["name"], "cxpak");
        // tools/list: static catalog projection, answers while Building.
        assert_eq!(lines[1]["id"], 2);
        assert!(lines[1]["result"]["tools"].is_array());
        // tools/call before ready: a `result` (not `error`) carrying the status.
        assert_eq!(lines[2]["id"], 3);
        assert!(
            lines[2]["error"].is_null(),
            "before-ready tool call must not be a protocol error"
        );
        let text = lines[2]["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text");
        assert!(
            text.contains("indexing in progress"),
            "before-ready tool call must carry the retry status, got: {text}"
        );
    }

    /// Once `Ready`, the same tool call returns normal results (no status text).
    #[test]
    fn stdio_loop_after_ready_returns_normal_results() {
        let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Ready(Arc::new(
            make_test_index(),
        ))));
        let snapshot = make_shared_snapshot();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"cxpak_context","arguments":{"op":"stats"}}}"#,
            "\n",
        );
        let mut out: Vec<u8> = Vec::new();
        mcp_stdio_loop_readiness(
            std::path::Path::new("/tmp"),
            &readiness,
            &snapshot,
            std::io::Cursor::new(input.as_bytes()),
            &mut out,
        )
        .expect("loop");
        let resp: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(resp["id"], 7);
        assert!(resp["error"].is_null());
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text");
        assert!(
            !text.contains("indexing in progress"),
            "ready tool call must return real results, got status: {text}"
        );
        // `stats` reports the two indexed files — proves it ran against the index.
        let parsed: Value = serde_json::from_str(text).expect("stats result is JSON");
        assert_eq!(parsed["files"], 2, "stats should reflect 2 indexed files");
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
        let (status, body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.get("error").is_some());
    }

    #[test]
    fn test_trace_handler_empty_target() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("".to_string()),
            tokens: None,
        };
        let (status, body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.get("error").is_some());
    }

    #[test]
    fn test_trace_handler_symbol_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("main".to_string()),
            tokens: None,
        };
        let (status, body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["target"], "main");
        assert_eq!(body["found"], true);
        assert_eq!(body["token_budget"], 50_000);
    }

    #[test]
    fn test_trace_handler_content_match() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("hello".to_string()),
            tokens: None,
        };
        let (status, body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["target"], "hello");
        assert_eq!(body["found"], true);
    }

    #[test]
    fn test_trace_handler_not_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("nonexistent_xyz".to_string()),
            tokens: None,
        };
        let (status, body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["target"], "nonexistent_xyz");
        assert_eq!(body["found"], false);
    }

    #[test]
    fn test_trace_handler_custom_tokens() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = TraceParams {
            target: Some("main".to_string()),
            tokens: Some("10k".to_string()),
        };
        let (status, body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["token_budget"], 10_000);
    }

    // --- v1.5.0: HTTP handlers for data_flow and cross_lang ---

    #[test]
    fn test_http_data_flow_handler() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = DataFlowParams {
            symbol: Some("main".into()),
            sink: None,
            depth: Some(10),
            focus: None,
        };
        let result = rt
            .block_on(data_flow_handler(State(shared), Json(params)))
            .unwrap();
        assert!(
            result.0.get("source").is_some(),
            "data_flow handler must return a source field"
        );
        assert!(
            result.0.get("limitations").is_some(),
            "data_flow handler must include limitations"
        );
    }

    #[test]
    fn test_http_data_flow_handler_missing_symbol() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = DataFlowParams {
            symbol: None,
            sink: None,
            depth: None,
            focus: None,
        };
        let result = rt.block_on(data_flow_handler(State(shared), Json(params)));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_http_cross_lang_handler() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = make_shared_index();
        let params = CrossLangParams {
            file: None,
            focus: None,
        };
        let result = rt
            .block_on(cross_lang_handler(State(shared), Query(params)))
            .unwrap();
        assert!(result.0["edges"].is_array());
        assert!(result.0["total"].is_u64());
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

    /// Build a temp git repo where `src/main.rs` imports `src/helper.rs`,
    /// commit both, then edit `helper.rs` on disk (uncommitted) so the
    /// working-tree diff is non-empty with a real dependent edge.
    fn review_git_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@test.com").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/helper.rs"),
            "pub fn work() -> i32 {\n    1\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "use crate::helper::work;\nfn main() {\n    let _ = work();\n}\n",
        )
        .unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        // Uncommitted edit to the depended-upon file.
        std::fs::write(
            dir.path().join("src/helper.rs"),
            "pub fn work() -> i32 {\n    2\n}\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn test_handle_tool_call_diff_without_review_has_no_review_field() {
        let dir = review_git_repo();
        let index = build_index(dir.path()).unwrap();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(10)),
            "cxpak_diff",
            &json!({}),
            &index,
            dir.path(),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(parsed["changed_files"].as_u64().unwrap() >= 1);
        assert!(parsed.get("review").is_none(), "review must be opt-in");
    }

    #[test]
    fn test_handle_tool_call_diff_review_attaches_bundle() {
        let dir = review_git_repo();
        let index = build_index(dir.path()).unwrap();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(11)),
            "cxpak_diff",
            &json!({"review": true}),
            &index,
            dir.path(),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        let review = parsed
            .get("review")
            .expect("review bundle present when review=true");
        // The structured bundle carries its sub-sections.
        assert!(review.get("changed_paths").is_some());
        assert!(review.get("blast").is_some());
        assert!(review.get("omissions").is_some());
        // helper.rs is the changed file; main.rs imports it → a dependent.
        let changed = review["changed_paths"].as_array().unwrap();
        assert!(changed.iter().any(|p| p == "src/helper.rs"));
        let direct = review["blast"]["categories"]["direct_dependents"]
            .as_array()
            .unwrap();
        let transitive = review["blast"]["categories"]["transitive_dependents"]
            .as_array()
            .unwrap();
        assert!(
            !direct.is_empty() || !transitive.is_empty(),
            "a changed file with a dependent must populate blast radius"
        );
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
        // Unknown tool must return a JSON-RPC error (-32601), not a result+isError.
        assert_eq!(
            resp["error"]["code"], -32601,
            "unknown tool must use error.code -32601"
        );
        let msg = resp["error"]["message"].as_str().unwrap();
        assert!(msg.contains("Unknown tool"), "got: {msg}");
    }

    // --- v1.5.0: data_flow and cross_lang MCP tools ---

    #[test]
    fn test_mcp_data_flow_tool() {
        // Use make_test_index which has a "main" symbol in src/main.rs.
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(10)),
            "cxpak_data_flow",
            &json!({"symbol": "main"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed.get("source").is_some(),
            "response must contain source"
        );
        assert!(
            parsed.get("limitations").is_some(),
            "response must contain limitations array"
        );
    }

    #[test]
    fn test_mcp_data_flow_missing_symbol() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(11)),
            "cxpak_data_flow",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("required"),
            "missing symbol should error with 'required', got: {text}"
        );
    }

    #[test]
    fn test_mcp_auto_context_cost_model_reaches_estimate() {
        // Proves the cost_model param is wired end-to-end through the MCP entry
        // point into the efficiency report (not just reachable in a unit test).
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(42)),
            "cxpak_auto_context",
            &json!({"task": "main", "cost_model": "claude-opus-4-8"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        let cost = &parsed["efficiency"]["cost_estimate"];
        assert!(
            cost.is_object(),
            "cost_model param must produce a cost_estimate; got {cost}"
        );
        assert_eq!(cost["model"], json!("claude-opus-4-8"));
        assert!(cost["input_usd"].as_f64().unwrap() >= 0.0);
    }

    #[test]
    fn test_mcp_auto_context_no_cost_model_no_estimate() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(43)),
            "cxpak_auto_context",
            &json!({"task": "main"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed["efficiency"]["cost_estimate"].is_null(),
            "no cost_model → no estimate"
        );
    }

    #[test]
    fn test_mcp_cross_lang_tool() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(12)),
            "cxpak_cross_lang",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(parsed["edges"].is_array(), "response must have edges array");
        assert!(parsed["total"].is_u64(), "response must have total count");
    }

    #[test]
    fn test_mcp_cross_lang_file_filter() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(13)),
            "cxpak_cross_lang",
            &json!({"file": "src/main.rs"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        let edges = parsed["edges"].as_array().unwrap();
        for e in edges {
            let src = e["source_file"].as_str().unwrap();
            let tgt = e["target_file"].as_str().unwrap();
            assert!(
                src == "src/main.rs" || tgt == "src/main.rs",
                "filtered edge must touch src/main.rs"
            );
        }
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
        // C3 (ADR-0182): the live MCP surface is the ≤8 intent-tool projection
        // of the capability catalog, not the 26 legacy tools.
        assert!(
            tools.len() <= 8,
            "MCP surface must be ≤8; got {}",
            tools.len()
        );
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "cxpak_context",
                "cxpak_graph",
                "cxpak_data",
                "cxpak_review",
                "cxpak_insight"
            ],
            "live tools/list must equal the deterministic catalog projection"
        );
        // Every intent-tool advertises read-only + a required `op` selector.
        for t in tools {
            assert_eq!(t["annotations"]["readOnlyHint"], serde_json::json!(true));
            assert_eq!(t["inputSchema"]["required"][0], "op");
            assert!(t["inputSchema"]["properties"]["op"]["enum"].is_array());
        }
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
    fn test_mcp_stdio_loop_invalid_json_returns_parse_error() {
        // Previously the server silently skipped invalid JSON lines (deadlock hazard).
        // After FIX-HIGH-4, it must respond with a JSON-RPC -32700 error so the client
        // knows the message was received and rejected.
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
        assert!(
            !output.is_empty(),
            "server must write a parse-error response for invalid JSON"
        );
        let response: Value = serde_json::from_slice(&output).expect("response must be valid JSON");
        assert!(
            response["id"].is_null(),
            "parse error id must be null, got: {}",
            response["id"]
        );
        assert_eq!(
            response["error"]["code"], -32700,
            "parse error must use code -32700"
        );
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
        let _router = build_router(shared, repo_path, None);
        // Router created without panic = success
    }

    // --- Axum integration (in-process HTTP) ---

    #[test]
    fn test_axum_health_endpoint() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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

    /// After an edit, the live watcher path must produce a graph **bit-identical
    /// to a full rebuild** and a PageRank equal to a cold recompute within float
    /// epsilon — it now uses the edge-delta rebuild and warm-started PageRank
    /// (ADR-0165/0166) instead of a full O(repo) rebuild. This is the
    /// daemon-level parity guard for that wiring (the per-function parity is
    /// proven exhaustively in tests/parity.rs).
    #[test]
    fn process_watcher_changes_delta_parity_with_full_rebuild() {
        use crate::daemon::watcher::FileChange;
        use crate::parser::language::{Import, ParseResult};

        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let a_abs = dir.path().join("src/a.rs");
        let b_abs = dir.path().join("src/b.rs");
        std::fs::write(&a_abs, "pub fn a() {}\n").unwrap();
        std::fs::write(&b_abs, "use crate::a;\npub fn go() {}\n").unwrap();

        // Initial index with a b->a edge carried on the *unmodified* importer
        // (b.rs).  Only a.rs is edited below, so b.rs keeps this import and the
        // edge is a stable, non-trivial structure for the parity check —
        // independent of what the real re-parser extracts from edited a.rs.
        let counter = TokenCounter::new();
        let files = vec![
            ScannedFile {
                relative_path: "src/a.rs".to_string(),
                absolute_path: a_abs.clone(),
                language: Some("rust".to_string()),
                size_bytes: 14,
            },
            ScannedFile {
                relative_path: "src/b.rs".to_string(),
                absolute_path: b_abs.clone(),
                language: Some("rust".to_string()),
                size_bytes: 30,
            },
        ];
        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/a.rs".to_string(),
            ParseResult {
                symbols: vec![],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/b.rs".to_string(),
            ParseResult {
                symbols: vec![],
                imports: vec![Import {
                    source: "crate::a".to_string(),
                    names: vec![],
                }],
                exports: vec![],
            },
        );
        let mut content = HashMap::new();
        content.insert("src/a.rs".to_string(), "pub fn a() {}\n".to_string());
        content.insert(
            "src/b.rs".to_string(),
            "use crate::a;\npub fn go() {}\n".to_string(),
        );
        let initial = CodebaseIndex::build_with_content(files, parse_results, &counter, content);
        assert!(
            initial
                .graph
                .edges
                .get("src/b.rs")
                .is_some_and(|s| !s.is_empty()),
            "pre-edit index should already have the b->a edge"
        );

        let shared: SharedIndex = Arc::new(RwLock::new(Arc::new(initial)));

        // Content-modify a.rs only → exercises the per-file delta path while
        // b.rs (and its b->a edge) stays unchanged.
        std::fs::write(&a_abs, "pub fn a() { /* edited */ }\n").unwrap();
        process_watcher_changes(&[FileChange::Modified(a_abs)], dir.path(), &shared);

        let got = shared.read().unwrap();
        // Oracle: full rebuild + cold PageRank over the same post-edit files.
        let mut oracle = (**got).clone();
        oracle.rebuild_graph();
        oracle.pagerank = crate::intelligence::pagerank::compute_pagerank(&oracle.graph, 0.85, 100);

        assert_eq!(
            got.graph.edges, oracle.graph.edges,
            "live delta graph must equal a full rebuild (bit-identical)"
        );
        assert_eq!(got.graph.reverse_edges, oracle.graph.reverse_edges);
        // PageRank converges to the same unique stationary distribution from a
        // warm or cold start, but after a fixed iteration count the two paths
        // agree only within float epsilon (same 2e-6 bound as tests/parity.rs).
        assert_eq!(
            got.pagerank
                .keys()
                .collect::<std::collections::BTreeSet<_>>(),
            oracle
                .pagerank
                .keys()
                .collect::<std::collections::BTreeSet<_>>(),
            "warm and cold PageRank must cover the same nodes"
        );
        for (k, cold) in &oracle.pagerank {
            let warm = got.pagerank[k];
            assert!(
                (warm - cold).abs() <= 2e-6,
                "warm-started PageRank for {k} diverged from cold: {warm} vs {cold}"
            );
        }
        assert!(
            got.graph
                .edges
                .get("src/b.rs")
                .is_some_and(|s| !s.is_empty()),
            "the b->a import edge must survive the watcher delta update"
        );
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
        let (status, _body) = rt.block_on(trace_handler(State(shared), Query(params)));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
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
        // C3: context_for_task and pack_context are now `op`s under cxpak_context.
        assert!(tools.len() <= 8);
        let ctx = tools
            .iter()
            .find(|t| t["name"] == "cxpak_context")
            .expect("cxpak_context intent-tool present");
        let ops: Vec<&str> = ctx["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(ops.contains(&"context_for_task"), "ops={ops:?}");
        assert!(ops.contains(&"pack_context"), "ops={ops:?}");
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

    // --- FIX-HIGH-4: parse error, unknown tool, pattern length guard ---

    #[test]
    fn test_mcp_malformed_json_returns_parse_error_response() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        // Deliberately malformed JSON line.
        let input = b"{ not valid json !!\n";
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(
            repo_path,
            &index,
            &make_shared_snapshot(),
            input.as_ref(),
            &mut output,
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&output)
            .expect("server must write valid JSON even for bad input");
        assert!(
            response["id"].is_null(),
            "parse error response must have id: null"
        );
        assert_eq!(
            response["error"]["code"], -32700,
            "parse error must use code -32700"
        );
    }

    #[test]
    fn test_mcp_unknown_tool_returns_json_rpc_error() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(42)),
            "nonexistent_tool_xyz",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(
            resp["error"]["code"], -32601,
            "unknown tool must return error.code -32601, got: {resp}"
        );
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("Unknown tool"),
            "error message must mention 'Unknown tool', got: {msg}"
        );
    }

    #[test]
    fn test_mcp_search_pattern_exceeds_max_length_returns_error() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let long_pattern = "a".repeat(MAX_PATTERN_LEN + 1);
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_search",
            &json!({"pattern": long_pattern}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("Error") || text.contains("error"),
            "oversized pattern must return an error, got: {text}"
        );
        assert!(
            text.contains("maximum length") || text.contains("exceeds"),
            "error must mention the length limit, got: {text}"
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
        // C3: `search` is now an `op` under cxpak_context (legacy regex search
        // preserved verbatim, distinct from the newer `retrieval` op).
        let ctx = tools
            .iter()
            .find(|t| t["name"] == "cxpak_context")
            .expect("cxpak_context intent-tool present");
        let ops: Vec<&str> = ctx["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            ops.contains(&"search"),
            "cxpak_context ops must include search: {ops:?}"
        );
        // Every intent-tool exposes the `op` selector and additional per-op params.
        for tool in tools {
            let props = tool["inputSchema"]["properties"].as_object().unwrap();
            assert!(
                props.contains_key("op"),
                "tool {} should have op selector",
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

    #[test]
    fn test_format_dependency_parent_tags_inferred() {
        // Import edges render the bare path.
        assert_eq!(
            format_dependency_parent("src/main.rs", &EdgeType::Import, EdgeConfidence::Extracted),
            "src/main.rs"
        );
        // Extracted non-import edge: label, no `inferred` tag.
        assert_eq!(
            format_dependency_parent(
                "schema/users.sql",
                &EdgeType::ForeignKey,
                EdgeConfidence::Extracted
            ),
            "schema/users.sql (via: foreign_key)"
        );
        // Inferred edge: label carries the `inferred` tag.
        assert_eq!(
            format_dependency_parent(
                "schema/orders.sql",
                &EdgeType::EmbeddedSql,
                EdgeConfidence::Inferred
            ),
            "schema/orders.sql (via: embedded_sql, inferred)"
        );
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
    fn test_mcp_context_intent_first_in_tools_list() {
        let index = make_test_index();
        let repo_path = std::path::Path::new("/tmp");
        let snap = make_shared_snapshot();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let input = format!("{request}\n");
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo_path, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        // C3: cxpak_context (Intent::Context) is the first intent-tool, and the
        // `context` (auto_context) op is its first op.
        assert_eq!(
            tools[0]["name"], "cxpak_context",
            "cxpak_context must be first in the tools list"
        );
        assert_eq!(
            tools[0]["inputSchema"]["properties"]["op"]["enum"][0], "context",
            "context (auto_context) must be the first op under cxpak_context"
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
        // C3: auto_context is `op=context` under cxpak_context; context_diff is
        // `op=review` under cxpak_review.
        let op_enum = |tool: &str| -> Vec<String> {
            tools
                .iter()
                .find(|t| t["name"] == tool)
                .and_then(|t| t["inputSchema"]["properties"]["op"]["enum"].as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        assert!(op_enum("cxpak_context").contains(&"context".to_string()));
        assert!(op_enum("cxpak_review").contains(&"review".to_string()));
        assert!(tools.len() <= 8, "total tool count must be ≤8");
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
        // Risks count should be <= default limit of 20
        assert!(risks.len() <= 20, "risks should be capped at 20 by default");
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
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

    #[test]
    fn path_validation_rejects_traversal() {
        let ws = std::path::Path::new("/tmp");
        assert!(validate_workspace_path(ws, "../../../etc/passwd").is_err());
    }

    #[test]
    fn path_validation_rejects_absolute() {
        let ws = std::path::Path::new("/tmp");
        assert!(validate_workspace_path(ws, "/etc/passwd").is_err());
    }

    #[test]
    fn path_validation_accepts_relative_subpath() {
        let ws = std::path::Path::new("/tmp");
        assert!(validate_workspace_path(ws, "src/main.rs").is_ok());
    }

    #[test]
    fn bearer_token_extracted_correctly() {
        assert_eq!(
            extract_bearer_token("Bearer mytoken123"),
            Some("mytoken123")
        );
    }

    #[test]
    fn bearer_token_returns_none_for_missing() {
        assert_eq!(extract_bearer_token("Basic abc"), None);
    }

    #[test]
    fn check_auth_allows_when_no_token_expected() {
        assert!(check_auth(None, None));
        assert!(check_auth(None, Some("anything")));
    }

    #[test]
    fn check_auth_rejects_when_token_missing() {
        assert!(!check_auth(Some("secret"), None));
    }

    #[test]
    fn check_auth_rejects_wrong_token() {
        assert!(!check_auth(Some("secret"), Some("wrong")));
    }

    #[test]
    fn check_auth_accepts_matching_token() {
        assert!(check_auth(Some("secret"), Some("secret")));
    }

    // --- Timing-safe auth tests ---

    #[test]
    fn check_auth_rejects_different_length_tokens() {
        // A shorter provided token must be rejected even if it's a prefix of the expected.
        assert!(!check_auth(Some("secretlong"), Some("secret")));
        assert!(!check_auth(Some("secret"), Some("secretlong")));
    }

    #[test]
    fn check_auth_uses_constant_time_comparison() {
        // This test verifies the function works correctly for same-length wrong tokens,
        // ensuring ConstantTimeEq is actually exercised on the byte slices.
        assert!(!check_auth(Some("aaaaaaaa"), Some("aaaaaaab")));
        assert!(!check_auth(Some("aaaaaaab"), Some("aaaaaaaa")));
        assert!(check_auth(Some("aaaaaaaa"), Some("aaaaaaaa")));
    }

    // --- Path canonicalization tests ---

    #[test]
    fn path_validation_rejects_dotdot_traversal_via_existing_parent() {
        // foo/../../etc/passwd: the second `..` would pop above the workspace
        // root, which is a traversal attempt and must be rejected.
        let ws = std::path::Path::new("/tmp");
        let result = validate_workspace_path(ws, "foo/../../etc/passwd");
        assert!(
            result.is_err(),
            "should reject path that attempts traversal above workspace"
        );
    }

    // --- MAX_PATTERN_LEN tests ---

    // Replaced cheater test "max_pattern_len_constant_is_1000" (which merely
    // asserted the constant value and would pass even if the guard were
    // disabled) with a real behavioural test: a pattern of exactly 1000 chars
    // must succeed while a pattern of 1001 chars must be rejected.

    #[test]
    fn search_accepts_pattern_at_exact_limit() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let repo_path = Arc::new(std::path::PathBuf::from("/tmp"));
            let app = build_router_for_test(shared, repo_path);

            // Exactly MAX_PATTERN_LEN characters — must NOT return 400.
            let exact_pattern = "a".repeat(MAX_PATTERN_LEN);
            let body = serde_json::to_vec(&serde_json::json!({"pattern": exact_pattern})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/search")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "pattern at exactly MAX_PATTERN_LEN must NOT return 400; \
                 if 400, the guard uses > instead of >"
            );
        });
    }

    #[test]
    fn search_returns_400_for_oversized_pattern() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let repo_path = Arc::new(std::path::PathBuf::from("/tmp"));
            let app = build_router_for_test(shared, repo_path);

            let long_pattern = "a".repeat(MAX_PATTERN_LEN + 1);
            let body = serde_json::to_vec(&serde_json::json!({"pattern": long_pattern})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/search")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "pattern exceeding MAX_PATTERN_LEN must return 400"
            );
        });
    }

    #[test]
    fn search_returns_400_for_empty_pattern() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let repo_path = Arc::new(std::path::PathBuf::from("/tmp"));
            let app = build_router_for_test(shared, repo_path);

            let body = serde_json::to_vec(&serde_json::json!({"pattern": ""})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/search")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "empty pattern must return 400"
            );
        });
    }

    // --- v1 router tests ---

    fn make_app_state_with_token(token: Option<&str>) -> AppState {
        AppState {
            index: make_shared_index(),
            repo_path: Arc::new(std::path::PathBuf::from("/tmp")),
            snapshot: make_shared_snapshot(),
            expected_token: token.map(|t| t.to_string()),
            workspace_root: Arc::new(std::path::PathBuf::from("/tmp")),
        }
    }

    fn build_full_router_with_state(state: AppState) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .merge(build_v1_router(state.clone()))
            .with_state(state)
    }

    #[test]
    fn v1_router_has_health_route() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let state = make_app_state_with_token(None);
            let app = build_full_router_with_state(state);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("GET")
                        .uri("/v1/health")
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
            assert!(json["total_files"].is_number());
            assert!(json["total_tokens"].is_number());
        });
    }

    #[test]
    fn v1_router_rejects_unauthorized() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let state = make_app_state_with_token(Some("secret"));
            let app = build_full_router_with_state(state);
            let body = serde_json::to_vec(&json!({})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/v1/health")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        });
    }

    #[test]
    fn v1_router_accepts_valid_token() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let state = make_app_state_with_token(Some("secret"));
            let app = build_full_router_with_state(state);
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("GET")
                        .uri("/v1/health")
                        .header("authorization", "Bearer secret")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        });
    }

    #[test]
    fn v1_conventions_returns_profile() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let state = make_app_state_with_token(None);
            let app = build_full_router_with_state(state);
            let body = serde_json::to_vec(&json!({})).unwrap();
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/v1/conventions")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        });
    }

    #[test]
    fn v1_wired_endpoints_return_ok() {
        // These endpoints now call real intelligence functions. Verify that
        // each returns 200 with a valid (non-stub) payload for a well-formed request.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Endpoints that accept empty {} body
            let simple_endpoints = vec![
                "/v1/risks",
                "/v1/architecture",
                "/v1/call_graph",
                "/v1/dead_code",
                "/v1/drift",
                "/v1/security_surface",
                "/v1/cross_lang",
            ];
            for uri in simple_endpoints {
                let state = make_app_state_with_token(None);
                let app = build_full_router_with_state(state);
                let body = serde_json::to_vec(&json!({})).unwrap();
                let response = app
                    .oneshot(
                        axum::http::Request::builder()
                            .method("POST")
                            .uri(uri)
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK, "expected 200 for {uri}");
            }

            // /v1/predict requires files
            {
                let state = make_app_state_with_token(None);
                let app = build_full_router_with_state(state);
                let body = serde_json::to_vec(&json!({"files": ["src/main.rs"]})).unwrap();
                let response = app
                    .oneshot(
                        axum::http::Request::builder()
                            .method("POST")
                            .uri("/v1/predict")
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "expected 200 for /v1/predict"
                );
            }

            // /v1/data_flow requires symbol
            {
                let state = make_app_state_with_token(None);
                let app = build_full_router_with_state(state);
                let body = serde_json::to_vec(&json!({"symbol": "main"})).unwrap();
                let response = app
                    .oneshot(
                        axum::http::Request::builder()
                            .method("POST")
                            .uri("/v1/data_flow")
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "expected 200 for /v1/data_flow"
                );
            }
        });
    }

    // --- Task 15: cxpak_visual MCP tool ---

    #[test]
    fn test_mcp_tools_list_hosts_visual_and_onboard_ops() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let repo = std::path::Path::new("/tmp");
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(repo, &index, &snap, input.as_bytes(), &mut output).unwrap();
        let resp: Value = serde_json::from_slice(&output).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        // C3: visual and onboard are `op`s under cxpak_insight.
        let insight = tools
            .iter()
            .find(|t| t["name"] == "cxpak_insight")
            .expect("cxpak_insight intent-tool present");
        let ops: Vec<&str> = insight["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            ops.contains(&"visual"),
            "insight ops must include visual: {ops:?}"
        );
        assert!(
            ops.contains(&"onboard"),
            "insight ops must include onboard: {ops:?}"
        );
    }

    #[test]
    fn test_mcp_visual_flow_requires_symbol() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(10)),
            "cxpak_visual",
            &json!({"type": "flow"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        // Per MCP spec, tool parameter validation uses mcp_tool_result with an Error: prefix.
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with("Error:"),
            "flow without symbol must return Error: tool result, got: {text}"
        );
        assert!(
            text.contains("symbol"),
            "error text must mention 'symbol', got: {text}"
        );
    }

    #[test]
    fn test_mcp_visual_diff_requires_files() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(11)),
            "cxpak_visual",
            &json!({"type": "diff"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        // Per MCP spec, tool parameter validation uses mcp_tool_result with an Error: prefix.
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.starts_with("Error:"),
            "diff without files must return Error: tool result, got: {text}"
        );
        assert!(
            text.contains("files"),
            "error text must mention 'files', got: {text}"
        );
    }

    #[cfg(feature = "visual")]
    #[test]
    fn test_mcp_visual_dashboard_returns_content() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(12)),
            "cxpak_visual",
            &json!({"type": "dashboard", "format": "html"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.starts_with("Error"),
            "dashboard should not return error, got: {text}"
        );
        assert!(
            text.len() > 100,
            "dashboard HTML should have substantial content"
        );
    }

    #[cfg(feature = "visual")]
    #[test]
    fn test_mcp_visual_mermaid_format() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(13)),
            "cxpak_visual",
            &json!({"type": "dashboard", "format": "mermaid"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.starts_with("Error"),
            "mermaid format should not error, got: {text}"
        );
        // Mermaid output starts with "graph TD"
        assert!(
            text.starts_with("graph TD"),
            "mermaid output should start with 'graph TD', got: {text}"
        );
    }

    // --- Task 15: cxpak_onboard MCP tool ---

    #[cfg(feature = "visual")]
    #[test]
    fn test_mcp_onboard_returns_phases() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(20)),
            "cxpak_onboard",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.starts_with("Error"),
            "onboard should not return error, got: {text}"
        );
        let parsed: Value = serde_json::from_str(text).expect("onboard result must be valid JSON");
        assert!(
            parsed["phases"].is_array(),
            "onboard result must have 'phases' array"
        );
        assert!(
            parsed["total_files"].is_number(),
            "onboard result must have 'total_files'"
        );
        assert!(
            parsed["estimated_reading_time"].is_string(),
            "onboard result must have 'estimated_reading_time'"
        );
    }

    #[cfg(feature = "visual")]
    #[test]
    fn test_mcp_onboard_markdown_format() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(21)),
            "cxpak_onboard",
            &json!({"format": "markdown"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        assert_eq!(resp["jsonrpc"], "2.0");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.starts_with("Error"),
            "onboard markdown should not error, got: {text}"
        );
        assert!(
            text.contains("# Codebase Onboarding Map"),
            "markdown output should contain h1 title"
        );
    }

    // ── MCP parameter wiring tests ────────────────────────────────────────────

    // cxpak_conventions: strength filter removes low-confidence observations.
    #[test]
    fn test_mcp_conventions_strength_convention_filters_weak_observations() {
        let mut index = make_test_index();
        // Set a Convention-strength (95%) and a Mixed-strength (55%) observation.
        index.conventions.naming.function_style =
            crate::conventions::PatternObservation::new("fn_naming", "snake_case", 95, 100);
        index.conventions.naming.type_style =
            crate::conventions::PatternObservation::new("type_naming", "camelCase", 55, 100);

        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(1)),
            "cxpak_conventions",
            &json!({"strength": "convention"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let val: Value = serde_json::from_str(text).unwrap();

        // snake_case has 95% — survives "convention" filter.
        assert!(
            !val["naming"]["function_style"].is_null(),
            "95% observation must survive 'convention' filter"
        );
        // camelCase has 55% — must be nullified by the filter.
        assert!(
            val["naming"]["type_style"].is_null(),
            "55% observation must be removed by 'convention' filter"
        );
    }

    // cxpak_conventions: focus prefix filters file_contributions.
    #[test]
    fn test_mcp_conventions_focus_filters_file_contributions() {
        let mut index = make_test_index();
        // Inject a file_contribution manually into naming.
        let mut contribs = std::collections::HashMap::new();
        contribs.insert(
            "src/main.rs".to_string(),
            crate::conventions::FileContribution {
                counts: Default::default(),
            },
        );
        contribs.insert(
            "tests/lib_test.rs".to_string(),
            crate::conventions::FileContribution {
                counts: Default::default(),
            },
        );
        index.conventions.naming.file_contributions = contribs;

        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(2)),
            "cxpak_conventions",
            &json!({"focus": "src/"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let val: Value = serde_json::from_str(text).unwrap();

        // Only "src/" entries should remain in naming file_contributions.
        if let Some(contribs) = val["naming"]["file_contributions"].as_object() {
            for key in contribs.keys() {
                assert!(
                    key.starts_with("src/"),
                    "focus='src/' must remove non-src contributions, found: {key}"
                );
            }
        }
        // The tests/ key must not appear.
        assert!(
            val["naming"]["file_contributions"]["tests/lib_test.rs"].is_null(),
            "tests/lib_test.rs must be removed by focus='src/' filter"
        );
    }

    // ── C4: conventions surface token-budget acceptance gate ─────────────────

    /// Build a CodebaseIndex whose `conventions.git_health` is large enough that
    /// serialising `category=all` exceeds `MAX_MCP_CONVENTIONS_TOKENS` (5 000).
    ///
    /// 250 co_changes entries × ~35 tok/entry ≈ 8 750 tokens, well above 5 000.
    /// 100 churn_30d / 100 churn_180d / 100 bugfix_density entries add ~4 000 more.
    fn make_large_conventions_index() -> CodebaseIndex {
        use crate::conventions::git_health::{ChurnEntry, GitHealthProfile};
        use crate::core_graph::intel::CoChangeEdge;
        use std::collections::HashMap;

        let mut index = make_test_index();

        let co_changes: Vec<CoChangeEdge> = (0..250u32)
            .map(|i| CoChangeEdge {
                file_a: format!(
                    "src/subsystem_alpha/module_{i}/component_{i}/implementation_{i}.rs"
                ),
                file_b: format!(
                    "tests/subsystem_alpha/module_{}/unit_tests_{i}.rs",
                    (i + 1) % 250
                ),
                count: (i % 20) + 1,
                recency_weight: 0.5 + (i % 10) as f64 * 0.05,
            })
            .collect();

        let churn_30d: Vec<ChurnEntry> = (0..100u32)
            .map(|i| ChurnEntry {
                path: format!("src/subsystem_beta/module_{i}/heavy_file_{i}.rs"),
                modifications: (100 - i) as usize,
                last_commit_epoch: None,
            })
            .collect();
        let churn_180d = churn_30d.clone();

        let mut bugfix_density: HashMap<String, f64> = HashMap::new();
        for i in 0..100u32 {
            bugfix_density.insert(
                format!("src/subsystem_beta/module_{i}/heavy_file_{i}.rs"),
                0.1 + (i % 10) as f64 * 0.02,
            );
        }

        index.conventions.git_health = GitHealthProfile {
            churn_30d,
            churn_180d,
            bugfix_density,
            reverts: vec![],
            churn_trend: HashMap::new(),
            co_changes,
            last_computed: None,
        };
        index
    }

    /// Gate: large ConventionProfile → MCP op output ≤ MAX_MCP_CONVENTIONS_TOKENS.
    #[test]
    fn test_mcp_conventions_output_honors_default_token_cap() {
        use crate::conventions::render::MAX_MCP_CONVENTIONS_TOKENS;

        let index = make_large_conventions_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(10)),
            "cxpak_conventions",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("conventions op must return text");

        let counter = TokenCounter::new();
        let token_count = counter.count(text);
        assert!(
            token_count <= MAX_MCP_CONVENTIONS_TOKENS,
            "conventions op output must be ≤ {MAX_MCP_CONVENTIONS_TOKENS} tokens, \
             but got {token_count} tokens"
        );
    }

    /// Gate: `tokens` override expands the budget — a larger limit includes more
    /// content than the default cap.
    #[test]
    fn test_mcp_conventions_tokens_override_expands_budget() {
        let index = make_large_conventions_index();
        let snap1 = make_shared_snapshot();
        let snap2 = make_shared_snapshot();

        // Response under the default cap (5 000 tokens).
        let resp_default = handle_tool_call(
            Some(json!(11)),
            "cxpak_conventions",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap1,
        );
        let text_default = resp_default["result"]["content"][0]["text"]
            .as_str()
            .expect("conventions op must return text");
        let len_default = text_default.len();

        // Response with a generous override (200 000 tokens — no truncation expected).
        let resp_large = handle_tool_call(
            Some(json!(12)),
            "cxpak_conventions",
            &json!({"tokens": "200k"}),
            &index,
            Path::new("/tmp"),
            &snap2,
        );
        let text_large = resp_large["result"]["content"][0]["text"]
            .as_str()
            .expect("conventions op must return text");
        let len_large = text_large.len();

        assert!(
            len_large > len_default,
            "a larger `tokens` budget must yield more content \
             (default={len_default} chars, large={len_large} chars)"
        );

        // The large response must not contain an _omitted marker (no truncation).
        let val_large: Value = serde_json::from_str(text_large).unwrap();
        assert!(
            val_large.get("_omitted").is_none(),
            "no truncation expected with a 200k token budget; _omitted should be absent"
        );

        // The small (default) response must not exceed the default cap.
        let counter = TokenCounter::new();
        let tok_default = counter.count(text_default);
        assert!(
            tok_default <= crate::conventions::render::MAX_MCP_CONVENTIONS_TOKENS,
            "default-cap response must be ≤ {} tokens, got {tok_default}",
            crate::conventions::render::MAX_MCP_CONVENTIONS_TOKENS
        );
    }

    /// Gate: `_omitted` marker is present when content was dropped.
    #[test]
    fn test_mcp_conventions_omission_marker_present_when_truncated() {
        let index = make_large_conventions_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(13)),
            "cxpak_conventions",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("conventions op must return text");
        let val: Value = serde_json::from_str(text).unwrap();

        assert!(
            val.get("_omitted").is_some(),
            "large profile with default cap must trigger omission — \
             `_omitted` key must be present in the response"
        );
        // The _omitted object must carry the applied_budget and steps_applied fields.
        let omitted = &val["_omitted"];
        assert!(
            omitted["applied_budget"].is_number(),
            "_omitted.applied_budget must be a number"
        );
        assert!(
            omitted["steps_applied"].is_array(),
            "_omitted.steps_applied must be an array"
        );
        assert!(
            !omitted["steps_applied"]
                .as_array()
                .unwrap_or(&vec![])
                .is_empty(),
            "_omitted.steps_applied must not be empty when content was dropped"
        );
    }

    /// Gate: `_omitted` marker is ABSENT when the profile fits under the budget.
    #[test]
    fn test_mcp_conventions_omission_marker_absent_when_fits() {
        // Use the default (small) test index — its conventions profile is tiny.
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(14)),
            "cxpak_conventions",
            &json!({}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("conventions op must return text");
        let val: Value = serde_json::from_str(text).unwrap();

        assert!(
            val.get("_omitted").is_none(),
            "small profile must fit under default cap — \
             `_omitted` must be ABSENT, got: {val}"
        );
    }

    /// Gate: same profile + same budget → byte-identical output (no HashMap
    /// iteration leak, no non-deterministic ordering).
    #[test]
    fn test_mcp_conventions_deterministic_output() {
        let index = make_large_conventions_index();

        let text_a = {
            let snap = make_shared_snapshot();
            let resp = handle_tool_call(
                Some(json!(15)),
                "cxpak_conventions",
                &json!({}),
                &index,
                Path::new("/tmp"),
                &snap,
            );
            resp["result"]["content"][0]["text"]
                .as_str()
                .expect("conventions op must return text")
                .to_string()
        };

        let text_b = {
            let snap = make_shared_snapshot();
            let resp = handle_tool_call(
                Some(json!(16)),
                "cxpak_conventions",
                &json!({}),
                &index,
                Path::new("/tmp"),
                &snap,
            );
            resp["result"]["content"][0]["text"]
                .as_str()
                .expect("conventions op must return text")
                .to_string()
        };

        assert_eq!(
            text_a, text_b,
            "conventions op output must be byte-identical across two calls \
             with the same profile and budget (no HashMap iteration leak)"
        );
    }

    // cxpak_diff: tokens budget truncates the files list.
    #[test]
    fn test_mcp_diff_tokens_budget_field_present_in_response() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(3)),
            "cxpak_diff",
            &json!({"tokens": "1k"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        // When there is no git repo at /tmp the tool returns an error — that's fine;
        // when it succeeds the response must contain token_budget.
        if !text.starts_with("Error") {
            let val: Value = serde_json::from_str(text).unwrap();
            assert!(
                val.get("token_budget").is_some(),
                "token_budget field must be present in diff response"
            );
            assert_eq!(
                val["token_budget"].as_u64().unwrap_or(0),
                1000,
                "token_budget must be 1k = 1000"
            );
        }
    }

    // cxpak_call_graph: depth=1 only returns seed edges, depth=2 expands one hop.
    #[test]
    fn test_mcp_call_graph_depth_parameter_accepted() {
        let mut index = make_test_index();
        use crate::intelligence::call_graph::{CallEdge, CallGraph};
        index.call_graph = CallGraph {
            edges: vec![
                CallEdge {
                    caller_file: "src/main.rs".to_string(),
                    caller_symbol: "main".to_string(),
                    callee_file: "src/lib.rs".to_string(),
                    callee_symbol: "helper".to_string(),
                    confidence: crate::intelligence::call_graph::CallConfidence::Exact,
                    resolution_note: None,
                },
                CallEdge {
                    caller_file: "src/lib.rs".to_string(),
                    caller_symbol: "helper".to_string(),
                    callee_file: "src/util.rs".to_string(),
                    callee_symbol: "util_fn".to_string(),
                    confidence: crate::intelligence::call_graph::CallConfidence::Exact,
                    resolution_note: None,
                },
            ],
            unresolved: vec![],
        };

        let snap = make_shared_snapshot();

        // depth=1: only edges directly involving src/main.rs
        let resp_d1 = handle_tool_call(
            Some(json!(4)),
            "cxpak_call_graph",
            &json!({"target": "src/main.rs", "depth": 1}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text_d1 = resp_d1["result"]["content"][0]["text"].as_str().unwrap();
        let val_d1: Value = serde_json::from_str(text_d1).unwrap();
        let edges_d1 = val_d1["edges"].as_array().unwrap();

        // depth=2: should also include the lib→util edge via BFS
        let resp_d2 = handle_tool_call(
            Some(json!(5)),
            "cxpak_call_graph",
            &json!({"target": "src/main.rs", "depth": 2}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text_d2 = resp_d2["result"]["content"][0]["text"].as_str().unwrap();
        let val_d2: Value = serde_json::from_str(text_d2).unwrap();
        let edges_d2 = val_d2["edges"].as_array().unwrap();

        // depth=2 must reach at least as many edges as depth=1.
        assert!(
            edges_d2.len() >= edges_d1.len(),
            "depth=2 must reach at least as many edges as depth=1"
        );
        // At depth=2 the lib→util edge must also be reachable.
        let has_util_edge = edges_d2.iter().any(|e| {
            e["callee_file"].as_str() == Some("src/util.rs")
                || e["caller_file"].as_str() == Some("src/util.rs")
        });
        assert!(has_util_edge, "depth=2 must include the util edge via BFS");
    }

    // cxpak_call_graph: workspace filters so both caller and callee must match.
    #[test]
    fn test_mcp_call_graph_workspace_both_sides_must_match() {
        let mut index = make_test_index();
        use crate::intelligence::call_graph::{CallEdge, CallGraph};
        index.call_graph = CallGraph {
            edges: vec![
                // Both in src/
                CallEdge {
                    caller_file: "src/main.rs".to_string(),
                    caller_symbol: "main".to_string(),
                    callee_file: "src/lib.rs".to_string(),
                    callee_symbol: "helper".to_string(),
                    confidence: crate::intelligence::call_graph::CallConfidence::Exact,
                    resolution_note: None,
                },
                // Only caller in src/
                CallEdge {
                    caller_file: "src/main.rs".to_string(),
                    caller_symbol: "main".to_string(),
                    callee_file: "vendor/ext.rs".to_string(),
                    callee_symbol: "ext_fn".to_string(),
                    confidence: crate::intelligence::call_graph::CallConfidence::Exact,
                    resolution_note: None,
                },
            ],
            unresolved: vec![],
        };

        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(6)),
            "cxpak_call_graph",
            &json!({"workspace": "src/"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let val: Value = serde_json::from_str(text).unwrap();
        let edges = val["edges"].as_array().unwrap();

        // Only the src/→src/ edge must remain.
        assert_eq!(
            edges.len(),
            1,
            "workspace filter must keep only src/→src/ edges"
        );
        assert_eq!(edges[0]["callee_file"].as_str().unwrap(), "src/lib.rs");
    }

    // cxpak_dead_code: workspace parameter narrows the dead-code search prefix.
    #[test]
    fn test_mcp_dead_code_workspace_parameter_accepted() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(7)),
            "cxpak_dead_code",
            &json!({"workspace": "src/"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let val: Value = serde_json::from_str(text).unwrap();
        // Response must be structured even with workspace filter.
        assert!(
            val.get("dead_symbols").is_some(),
            "dead_code response must contain dead_symbols field"
        );
        // Any returned symbols must be within the workspace prefix.
        if let Some(syms) = val["dead_symbols"].as_array() {
            for sym in syms {
                if let Some(path) = sym["file"].as_str() {
                    assert!(
                        path.starts_with("src/"),
                        "workspace='src/' must restrict dead symbols to src/, got: {path}"
                    );
                }
            }
        }
    }

    // cxpak_predict: focus prefix filters all impact vectors.
    #[test]
    fn test_mcp_predict_focus_filters_impact_entries() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(8)),
            "cxpak_predict",
            &json!({"files": ["src/main.rs"], "focus": "src/"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let val: Value = serde_json::from_str(text).unwrap();

        // All entries in structural_impact, historical_impact, call_impact must
        // have paths starting with "src/".
        for field in &["structural_impact", "historical_impact", "call_impact"] {
            if let Some(arr) = val[field].as_array() {
                for entry in arr {
                    if let Some(path) = entry["path"].as_str() {
                        assert!(
                            path.starts_with("src/"),
                            "focus='src/' must restrict {field} entries to src/, got: {path}"
                        );
                    }
                }
            }
        }
        // test_impact entries must have test_file starting with "src/".
        if let Some(arr) = val["test_impact"].as_array() {
            for entry in arr {
                if let Some(tf) = entry["test_file"].as_str() {
                    assert!(
                        tf.starts_with("src/"),
                        "focus='src/' must restrict test_impact to src/, got: {tf}"
                    );
                }
            }
        }
    }

    // cxpak_architecture: workspace parameter filters modules.
    #[test]
    fn test_mcp_architecture_workspace_filters_modules() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        let resp = handle_tool_call(
            Some(json!(9)),
            "cxpak_architecture",
            &json!({"workspace": "src/"}),
            &index,
            Path::new("/tmp"),
            &snap,
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let val: Value = serde_json::from_str(text).unwrap();

        // All returned modules must have prefix starting with "src/".
        if let Some(modules) = val["modules"].as_array() {
            for m in modules {
                if let Some(prefix) = m["prefix"].as_str() {
                    assert!(
                        prefix.starts_with("src/"),
                        "workspace='src/' must restrict modules to src/, got: {prefix}"
                    );
                }
            }
        }
        // The response must always contain circular_deps.
        assert!(
            val.get("circular_deps").is_some(),
            "architecture response must contain circular_deps field"
        );
    }

    // --- FIX-WAVE5 #2: body size limit (413 Payload Too Large) ---

    #[test]
    fn test_body_size_limit_returns_413() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let shared = make_shared_index();
            let app = build_router(shared, Arc::new(std::path::PathBuf::from("/tmp")), None);
            // Build a 3 MB body (exceeds the 2 MB DefaultBodyLimit)
            let oversized_body = vec![b'x'; 3 * 1024 * 1024];
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/search")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(oversized_body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::PAYLOAD_TOO_LARGE,
                "requests with >2MB body must be rejected with 413"
            );
        });
    }

    // --- FIX-WAVE5 #4: score_coupling uses forward edges only ---

    #[test]
    fn test_score_coupling_forward_edges_only() {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use crate::schema::EdgeType;
        use std::collections::HashMap;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let make = |name: &str| {
            let fp = dir.path().join(name.replace('/', "_"));
            std::fs::write(&fp, "fn f() {}").unwrap();
            ScannedFile {
                relative_path: name.to_string(),
                absolute_path: fp,
                language: Some("rust".into()),
                size_bytes: 9,
            }
        };
        // 2 files in src/mod, 1 in src/other → 3 files in "src" module (depth=1)
        let mut index = CodebaseIndex::build(
            vec![
                make("src/mod/a.rs"),
                make("src/mod/b.rs"),
                make("src/mod/c.rs"),
            ],
            HashMap::new(),
            &counter,
        );
        // 1 intra-module edge, 1 cross-module edge → coupling = 1/2 = 0.5
        index
            .graph
            .add_edge("src/mod/a.rs", "src/mod/b.rs", EdgeType::Import);
        index
            .graph
            .add_edge("src/mod/a.rs", "other/x.rs", EdgeType::Import);

        let score = crate::intelligence::health::score_coupling(&index, 2);
        // With the fix, total=2 forward edges, cross=1 → ratio=0.5 → score=5.0
        // Pre-fix (double-counting reverse_edges) would give ratio=1/3 → score=6.67
        assert!(
            (score - 5.0).abs() < 1e-6,
            "coupling score must be 5.0 (1 cross / 2 total forward edges), got {score}; \
             if 6.67 reverse-edge double-count is back"
        );
    }

    // --- FIX-WAVE5 #12: batch requests rejected ---

    #[test]
    fn test_mcp_batch_request_rejected() {
        let index = make_test_index();
        let snap = make_shared_snapshot();
        // A JSON array is a batch request
        let input = concat!(
            r#"[{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}},"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}]"#,
            "\n"
        );
        let cursor = std::io::Cursor::new(input.as_bytes().to_vec());
        let mut output = Vec::new();
        mcp_stdio_loop_with_io(Path::new("/tmp"), &index, &snap, cursor, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        let resp: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(resp["error"]["code"], -32600);
        assert!(
            resp["error"]["message"].as_str().unwrap().contains("Batch"),
            "batch request error message must mention Batch"
        );
    }
}
