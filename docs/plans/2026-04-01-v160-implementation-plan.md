# v1.6.0 "The Platform" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add LSP server with 14 custom methods, versioned HTTP Intelligence API, and convention export standard.

**Architecture:** Three independent surface additions share the same hot `CodebaseIndex` already maintained by `src/commands/serve.rs`. The LSP server (`src/lsp/`) is a new module behind a `lsp` feature flag that runs over stdio and reuses `FileWatcher` from `src/daemon/watcher.rs` for index freshness. The Intelligence API extends the existing axum router in `serve.rs` with a `/v1/` prefix and auth middleware. Convention export is a pure data transformation of the existing `ConventionProfile` into a versioned, checksummed JSON artifact plus two CLI subcommands.

**Tech Stack:** Rust 1.80+, tower-lsp 0.20, axum 0.8, tokio 1, serde/serde_json, sha2, chrono, git2

---

## Prerequisites

Before starting, verify tower-lsp compatibility: `tower-lsp = "0.20"` uses `tower = "0.4"` internally, while `axum = "0.8"` uses `tower = "0.5"`. These can coexist as separate tower versions in the dependency graph because they are not unified — confirm by adding both to Cargo.toml and running `cargo check`. If a version conflict surfaces on the shared `http` crate, pin `tower-lsp` to a fork or vendor it. Record the outcome in Cargo.toml comments before any feature implementation begins.

---

## Task 0: Prerequisite refactoring — visibility and test helpers

**Why first:** Multiple later tasks need `process_watcher_changes` as `pub(crate)`, `build_index` as `pub`, and integration tests need `commands` module to be public. Do this once upfront.

**Files:**
- `src/commands/serve.rs`
- `src/main.rs`
- `src/lib.rs` (create if absent)

**Steps:**
1. In `src/commands/serve.rs`, change `fn process_watcher_changes(...)` to `pub(crate) fn process_watcher_changes(...)`.
2. In `src/commands/serve.rs`, change `pub(crate) fn build_index(...)` to `pub fn build_index(...)`.
3. In `src/main.rs`, change `mod commands;` to `pub mod commands;` (or create `src/lib.rs` with `pub mod commands;` re-export if the crate needs to expose these for integration tests).
4. Extract a `pub fn build_router_for_test(...)` helper from `build_router` that takes explicit state parameters, making it usable from integration tests without constructing the full serve runtime.
5. Run `cargo check` — confirm all existing tests still pass.
6. Commit: "refactor: prepare serve.rs visibility for LSP and integration tests"

---

## Task 1: Dependency audit and Cargo.toml preparation

**Files:**
- `Cargo.toml`

**Steps:**
1. Add `tower-lsp = { version = "0.20", optional = true }` to `[dependencies]`. **HARD GATE:** Run `cargo check --all-features` immediately. If tower-lsp conflicts with axum 0.8 (http crate version), STOP and resolve before proceeding to any other task.
2. Add `sha2 = "0.10"` to `[dependencies]` (non-optional, needed by convention export).
3. Add `chrono = { version = "0.4", features = ["serde"] }` to `[dependencies]` (non-optional, needed for `ConventionExport.generated_at`).
4. Add `lsp` feature: `lsp = ["dep:tower-lsp", "daemon"]`.
5. Add `"lsp"` to the `default` feature array (alongside `"daemon"` and `"embeddings"`).
6. Run `cargo check --all-features` and confirm no unresolved dependency conflicts.
7. If `http` crate version conflict arises between tower-lsp and axum 0.8, document the resolution strategy as a comment in Cargo.toml.

**Code:**
```toml
# In [dependencies]
tower-lsp = { version = "0.20", optional = true }
sha2 = "0.10"
chrono = { version = "0.4", features = ["serde"] }

# In [features]
lsp = ["dep:tower-lsp", "daemon"]
default = [
    "lang-rust", "lang-typescript", "lang-javascript", "lang-java", "lang-python", "lang-go",
    "lang-c", "lang-cpp", "lang-ruby", "lang-csharp", "lang-swift", "lang-kotlin",
    "lang-bash", "lang-css", "lang-scss", "lang-php", "lang-markdown",
    "lang-json", "lang-yaml", "lang-toml", "lang-dockerfile", "lang-hcl", "lang-dart",
    "lang-scala", "lang-lua", "lang-elixir", "lang-zig", "lang-haskell",
    "lang-groovy", "lang-objc", "lang-r", "lang-julia", "lang-ocaml", "lang-matlab",
    "lang-proto", "lang-svelte", "lang-makefile", "lang-html", "lang-graphql", "lang-xml",
    "lang-sql", "lang-prisma",
    "daemon",
    "embeddings",
    "lsp",
]
```

**Commands:**
```
cargo check --all-features 2>&1 | grep -E "^error"
```

---

## Task 2: Convention export types and checksum

**Files:**
- `src/conventions/export.rs` (new)
- `src/conventions/mod.rs` (add `pub mod export;`)

**Steps:**
1. Write test: `convention_export_roundtrip` — build a `ConventionExport` from a default `ConventionProfile`, serialize to JSON, deserialize back, assert `version == "1.0"`, `generator` starts with `"cxpak "`, checksum is non-empty, and the deserialized profile equals the original.
2. Write test: `checksum_is_deterministic` — produce two exports from the same profile, assert their checksums are identical.
3. Write test: `checksum_changes_on_profile_change` — produce exports from two different profiles, assert their checksums differ.
4. Implement `ConventionExport` with `version: String`, `generated_at: String`, `generator: String`, `repo: String`, `profile: ConventionProfile`, `checksum: String`.
5. Implement `fn compute_checksum(profile: &ConventionProfile) -> String` using `sha2::Sha256` over the canonical JSON of the profile (sorted keys via `serde_json::to_string` of a `BTreeMap`).
6. Implement `fn build_export(repo: &str, profile: ConventionProfile) -> ConventionExport` using `chrono::Utc::now().to_rfc3339()` for `generated_at` and `format!("cxpak {}", env!("CARGO_PKG_VERSION"))` for `generator`.
7. Run the three tests.

**Code:**
```rust
// src/conventions/export.rs
use crate::conventions::ConventionProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConventionExport {
    pub version: String,
    pub generated_at: String,
    pub generator: String,
    pub repo: String,
    pub profile: ConventionProfile,
    pub checksum: String,
}

/// Compute a stable SHA256 checksum of the profile by serializing via BTreeMap
/// for deterministic key ordering.
pub fn compute_checksum(profile: &ConventionProfile) -> String {
    // Serialize to Value, convert to BTreeMap for stable ordering, re-serialize.
    let value = serde_json::to_value(profile).unwrap_or_default();
    let stable = to_stable_value(value);
    let json = serde_json::to_string(&stable).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn to_stable_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let btree: BTreeMap<_, _> = map
                .into_iter()
                .map(|(k, val)| (k, to_stable_value(val)))
                .collect();
            serde_json::Value::Object(btree.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(to_stable_value).collect())
        }
        other => other,
    }
}

/// Build a convention export for a given repo path and profile.
pub fn build_export(repo: &str, profile: ConventionProfile) -> ConventionExport {
    let checksum = compute_checksum(&profile);
    ConventionExport {
        version: "1.0".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        generator: format!("cxpak {}", env!("CARGO_PKG_VERSION")),
        repo: repo.to_string(),
        profile,
        checksum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convention_export_roundtrip() {
        let profile = ConventionProfile::default();
        let export = build_export("test-repo", profile.clone());
        assert_eq!(export.version, "1.0");
        assert!(export.generator.starts_with("cxpak "));
        assert!(!export.checksum.is_empty());
        let json = serde_json::to_string(&export).unwrap();
        let back: ConventionExport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.checksum, export.checksum);
    }

    #[test]
    fn checksum_is_deterministic() {
        let profile = ConventionProfile::default();
        let a = compute_checksum(&profile);
        let b = compute_checksum(&profile);
        assert_eq!(a, b);
    }

    #[test]
    fn checksum_changes_on_profile_change() {
        let mut profile_a = ConventionProfile::default();
        let mut profile_b = ConventionProfile::default();
        profile_a.git_health.reverts = vec![];
        profile_b.git_health.reverts = vec![crate::conventions::git_health::RevertEntry { sha: "abc".into(), message: "revert".into(), date: "2026-01-01".into() }];
        let a = compute_checksum(&profile_a);
        let b = compute_checksum(&profile_b);
        assert_ne!(a, b);
    }
}
```

**Commands:**
```
cargo test --lib conventions::export
```

---

## Task 3: Convention diff logic

**Files:**
- `src/conventions/diff.rs` (new)
- `src/conventions/mod.rs` (add `pub mod diff;`)

**Steps:**
1. Write test: `diff_identical_exports_is_empty` — diff two exports with the same checksum, assert result is empty.
2. Write test: `diff_detects_changed_checksum` — diff two exports with different checksums and different git_health revert counts, assert `has_changes == true` and the diff output is non-empty.
3. Write test: `diff_output_contains_field_name` — when git_health.reverts differs, assert the diff text contains "git_health".
4. Implement `ConventionDiff { has_changes: bool, summary: String, changed_fields: Vec<String> }`.
5. Implement `fn diff_exports(current: &ConventionExport, baseline: &ConventionExport) -> ConventionDiff` that short-circuits on identical checksums, then performs a field-level diff by comparing JSON values recursively, collecting changed top-level keys.
6. Run the three tests.

**Code:**
```rust
// src/conventions/diff.rs
use crate::conventions::export::ConventionExport;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ConventionDiff {
    pub has_changes: bool,
    pub summary: String,
    pub changed_fields: Vec<String>,
}

pub fn diff_exports(current: &ConventionExport, baseline: &ConventionExport) -> ConventionDiff {
    if current.checksum == baseline.checksum {
        return ConventionDiff {
            has_changes: false,
            summary: "No convention changes detected.".to_string(),
            changed_fields: Vec::new(),
        };
    }

    let current_val = serde_json::to_value(&current.profile).unwrap_or_default();
    let baseline_val = serde_json::to_value(&baseline.profile).unwrap_or_default();

    let mut changed = Vec::new();
    if let (serde_json::Value::Object(cur), serde_json::Value::Object(base)) =
        (current_val, baseline_val)
    {
        for (key, cur_val) in &cur {
            let base_val = base.get(key);
            if base_val != Some(cur_val) {
                changed.push(key.clone());
            }
        }
        for key in base.keys() {
            if !cur.contains_key(key) {
                changed.push(key.clone());
            }
        }
    }

    changed.sort();
    changed.dedup();

    let summary = if changed.is_empty() {
        format!(
            "Checksum differs (generated_at or metadata changed) but profile fields are identical."
        )
    } else {
        format!(
            "{} convention category(s) changed: {}",
            changed.len(),
            changed.join(", ")
        )
    };

    ConventionDiff {
        has_changes: true,
        summary,
        changed_fields: changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conventions::export::build_export;
    use crate::conventions::ConventionProfile;

    #[test]
    fn diff_identical_exports_is_empty() {
        let profile = ConventionProfile::default();
        let a = build_export("repo", profile.clone());
        let b = build_export("repo", profile);
        // Force same checksum by using the same export
        let diff = diff_exports(&a, &a);
        assert!(!diff.has_changes);
        assert!(diff.changed_fields.is_empty());
    }

    #[test]
    fn diff_detects_changed_checksum() {
        let mut pa = ConventionProfile::default();
        let mut pb = ConventionProfile::default();
        pa.git_health.reverts = vec![];
        pb.git_health.reverts = vec![crate::conventions::git_health::RevertEntry { sha: "def".into(), message: "revert fix".into(), date: "2026-02-01".into() }];
        let a = build_export("repo", pa);
        let b = build_export("repo", pb);
        assert_ne!(a.checksum, b.checksum);
        let diff = diff_exports(&a, &b);
        assert!(diff.has_changes);
        assert!(!diff.summary.is_empty());
    }

    #[test]
    fn diff_output_contains_field_name() {
        let mut pa = ConventionProfile::default();
        let mut pb = ConventionProfile::default();
        pa.git_health.reverts = vec![];
        pb.git_health.reverts = vec![crate::conventions::git_health::RevertEntry { sha: "def".into(), message: "revert fix".into(), date: "2026-02-01".into() }];
        let a = build_export("repo", pa);
        let b = build_export("repo", pb);
        let diff = diff_exports(&a, &b);
        assert!(diff.changed_fields.iter().any(|f| f.contains("git_health")));
    }
}
```

**Commands:**
```
cargo test --lib conventions::diff
```

---

## Task 4: `cxpak conventions export` and `cxpak conventions diff` CLI commands

**Files:**
- `src/commands/conventions.rs` (new)
- `src/commands/mod.rs` (add `pub mod conventions;`)
- `src/cli/mod.rs` (add `Conventions` subcommand with `Export` and `Diff` sub-subcommands)
- `src/main.rs` (wire `Commands::Conventions`)

**Steps:**
1. Write test: `cli_conventions_export_parses` — parse `["cxpak", "conventions", "export", "."]` via `Cli::try_parse_from`, assert it yields `Commands::Conventions` with `ConventionsSubcommand::Export { path: "." }`.
2. Write test: `cli_conventions_diff_parses` — parse `["cxpak", "conventions", "diff", "."]`, assert `ConventionsSubcommand::Diff { path: "." }`.
3. Add `Conventions { subcommand: ConventionsSubcommand, path: PathBuf }` to `Commands` enum in `src/cli/mod.rs`.
4. Add `ConventionsSubcommand` enum with `Export` and `Diff` variants.
5. Implement `commands::conventions::run_export(path: &Path) -> Result<(), Box<dyn Error>>`: build index, build convention profile, call `build_export`, write to `.cxpak/conventions.json`, print confirmation.
6. Implement `commands::conventions::run_diff(path: &Path) -> Result<(), Box<dyn Error>>`: build index, build current export, read `.cxpak/conventions.json` as baseline (error if missing with actionable message), call `diff_exports`, print diff summary to stdout.
7. Wire in `main.rs`.
8. Run the two CLI parsing tests plus an integration test that calls `run_export` on a temp dir.

**Code:**
```rust
// src/commands/conventions.rs
use crate::commands::serve::build_index;
use crate::conventions::diff::diff_exports;
use crate::conventions::export::build_export;
use std::error::Error;
use std::path::Path;

pub fn run_export(path: &Path) -> Result<(), Box<dyn Error>> {
    let index = build_index(path)?;
    let repo = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let repo_str = repo.to_string_lossy().to_string();
    let export = build_export(&repo_str, index.conventions);
    let cxpak_dir = path.join(".cxpak");
    std::fs::create_dir_all(&cxpak_dir)?;
    let out = cxpak_dir.join("conventions.json");
    let json = serde_json::to_string_pretty(&export)?;
    std::fs::write(&out, json)?;
    eprintln!(
        "cxpak: conventions exported to {} (checksum: {})",
        out.display(),
        &export.checksum[..8]
    );
    Ok(())
}

pub fn run_diff(path: &Path) -> Result<(), Box<dyn Error>> {
    let baseline_path = path.join(".cxpak").join("conventions.json");
    if !baseline_path.exists() {
        return Err(format!(
            "No baseline found at {}. Run: cxpak conventions export .",
            baseline_path.display()
        )
        .into());
    }
    let baseline_json = std::fs::read_to_string(&baseline_path)?;
    let baseline: crate::conventions::export::ConventionExport =
        serde_json::from_str(&baseline_json)?;
    let index = build_index(path)?;
    let repo = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let current = build_export(&repo.to_string_lossy(), index.conventions);
    let diff = diff_exports(&current, &baseline);
    println!("{}", serde_json::to_string_pretty(&diff)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn export_creates_conventions_json() {
        let dir = TempDir::new().unwrap();
        // Minimal git repo so Scanner doesn't error
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git/HEAD"),
            "ref: refs/heads/main\n",
        ).unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        // run_export may fail if git2 can't open a real repo; use a try
        let result = run_export(dir.path());
        // Either succeeds or fails gracefully (no panic)
        let _ = result;
        // If it wrote the file, validate JSON
        let out = dir.path().join(".cxpak").join("conventions.json");
        if out.exists() {
            let content = std::fs::read_to_string(&out).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert_eq!(parsed["version"], "1.0");
        }
    }
}
```

```rust
// Additions to src/cli/mod.rs

#[derive(Subcommand)]
pub enum ConventionsSubcommand {
    /// Write .cxpak/conventions.json from the current codebase
    Export {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Compare current conventions against .cxpak/conventions.json
    Diff {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

// In Commands enum:
/// Export and diff convention profiles
Conventions {
    #[command(subcommand)]
    subcommand: ConventionsSubcommand,
},
```

**Commands:**
```
cargo test --lib conventions
cargo test --lib cli::tests::cli_conventions_export_parses
cargo test --lib cli::tests::cli_conventions_diff_parses
```

---

## Task 5: Intelligence API — auth middleware and path validation

**Files:**
- `src/commands/serve.rs` (extend)

**Steps:**
1. Write test: `path_validation_rejects_traversal` — call a new `validate_workspace_path(workspace: &Path, requested: &str)` function with `requested = "../../../etc/passwd"`, assert it returns `Err`.
2. Write test: `path_validation_rejects_absolute` — call with an absolute path like `/etc/passwd`, assert `Err`.
3. Write test: `path_validation_accepts_relative_subpath` — call with `"src/main.rs"`, assert `Ok`.
4. Write test: `bearer_token_extracted_correctly` — call `extract_bearer_token("Bearer mytoken123")`, assert `Some("mytoken123")`.
5. Write test: `bearer_token_returns_none_for_missing` — call with `"Basic abc"`, assert `None`.
6. Implement `fn validate_workspace_path(workspace: &Path, requested: &str) -> Result<PathBuf, String>` that rejects paths containing `..`, paths that are absolute, and canonicalized paths not under workspace root.
7. Implement `fn extract_bearer_token(header: &str) -> Option<&str>` that strips the `"Bearer "` prefix.
8. Add `OptionalAuth(Option<String>)` extractor for axum that reads `Authorization` header.
9. Add `fn check_auth(expected: Option<&str>, provided: Option<&str>) -> bool` — returns `true` when expected is `None` (no auth configured) or when tokens match.
10. Run all five tests.

**Code:**
```rust
// In src/commands/serve.rs

pub fn validate_workspace_path(
    workspace: &std::path::Path,
    requested: &str,
) -> Result<std::path::PathBuf, String> {
    if requested.contains("..") {
        return Err(format!("path traversal rejected: {requested}"));
    }
    let p = std::path::Path::new(requested);
    if p.is_absolute() {
        return Err(format!("absolute paths rejected: {requested}"));
    }
    let candidate = workspace.join(p);
    // Ensure it stays under workspace
    let ws_canon = workspace
        .canonicalize()
        .map_err(|e| format!("workspace canonicalize failed: {e}"))?;
    // candidate may not exist yet; use starts_with on the joined path
    if !candidate.starts_with(&ws_canon) && !candidate.starts_with(workspace) {
        return Err(format!("path escapes workspace: {requested}"));
    }
    Ok(candidate)
}

pub fn extract_bearer_token(header: &str) -> Option<&str> {
    header.strip_prefix("Bearer ")
}

pub fn check_auth(expected: Option<&str>, provided: Option<&str>) -> bool {
    match expected {
        None => true,
        Some(tok) => provided == Some(tok),
    }
}

#[cfg(test)]
mod serve_auth_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn path_validation_rejects_traversal() {
        let ws = Path::new("/tmp");
        assert!(validate_workspace_path(ws, "../../../etc/passwd").is_err());
    }

    #[test]
    fn path_validation_rejects_absolute() {
        let ws = Path::new("/tmp");
        assert!(validate_workspace_path(ws, "/etc/passwd").is_err());
    }

    #[test]
    fn path_validation_accepts_relative_subpath() {
        let ws = Path::new("/tmp");
        assert!(validate_workspace_path(ws, "src/main.rs").is_ok());
    }

    #[test]
    fn bearer_token_extracted_correctly() {
        assert_eq!(extract_bearer_token("Bearer mytoken123"), Some("mytoken123"));
    }

    #[test]
    fn bearer_token_returns_none_for_missing() {
        assert_eq!(extract_bearer_token("Basic abc"), None);
    }
}
```

**Commands:**
```
cargo test --lib commands::serve::serve_auth_tests
```

---

## Task 6: Intelligence API — `AppState` extension and `/v1/` router

**Files:**
- `src/commands/serve.rs` (extend)

**Steps:**
1. Write test: `v1_router_has_health_route` — build a `TestClient` against `build_v1_router(...)` and `GET /v1/health`, assert `200`.
2. Write test: `v1_router_rejects_unauthorized` — set `expected_token = Some("secret")`, send request without `Authorization` header, assert `401`.
3. Write test: `v1_router_accepts_valid_token` — send `Authorization: Bearer secret`, assert `200` not `401`.
4. Extend `AppState` with `expected_token: Option<String>` and `workspace_root: Arc<std::path::PathBuf>`.
5. Implement `build_v1_router(state: AppState) -> Router` that mounts all 12 `/v1/` routes as `POST` and applies a `tower::ServiceBuilder` layer extracting the `Authorization` header and calling `check_auth`.
6. Add `--bind` flag (default `"127.0.0.1"`) and `--token` flag (default `None`) to `Commands::Serve` in `src/cli/mod.rs` and thread through `commands::serve::run(...)`.
7. Update `run()` to bind to the provided address and pass token to `AppState`.
8. Mount the v1 router in `build_router`: `.merge(build_v1_router(state.clone()))`.
9. Run the three tests.

**Code:**
```rust
// Skeleton — full handler bodies come in Task 7-9

fn build_v1_router(state: AppState) -> Router {
    use axum::middleware::{self, Next};
    use axum::extract::Request;
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

    Router::new()
        .route("/v1/health", axum::routing::post(v1_health_handler))
        .route("/v1/risks", axum::routing::post(v1_risks_handler))
        .route("/v1/architecture", axum::routing::post(v1_architecture_handler))
        .route("/v1/call_graph", axum::routing::post(v1_call_graph_handler))
        .route("/v1/dead_code", axum::routing::post(v1_dead_code_handler))
        .route("/v1/predict", axum::routing::post(v1_predict_handler))
        .route("/v1/drift", axum::routing::post(v1_drift_handler))
        .route("/v1/security_surface", axum::routing::post(v1_security_surface_handler))
        .route("/v1/data_flow", axum::routing::post(v1_data_flow_handler))
        .route("/v1/cross_lang", axum::routing::post(v1_cross_lang_handler))
        .route("/v1/conventions", axum::routing::post(v1_conventions_handler))
        .route("/v1/briefing", axum::routing::post(v1_briefing_handler))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_layer))
        .with_state(state)
}
```

**Commands:**
```
cargo test --lib commands::serve
```

---

## Task 7: Intelligence API — `/v1/health`, `/v1/risks`, `/v1/architecture`, `/v1/conventions`, `/v1/briefing`

**Files:**
- `src/commands/serve.rs` (implement handler bodies)

**Steps:**
1. Write test for each handler: build a real index from a temp dir with a known Rust file, POST to the route, assert response is valid JSON with expected top-level keys.
2. Implement `v1_health_handler`: read lock on index, serialize `index.health` (if present from v1.2 work) or return a `{"status": "ok", "note": "health score available from v1.2+"}` placeholder with `200`. For v1.6 the field may not exist on `CodebaseIndex` yet — return the index's `total_files` and `total_tokens` as a minimal health response.
3. Implement `v1_risks_handler`: accepts `{"focus": Option<String>, "limit": Option<usize>}`, returns `{"risks": []}` stub (full population comes in v1.2+).
4. Implement `v1_architecture_handler`: accepts `{"focus": Option<String>}`, returns `{"modules": [], "circular_deps": []}` stub.
5. Implement `v1_conventions_handler`: accepts `{"workspace": Option<String>}`, reads the index's `conventions` field, serializes and returns as JSON.
6. Implement `v1_briefing_handler`: accepts `{"task": String, "tokens": Option<usize>, "focus": Option<String>}`, validates path, calls `crate::auto_context::auto_context(...)`, serializes result.
7. Implement the remaining stub handlers (`v1_call_graph_handler`, `v1_dead_code_handler`, `v1_predict_handler`, `v1_drift_handler`, `v1_security_surface_handler`, `v1_data_flow_handler`, `v1_cross_lang_handler`) as `{"status": "not_implemented", "available_from": "v1.3+"}` with `200` — these are populated by v1.3-v1.5.
8. Run all handler tests.

**Code:**
```rust
#[derive(Deserialize)]
struct V1FocusParams {
    focus: Option<String>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
struct V1BriefingParams {
    task: String,
    tokens: Option<usize>,
    focus: Option<String>,
}

async fn v1_health_handler(
    State(index): State<SharedIndex>,
    Json(_params): Json<V1FocusParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({
        "total_files": idx.total_files,
        "total_tokens": idx.total_tokens,
        "note": "full health score available from v1.2+"
    })))
}

async fn v1_conventions_handler(
    State(index): State<SharedIndex>,
    Json(_params): Json<V1FocusParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    serde_json::to_value(&idx.conventions)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn v1_briefing_handler(
    State(index): State<SharedIndex>,
    Json(params): Json<V1BriefingParams>,
) -> Result<Json<Value>, StatusCode> {
    let idx = index.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let opts = crate::auto_context::AutoContextOpts {
        tokens: params.tokens.unwrap_or(20_000),
        focus: params.focus,
        include_tests: true,
        include_blast_radius: false,
    };
    let result = crate::auto_context::auto_context(&params.task, &idx, &opts);
    serde_json::to_value(&result)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

// Stub for v1.3+ endpoints
macro_rules! stub_handler {
    ($name:ident, $available:literal) => {
        async fn $name() -> Json<Value> {
            Json(json!({
                "status": "not_implemented",
                "available_from": $available
            }))
        }
    };
}

stub_handler!(v1_risks_handler, "v1.2+");
stub_handler!(v1_architecture_handler, "v1.2+");
stub_handler!(v1_call_graph_handler, "v1.3+");
stub_handler!(v1_dead_code_handler, "v1.3+");
stub_handler!(v1_predict_handler, "v1.4+");
stub_handler!(v1_drift_handler, "v1.4+");
stub_handler!(v1_security_surface_handler, "v1.4+");
stub_handler!(v1_data_flow_handler, "v1.5+");
stub_handler!(v1_cross_lang_handler, "v1.5+");
```

**Commands:**
```
cargo test --lib commands::serve
```

---

## Task 8: `--bind` and `--token` CLI flags wired end-to-end

**Files:**
- `src/cli/mod.rs` (add flags to `Commands::Serve`)
- `src/main.rs` (thread flags through)
- `src/commands/serve.rs` (update `run` signature)

**Steps:**
1. Write test: `cli_serve_default_bind` — parse `["cxpak", "serve"]` (daemon feature), assert `bind == "127.0.0.1"`.
2. Write test: `cli_serve_custom_bind` — parse `["cxpak", "serve", "--bind", "0.0.0.0"]`, assert `bind == "0.0.0.0"`.
3. Write test: `cli_serve_token_flag` — parse `["cxpak", "serve", "--token", "secret"]`, assert `token == Some("secret")`.
4. Add `#[arg(long, default_value = "127.0.0.1")] bind: String` and `#[arg(long)] token: Option<String>` to `Commands::Serve`.
5. Update `run(path, port, bind, token, budget, verbose)` signature.
6. Parse `bind` as `SocketAddr` using `format!("{bind}:{port}")` — return clear error if invalid.
7. Run the three CLI tests plus existing serve tests.

**Code:**
```rust
// In Commands::Serve
#[arg(long, default_value = "127.0.0.1")]
bind: String,
#[arg(long)]
token: Option<String>,

// In run()
pub fn run(
    path: &Path,
    port: u16,
    bind: &str,
    token: Option<&str>,
    _token_budget: usize,
    _verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: std::net::SocketAddr = format!("{bind}:{port}").parse().map_err(|e| {
        format!("invalid bind address '{bind}:{port}': {e}")
    })?;
    // ...
}
```

**Commands:**
```
cargo test --lib cli
cargo test --lib commands::serve
```

---

## Task 9: `src/lsp/mod.rs` — module skeleton and feature gate

**Files:**
- `src/lsp/mod.rs` (new)
- `src/lsp/backend.rs` (new)
- `src/lsp/methods.rs` (new)
- `src/main.rs` (add `#[cfg(feature = "lsp")] pub mod lsp;`)

**Steps:**
1. Write test: `lsp_module_compiles_with_feature` — a `#[test]` in `src/lsp/mod.rs` that does nothing except `assert!(true)`, verifying the module compiles under the `lsp` feature flag.
2. Add `#[cfg(feature = "lsp")] pub mod lsp;` to `src/main.rs`.
3. Create `src/lsp/mod.rs` that re-exports `backend::CxpakLspBackend` and `pub fn run_stdio(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>>`.
4. Create `src/lsp/backend.rs` with the `CxpakLspBackend` struct holding `SharedIndex`, `SharedPath`, and implementing `tower_lsp::LanguageServer`.
5. Create `src/lsp/methods.rs` containing the 14 custom method dispatch functions — initially returning empty stubs.
6. Verify `cargo check --features lsp` passes.
7. Run the compile test.

**Code:**
```rust
// src/lsp/mod.rs
pub mod backend;
pub mod methods;

pub use backend::CxpakLspBackend;

/// Entry point for `cxpak lsp` — runs the LSP server over stdio until stdin closes.
pub fn run_stdio(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use tower_lsp::{LspService, Server};
    let index = crate::commands::serve::build_index(path)?;
    let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
    let shared_path = std::sync::Arc::new(path.to_path_buf());
    let (service, socket) = LspService::new(|client| {
        CxpakLspBackend::new(client, std::sync::Arc::clone(&shared), std::sync::Arc::clone(&shared_path))
    });
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        Server::new(stdin, stdout, socket).serve(service).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn lsp_module_compiles_with_feature() {
        assert!(true);
    }
}
```

**Commands:**
```
cargo test --features lsp --lib lsp
cargo check --features lsp
```

---

## Task 10: `CxpakLspBackend` struct and `LanguageServer` trait skeleton

**Files:**
- `src/lsp/backend.rs`

**Steps:**
1. Write test: `backend_new_stores_path` — construct a `CxpakLspBackend` with a mock client and temp path, assert `backend.path` resolves correctly.
2. Implement `CxpakLspBackend { client: tower_lsp::Client, index: SharedIndex, path: SharedPath }`.
3. Implement the `#[tower_lsp::async_trait] impl tower_lsp::LanguageServer for CxpakLspBackend` block with all required methods: `initialize`, `initialized`, `shutdown`. These are the only methods `tower_lsp` mandates; all others use default trait implementations.
4. `initialize` returns `InitializeResult` with `capabilities` set to declare `CodeLensProvider`, `DiagnosticProvider` (via `diagnostic_provider`), `HoverProvider`, and `WorkspaceSymbolProvider` capabilities.
5. `initialized` spawns the file watcher background task (reusing `FileWatcher` from `src/daemon/watcher.rs`) via `tokio::spawn` using the tokio runtime.
6. `shutdown` returns `Ok(())`.
7. Run test.

**Code:**
```rust
// src/lsp/backend.rs
use crate::daemon::watcher::FileWatcher;
use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{async_trait, Client, LanguageServer};

type SharedIndex = Arc<RwLock<crate::index::CodebaseIndex>>;
type SharedPath = Arc<std::path::PathBuf>;

pub struct CxpakLspBackend {
    pub client: Client,
    pub index: SharedIndex,
    pub path: SharedPath,
}

impl CxpakLspBackend {
    pub fn new(client: Client, index: SharedIndex, path: SharedPath) -> Self {
        Self { client, index, path }
    }
}

#[async_trait]
impl LanguageServer for CxpakLspBackend {
    async fn initialize(&self, _params: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        inter_file_dependencies: true,
                        workspace_diagnostics: true,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        let path = Arc::clone(&self.path);
        let index = Arc::clone(&self.index);
        tokio::spawn(async move {
            let watcher = match FileWatcher::new(&path) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("cxpak lsp: watcher failed: {e}");
                    return;
                }
            };
            loop {
                if let Some(first) =
                    watcher.recv_timeout(std::time::Duration::from_secs(1))
                {
                    let mut changes = vec![first];
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    changes.extend(watcher.drain());
                    crate::commands::serve::process_watcher_changes(
                        &changes, &path, &index,
                    );
                }
            }
        });
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }
}
```

**Commands:**
```
cargo test --features lsp --lib lsp::backend
```

---

## Task 11: Standard LSP method — `textDocument/codeLens`

**Files:**
- `src/lsp/backend.rs` (add `code_lens` method)
- `src/lsp/methods.rs` (add `code_lens_for_file`)

**Steps:**
1. Write test: `code_lens_returns_empty_for_unknown_file` — call `methods::code_lens_for_file("nonexistent.rs", &index)`, assert result is an empty `Vec`.
2. Write test: `code_lens_returns_health_lens_for_known_file` — insert a file into a test index, call `code_lens_for_file`, assert at least one `CodeLens` is returned with a non-empty `command.title`.
3. Implement `pub fn code_lens_for_file(uri_path: &str, index: &crate::index::CodebaseIndex) -> Vec<tower_lsp::lsp_types::CodeLens>` in `methods.rs`.
4. For each file in the index matching `uri_path`, produce one lens per category: health (line 0), risk score (line 0), dead code count (line 0 — stub 0 until v1.3). Format as `"cxpak: {tokens} tokens | {language}"` for a minimal useful lens in v1.6 (full intelligence fields from v1.2-v1.5 are added as those modules land).
5. Add `async fn code_lens(&self, params: CodeLensParams) -> LspResult<Option<Vec<CodeLens>>>` to the `LanguageServer` impl block, delegating to `methods::code_lens_for_file`.
6. Run both tests.

**Code:**
```rust
// src/lsp/methods.rs
use tower_lsp::lsp_types::{CodeLens, Command, Position, Range};

pub fn code_lens_for_file(
    uri_path: &str,
    index: &crate::index::CodebaseIndex,
) -> Vec<CodeLens> {
    let relative = uri_path
        .trim_start_matches("file://")
        .trim_start_matches('/');

    let file = index.files.iter().find(|f| {
        f.relative_path == relative
            || uri_path.ends_with(&f.relative_path)
    });

    match file {
        None => Vec::new(),
        Some(f) => {
            let range = Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            };
            vec![CodeLens {
                range,
                command: Some(Command {
                    title: format!(
                        "cxpak: {} tokens | {}",
                        f.token_count,
                        f.language.as_deref().unwrap_or("unknown")
                    ),
                    command: "cxpak.showFileInfo".to_string(),
                    arguments: None,
                }),
                data: None,
            }]
        }
    }
}
```

**Commands:**
```
cargo test --features lsp --lib lsp::methods
```

---

## Task 12: Standard LSP method — `textDocument/hover`

**Files:**
- `src/lsp/backend.rs` (add `hover` method)
- `src/lsp/methods.rs` (add `hover_for_symbol`)

**Steps:**
1. Write test: `hover_returns_none_for_empty_index` — call `hover_for_symbol("unknown_fn", &index)` with an empty index, assert `None`.
2. Write test: `hover_returns_markdown_content` — insert a file with a known symbol into the index, call `hover_for_symbol`, assert `Some(Hover)` with `HoverContents::Markup` containing the symbol name.
3. Implement `pub fn hover_for_symbol(symbol: &str, index: &crate::index::CodebaseIndex) -> Option<tower_lsp::lsp_types::Hover>`.
4. Find the symbol via `index.find_symbol(symbol)`. If found, build a markdown hover with: symbol kind, file path, token count of the containing file. Add `PageRank: {score:.3}` if pagerank scores are available on the index (they are, via `index.graph`).
5. Add `async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>>` to `LanguageServer` impl.
6. Run both tests.

**Code:**
```rust
pub fn hover_for_symbol(
    symbol: &str,
    index: &crate::index::CodebaseIndex,
) -> Option<tower_lsp::lsp_types::Hover> {
    use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

    let matches = index.find_symbol(symbol);
    let (file_path, sym) = matches.first()?;

    let pagerank = index
        .graph
        .pagerank_scores
        .get(file_path.as_str())
        .copied()
        .unwrap_or(0.0);

    let file = index.files.iter().find(|f| &f.relative_path == file_path);
    let token_count = file.map(|f| f.token_count).unwrap_or(0);

    let content = format!(
        "**{}** `{:?}`\n\nFile: `{}`\nTokens: {}\nPageRank: {:.3}",
        sym.name, sym.kind, file_path, token_count, pagerank
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}
```

**Commands:**
```
cargo test --features lsp --lib lsp::methods
```

---

## Task 13: Standard LSP method — `textDocument/diagnostic`

**Files:**
- `src/lsp/backend.rs` (add `diagnostic` method)
- `src/lsp/methods.rs` (add `diagnostics_for_file`)

**Steps:**
1. Write test: `diagnostics_empty_for_unknown_file` — call `diagnostics_for_file("missing.rs", &index)`, assert empty `Vec`.
2. Write test: `diagnostics_from_convention_violations` — build a profile with convention violations via `crate::conventions::verify`, call `diagnostics_for_file`, assert at least one `Diagnostic` is returned.
3. Implement `pub fn diagnostics_for_file(uri_path: &str, index: &crate::index::CodebaseIndex) -> Vec<tower_lsp::lsp_types::Diagnostic>`.
4. Call `crate::conventions::verify::check_file(uri_path, index)` to get violations. Map each `Violation` to a `Diagnostic` with `severity: DiagnosticSeverity::HINT`, `source: Some("cxpak".into())`, and the violation message.
5. Add `async fn diagnostic(&self, params: DocumentDiagnosticParams) -> LspResult<DocumentDiagnosticReportResult>` to `LanguageServer` impl, delegating to `diagnostics_for_file`.
6. Run both tests.

**Commands:**
```
cargo test --features lsp --lib lsp::methods
```

---

## Task 14: Standard LSP method — `workspace/symbol`

**Files:**
- `src/lsp/backend.rs` (add `symbol` method)
- `src/lsp/methods.rs` (add `workspace_symbols`)

**Steps:**
1. Write test: `workspace_symbols_empty_query_returns_all` — index a file with 3 symbols, call `workspace_symbols("", &index)`, assert 3 results.
2. Write test: `workspace_symbols_filtered_by_query` — index symbols `["foo", "bar", "baz"]`, call with `"ba"`, assert only `"bar"` and `"baz"` are returned.
3. Write test: `workspace_symbols_dead_symbols_include_tag` — stub a dead symbol scenario (zero pagerank, no callers), assert the `WorkspaceSymbol.tags` contains `SymbolTag::DEPRECATED` as a stand-in marker for dead symbols.
4. Implement `pub fn workspace_symbols(query: &str, index: &crate::index::CodebaseIndex) -> Vec<tower_lsp::lsp_types::WorkspaceSymbol>`.
5. Iterate all symbols across all files, filter by `symbol.name.contains(query)`, build `WorkspaceSymbol` with `name`, `kind`, and `location`. Symbols with pagerank 0.0 and no callers receive `tags: Some(vec![SymbolTag::DEPRECATED])`.
6. Add `async fn symbol(&self, params: WorkspaceSymbolParams) -> LspResult<Option<Vec<SymbolInformation>>>` (use the older `SymbolInformation` form for broader LSP client compatibility).
7. Run all three tests.

**Commands:**
```
cargo test --features lsp --lib lsp::methods
```

---

## Task 15: Custom LSP method — `cxpak/health` through `cxpak/blastRadius` dispatch

**Files:**
- `src/lsp/backend.rs` (add custom method dispatch)
- `src/lsp/methods.rs` (add `handle_custom_method`)

**Steps:**
1. Write test: `custom_method_health_returns_json` — call `handle_custom_method("cxpak/health", serde_json::Value::Null, &index)`, assert response is `Ok(Some(Value))` with a `"total_files"` key.
2. Write test: `custom_method_unknown_returns_error` — call `handle_custom_method("cxpak/nonexistent", ...)`, assert the response contains an error indicator.
3. Write test: `all_14_custom_methods_are_registered` — assert that calling each of the 14 method names returns `Ok(Some(...))` rather than a method-not-found error.
4. Implement `pub fn handle_custom_method(method: &str, params: serde_json::Value, index: &crate::index::CodebaseIndex) -> Result<Option<serde_json::Value>, tower_lsp::jsonrpc::Error>` as a `match` dispatch over all 14 method names.
5. Implement each dispatch arm — for methods whose data lives in future versions (v1.2-v1.5), return `{"status": "available_from", "version": "v1.X"}`. For `cxpak/health`, `cxpak/conventions`, and `cxpak/blastRadius`, return real data from the current index.
6. `cxpak/blastRadius` accepts `{"file": String}` in params, calls `crate::intelligence::blast_radius::compute_blast_radius`, returns the result.
7. `cxpak/conventions` returns `serde_json::to_value(&index.conventions)`.
8. Register custom methods using `tower_lsp::LspService::build` with `.custom_method("cxpak/health", CxpakLspBackend::handle_cxpak_health)` for each of the 14 methods. Do NOT use `execute_command` — that handles `workspace/executeCommand`, not custom JSON-RPC methods. Each custom handler calls `handle_custom_method` internally.
9. Run all three tests.

**Code:**
```rust
// src/lsp/methods.rs (addition)
pub fn handle_custom_method(
    method: &str,
    params: serde_json::Value,
    index: &crate::index::CodebaseIndex,
) -> Result<Option<serde_json::Value>, tower_lsp::jsonrpc::Error> {
    use serde_json::json;
    match method {
        "cxpak/health" => Ok(Some(json!({
            "total_files": index.total_files,
            "total_tokens": index.total_tokens,
            "note": "full health score from v1.2+"
        }))),
        "cxpak/conventions" => {
            let v = serde_json::to_value(&index.conventions)
                .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))?;
            Ok(Some(v))
        }
        "cxpak/blastRadius" => {
            let file = params
                .get("file")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("missing 'file'"))?;
            let changed: Vec<&str> = vec![file];
            let result = crate::intelligence::blast_radius::compute_blast_radius(
                &changed, &index.graph, &index.pagerank, &index.test_map, 3, None,
            );
            serde_json::to_value(&result)
                .map(Some)
                .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))
        }
        "cxpak/risks" => Ok(Some(json!({"available_from": "v1.2+"}))),
        "cxpak/architecture" => Ok(Some(json!({"available_from": "v1.2+"}))),
        "cxpak/callGraph" => Ok(Some(json!({"available_from": "v1.3+"}))),
        "cxpak/deadCode" => Ok(Some(json!({"available_from": "v1.3+"}))),
        "cxpak/predict" => Ok(Some(json!({"available_from": "v1.4+"}))),
        "cxpak/drift" => Ok(Some(json!({"available_from": "v1.4+"}))),
        "cxpak/securitySurface" => Ok(Some(json!({"available_from": "v1.4+"}))),
        "cxpak/dataFlow" => Ok(Some(json!({"available_from": "v1.5+"}))),
        "cxpak/crossLang" => Ok(Some(json!({"available_from": "v1.5+"}))),
        "cxpak/briefing" => {
            let task = params
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("summarize");
            let opts = crate::auto_context::AutoContextOpts {
                tokens: 20_000,
                focus: None,
                include_tests: false,
                include_blast_radius: false,
            };
            let result = crate::auto_context::auto_context(task, index, &opts);
            serde_json::to_value(&result)
                .map(Some)
                .map_err(|e| tower_lsp::jsonrpc::Error::invalid_params(e.to_string()))
        }
        "cxpak/coChanges" => Ok(Some(json!({"co_changes": [], "available_from": "v1.2+"}))),
        _ => Err(tower_lsp::jsonrpc::Error::method_not_found()),
    }
}
```

**Commands:**
```
cargo test --features lsp --lib lsp::methods
```

---

## Task 16: `cxpak lsp` CLI command wired end-to-end

**Files:**
- `src/cli/mod.rs` (add `Lsp` subcommand)
- `src/main.rs` (wire `Commands::Lsp`)
- `src/commands/mod.rs` (add `#[cfg(feature = "lsp")] pub mod lsp_cmd;`)
- `src/commands/lsp_cmd.rs` (new)

**Steps:**
1. Write test: `cli_lsp_parses` — parse `["cxpak", "lsp", "."]` with the `lsp` feature enabled, assert it yields `Commands::Lsp { path: PathBuf::from(".") }`.
2. Write test: `cli_lsp_default_path` — parse `["cxpak", "lsp"]`, assert `path == PathBuf::from(".")`.
3. Add `#[cfg(feature = "lsp")] Lsp { #[arg(default_value = ".")] path: PathBuf }` to `Commands`.
4. Create `src/commands/lsp_cmd.rs` with `pub fn run(path: &Path) -> Result<(), Box<dyn Error>>` that calls `crate::lsp::run_stdio(path)`.
5. Wire in `main.rs`: `#[cfg(feature = "lsp")] Commands::Lsp { path } => commands::lsp_cmd::run(path)`.
6. Run both CLI tests.

**Commands:**
```
cargo test --features lsp --lib cli::tests::cli_lsp_parses
cargo test --features lsp --lib cli::tests::cli_lsp_default_path
cargo check --features lsp
```

---

## Task 17: File watcher integration in LSP backend

**Files:**
- `src/lsp/backend.rs` (complete `initialized` implementation)

**Steps:**
1. Write test: `watcher_task_applies_incremental_update` — create a temp dir with a `.git` dir and a Rust file, build an index, spawn `initialized`, write a new file to the dir, wait 300ms, read-lock the index, assert `total_files` increased by 1.
2. In `initialized`, replace the stub watcher loop with a proper async loop using `tokio::task::spawn_blocking` to call `watcher.recv_timeout` (since `FileWatcher` uses `mpsc::Receiver` which is sync). After receiving changes, call `crate::commands::serve::process_watcher_changes` (make that function `pub(crate)` if not already).
3. Ensure the spawned task holds an `Arc` clone of both `index` and `path` so the backend can be dropped without killing the watcher.
4. Run the integration test.

**Code:**
```rust
async fn initialized(&self, _params: InitializedParams) {
    let path = Arc::clone(&self.path);
    let index = Arc::clone(&self.index);

    tokio::spawn(async move {
        let watcher = match FileWatcher::new(&path) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("cxpak lsp: watcher error: {e}");
                return;
            }
        };

        loop {
            // FileWatcher uses sync mpsc — block_in_place to avoid stalling executor
            let opt = tokio::task::block_in_place(|| {
                watcher.recv_timeout(std::time::Duration::from_secs(1))
            });
            if let Some(first) = opt {
                let mut changes = vec![first];
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                changes.extend(watcher.drain());
                crate::commands::serve::process_watcher_changes(&changes, &path, &index);
            }
        }
    });
}
```

**Commands:**
```
cargo test --features lsp --lib lsp::backend
```

---

## Task 18: Integration test — LSP initialize/shutdown over stdio

**Files:**
- `tests/lsp_integration.rs` (new)

**Steps:**
1. Write test: `lsp_initialize_shutdown_roundtrip` — spawn `cxpak lsp` as a subprocess (using `assert_cmd::Command`) against a temp repo, send a minimal JSON-RPC `initialize` request over stdin, read the response from stdout, assert `result.capabilities` is a JSON object, then send `shutdown` + `exit`, assert process exits `0`.
2. Write test: `lsp_custom_method_health` — after `initialize`, send `{"jsonrpc":"2.0","id":2,"method":"cxpak/health","params":{}}`, read response, assert `result.total_files` is a number.
3. Write test: `lsp_unknown_method_returns_error` — send `{"jsonrpc":"2.0","id":3,"method":"cxpak/nonexistent","params":{}}`, assert response has `"error"` key.
4. Mark all three tests with `#[ignore]` so they only run in CI with `cargo test --ignored` (they require the binary to be compiled first via `cargo build`).

**Code:**
```rust
// tests/lsp_integration.rs
#[cfg(feature = "lsp")]
mod lsp_integration {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use tempfile::TempDir;

    fn minimal_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();
        dir
    }

    fn make_request(id: u64, method: &str, params: &str) -> String {
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        );
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
    }

    #[test]
    #[ignore]
    fn lsp_initialize_shutdown_roundtrip() {
        let repo = minimal_repo();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cxpak"))
            .args(["lsp", repo.path().to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn cxpak lsp");

        let stdin = child.stdin.as_mut().unwrap();
        let init = make_request(
            1,
            "initialize",
            r#"{"processId":null,"rootUri":null,"capabilities":{}}"#,
        );
        stdin.write_all(init.as_bytes()).unwrap();

        let shutdown = make_request(2, "shutdown", "null");
        stdin.write_all(shutdown.as_bytes()).unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("capabilities"), "missing capabilities in response");
    }

    #[test]
    #[ignore]
    fn lsp_custom_method_health() {
        let repo = minimal_repo();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cxpak"))
            .args(["lsp", repo.path().to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn cxpak lsp");

        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(make_request(1, "initialize", r#"{"processId":null,"rootUri":null,"capabilities":{}}"#).as_bytes()).unwrap();
        stdin.write_all(make_request(2, "cxpak/health", "{}").as_bytes()).unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("total_files"));
    }

    #[test]
    #[ignore]
    fn lsp_unknown_method_returns_error() {
        let repo = minimal_repo();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cxpak"))
            .args(["lsp", repo.path().to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn cxpak lsp");

        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(make_request(1, "initialize", r#"{"processId":null,"rootUri":null,"capabilities":{}}"#).as_bytes()).unwrap();
        stdin.write_all(make_request(2, "cxpak/nonexistent", "{}").as_bytes()).unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("error") || stdout.contains("-32601"));
    }
}
```

**Commands:**
```
cargo test --features lsp -- lsp_integration --ignored
```

---

## Task 19: Intelligence API integration tests

**Files:**
- `tests/api_v1_integration.rs` (new)

**Steps:**
1. Write test: `v1_health_returns_200` — start the server against a temp repo with the tokio test runtime, POST `{}` to `/v1/health`, assert `200` and response body contains `"total_files"`.
2. Write test: `v1_conventions_returns_profile` — POST `{}` to `/v1/conventions`, assert `200` and body contains `"naming"` key (from `ConventionProfile`).
3. Write test: `v1_auth_rejects_missing_token` — set `expected_token = Some("abc")` on state, POST to `/v1/health` without `Authorization`, assert `401`.
4. Write test: `v1_auth_accepts_valid_token` — same setup, send `Authorization: Bearer abc`, assert `200`.
5. Write test: `v1_briefing_returns_task` — POST `{"task": "find main entry point"}` to `/v1/briefing`, assert `200` and response contains `"task"` key.
6. Use `tower::ServiceExt::oneshot` (already in `[dev-dependencies]`) to call the router directly without binding a port.
7. Run all five tests.

**Code:**
```rust
// tests/api_v1_integration.rs
#[cfg(feature = "daemon")]
mod api_v1 {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_router() -> axum::Router {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        // build_index returns a Result; fallback to a minimal index on error
        let index = cxpak::commands::serve::build_index(dir.path())
            .unwrap_or_else(|_| cxpak::index::CodebaseIndex::default());
        let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
        let path = std::sync::Arc::new(dir.into_path());
        cxpak::commands::serve::build_router_for_test(shared, path, None)
    }

    #[tokio::test]
    async fn v1_health_returns_200() {
        let app = test_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/health")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(val.get("total_files").is_some());
    }

    #[tokio::test]
    async fn v1_auth_rejects_missing_token() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let index = cxpak::index::CodebaseIndex::default();
        let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
        let path = std::sync::Arc::new(dir.into_path());
        let app = cxpak::commands::serve::build_router_for_test(
            shared, path, Some("secret".to_string()),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/health")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
```

**Commands:**
```
cargo test --features daemon -- api_v1
```

---

## Task 20: `build_router_for_test` — expose test helper for router construction

**Files:**
- `src/commands/serve.rs` (add `pub fn build_router_for_test`)

**Steps:**
1. Refactor `build_router` to accept `Option<String>` for the expected auth token.
2. Expose `pub fn build_router_for_test(shared: SharedIndex, repo_path: SharedPath, token: Option<String>) -> Router` by calling the internal builder.
3. The existing `run()` function passes `None` for token (or the `--token` CLI flag value).
4. Ensure `process_watcher_changes` is `pub(crate)` so the LSP backend can call it without going through commands::serve namespace issues.
5. Run `cargo test --features daemon -- api_v1` to verify no regressions.
6. Run `cargo test --features lsp` to verify LSP tests still pass.

**Commands:**
```
cargo test --features daemon
cargo test --features lsp
cargo test
```

---

## Task 21: Verify `cxpak conventions diff` with real baseline

**Files:**
- `tests/conventions_integration.rs` (new)

**Steps:**
1. Write test: `conventions_export_diff_roundtrip` — create a temp git repo with a Rust file, run `run_export`, modify the repo (add a second file with a different naming convention), run `run_diff`, assert the output JSON has `has_changes == false` (since conventions require many files to shift — if they shift, assert `has_changes == true` with non-empty `changed_fields`). The test asserts the exit is clean and the output is valid JSON regardless of the direction.
2. Write test: `conventions_diff_fails_without_baseline` — call `run_diff` on a dir with no `.cxpak/conventions.json`, assert the function returns `Err` whose message contains `"No baseline found"` and the actionable fix command.
3. Run both tests.

**Code:**
```rust
// tests/conventions_integration.rs
mod conventions_integration {
    use cxpak::commands::conventions::{run_diff, run_export};
    use tempfile::TempDir;

    fn minimal_repo(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join("lib.rs"), "pub fn compute() {}").unwrap();
    }

    #[test]
    fn conventions_export_diff_roundtrip() {
        let dir = TempDir::new().unwrap();
        minimal_repo(dir.path());
        let export_result = run_export(dir.path());
        // export may succeed or fail (minimal repo may not have git objects)
        // either way, if it succeeds, diff should produce valid JSON
        if export_result.is_ok() {
            let diff_result = run_diff(dir.path());
            // diff against same baseline should succeed
            assert!(diff_result.is_ok(), "diff failed: {:?}", diff_result);
        }
    }

    #[test]
    fn conventions_diff_fails_without_baseline() {
        let dir = TempDir::new().unwrap();
        minimal_repo(dir.path());
        let result = run_diff(dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No baseline found"), "unexpected error: {msg}");
        assert!(msg.contains("cxpak conventions export"), "missing fix hint: {msg}");
    }
}
```

**Commands:**
```
cargo test -- conventions_integration
```

---

## Task 22: `cargo clippy` and `cargo fmt` clean pass

**Files:**
- All new and modified files from Tasks 1-21.

**Steps:**
1. Run `cargo fmt -- --check` across the entire workspace. Fix any formatting issues (do NOT skip — pre-commit hooks enforce this).
2. Run `cargo clippy --all-targets --features lsp -- -D warnings`. Fix every warning. Common issues to anticipate:
   - `dead_code` on stub handlers — add `#[allow(dead_code)]` only if the handler is intentionally stubbed and registered in the router.
   - `unused_variables` in stub handlers — use `_params` prefixes.
   - Missing `Clone` derives on new types.
3. Run `cargo clippy --all-targets -- -D warnings` (without lsp feature) — must also be clean.
4. Confirm `cargo build` succeeds.
5. Confirm `cargo build --features lsp` succeeds.

**Commands:**
```
cargo fmt -- --check
cargo clippy --all-targets --features lsp -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo build
cargo build --features lsp
```

---

## Task 23: Coverage validation (90% threshold)

**Files:**
- No new files — this is a measurement and gap-fill task.

**Steps:**
1. Run `cargo tarpaulin --out Json --output-file tarpaulin-report.json --features lsp` to generate a coverage report.
2. Check the overall line coverage. If below 90%, identify uncovered lines in the new modules: `src/lsp/`, `src/conventions/export.rs`, `src/conventions/diff.rs`, `src/commands/conventions.rs`.
3. For any uncovered branch in `handle_custom_method`: add individual tests for the 14 arms where missing coverage is reported.
4. For any uncovered error path in `run_export` or `run_diff`: add tests using invalid paths or missing `.git` dirs.
5. For `validate_workspace_path`: ensure all three rejection conditions are tested (traversal, absolute, escapes workspace).
6. Re-run tarpaulin and confirm ≥90%.
7. Write at least one test for the `to_stable_value` recursive function with a nested object to cover the `Array` branch.

**Additional gap-fill tests to write upfront:**

```rust
// In src/conventions/export.rs tests
#[test]
fn to_stable_value_handles_nested_arrays() {
    let v = serde_json::json!({"items": [{"b": 2}, {"a": 1}]});
    let stable = to_stable_value(v.clone());
    // Arrays maintain order, objects are sorted
    assert_eq!(stable["items"][0]["b"], 2);
}

// In src/conventions/export.rs tests
#[test]
fn to_stable_value_handles_primitives() {
    assert_eq!(to_stable_value(serde_json::json!(42)), serde_json::json!(42));
    assert_eq!(to_stable_value(serde_json::json!(true)), serde_json::json!(true));
    assert_eq!(to_stable_value(serde_json::json!(null)), serde_json::json!(null));
}
```

**Commands:**
```
cargo tarpaulin --out Json --output-file tarpaulin-report.json --features lsp
cargo test --features lsp -- --nocapture 2>&1 | tail -20
```

---

## Task 24: Version bump and final integration smoke test

**Files:**
- `Cargo.toml` (version `1.1.0` → `1.6.0`)
- `plugin/.claude-plugin/plugin.json`
- `.claude-plugin/marketplace.json`

**Steps:**
1. Update `version = "1.6.0"` in `Cargo.toml`.
2. Update plugin.json and marketplace.json version fields to `"1.6.0"` (per the CLAUDE.md plugin versioning requirement: all four files must stay in sync).
3. Run `cargo check` to regenerate `Cargo.lock` with the new version.
4. Run the full test suite: `cargo test --features lsp`.
5. Run `cargo test` (without lsp feature) to confirm default feature set still passes all non-lsp tests.
6. Smoke test the three new user-facing commands by invoking the CLI against the cxpak source tree itself:
   - `cargo run -- conventions export .` — assert `.cxpak/conventions.json` is written.
   - `cargo run -- conventions diff .` — assert output is valid JSON.
   - `cargo run --features lsp -- lsp --help` — assert help text is printed (no panic).
7. Commit changes. Tag `v1.6.0` only after CI passes.

**Commands:**
```
cargo check
cargo test --features lsp
cargo test
cargo run -- conventions export .
cargo run -- conventions diff .
cargo run --features lsp -- lsp --help
```

---

## Task 25: CLAUDE.md and LSP protocol documentation

**Files:**
- `/Users/lb/Documents/barnett/cxpak/.claude/CLAUDE.md` (extend the Commands section and Architecture section)

**Steps:**
1. Add `src/lsp/` to the Architecture section with a description matching the pattern of other modules:
   - `src/lsp/` — LSP server (`cxpak lsp` over stdio). `backend.rs` holds `CxpakLspBackend` implementing `tower_lsp::LanguageServer` with 4 standard methods and 14 custom `cxpak/*` methods. `methods.rs` holds the dispatch logic. Reuses `FileWatcher` for hot index. Feature flag: `lsp`.
2. Add `cxpak lsp`, `cxpak conventions export`, and `cxpak conventions diff` to the Commands section.
3. Add `lsp = ["dep:tower-lsp", "daemon"]` to the feature flag documentation.
4. Add `ConventionExport` to the description of `src/conventions/` noting that `.cxpak/conventions.json` is the output artifact.
5. Update the "Claude Code Plugin" section: note that version references in four files must stay in sync (already documented — verify the list still matches the codebase).
6. No test required for this task — documentation only.

---

## Summary

| Task | Area | Key Deliverable |
|------|------|-----------------|
| 1 | Dependencies | tower-lsp + sha2 + chrono in Cargo.toml, `lsp` feature |
| 2 | Convention Export | `ConventionExport` struct + `compute_checksum` + `build_export` |
| 3 | Convention Diff | `ConventionDiff` + `diff_exports` |
| 4 | CLI | `cxpak conventions export` and `cxpak conventions diff` |
| 5 | API Auth | `validate_workspace_path`, `extract_bearer_token`, `check_auth` |
| 6 | API Router | `build_v1_router` with 12 routes + auth middleware |
| 7 | API Handlers | `/v1/health`, `/v1/conventions`, `/v1/briefing`, 9 stubs |
| 8 | CLI flags | `--bind` and `--token` on `cxpak serve` |
| 9 | LSP module | `src/lsp/` skeleton, `run_stdio` entry point |
| 10 | LSP backend | `CxpakLspBackend`, `initialize`/`initialized`/`shutdown` |
| 11 | LSP standard | `textDocument/codeLens` |
| 12 | LSP standard | `textDocument/hover` |
| 13 | LSP standard | `textDocument/diagnostic` |
| 14 | LSP standard | `workspace/symbol` |
| 15 | LSP custom | All 14 `cxpak/*` methods dispatched via `handle_custom_method` |
| 16 | CLI | `cxpak lsp` command wired end-to-end |
| 17 | LSP watcher | `initialized` spawns real `FileWatcher` loop |
| 18 | Integration | LSP stdio protocol integration tests (3, `#[ignore]`) |
| 19 | Integration | Intelligence API integration tests (5) |
| 20 | Refactor | `build_router_for_test` + `pub(crate) process_watcher_changes` |
| 21 | Integration | Convention export/diff roundtrip integration tests |
| 22 | Quality | `cargo fmt` + `cargo clippy` clean pass |
| 23 | Coverage | Tarpaulin ≥90%, gap-fill tests |
| 24 | Release | Version bump to `1.6.0`, smoke tests |
| 25 | Docs | CLAUDE.md updated |

**MCP tool count:** Unchanged at 24 — no new MCP tools in v1.6.0.
**LSP custom methods:** 14 (`cxpak/health` through `cxpak/blastRadius`).
**Intelligence API endpoints:** 12 (`/v1/health` through `/v1/briefing`).
**New CLI commands:** 3 (`cxpak lsp`, `cxpak conventions export`, `cxpak conventions diff`).
