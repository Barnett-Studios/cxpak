//! Adversarial tests closing the 8 round-4 pending-issues defects.
//!
//! - #1 standard LSP methods now use the snapshot-then-release pattern
//! - #2 `--token ""` rejected at startup
//! - #3 IPv4-mapped IPv6 loopback pinned (covered in serve_security.rs)
//! - #5 architecture module names + god_files bidi-sanitised
//! - #7 clipboard .catch present (covered in spa_golden update)
//! - #9 7 previously-dead handler params now wired
//! - #12 DependencyGraph::edge_count() shared helper
//! - #13 feature-matrix script presence (covered by scripts/feature-matrix.sh)
#![cfg(all(feature = "visual", feature = "daemon", feature = "lsp"))]

use serde_json::json;

// ── #1: standard LSP handlers use snapshot pattern ──────────────────────────

#[test]
fn standard_lsp_handlers_use_snapshot_helper() {
    // Source-level pin: code_lens / hover / diagnostic / symbol must each
    // call self.snapshot()? rather than self.index.read().  Without this
    // pin a refactor that re-introduces the long-held read guard would
    // pass tests (the index is already a snapshot during single-test
    // runs) and only manifest under concurrent watcher load in production.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lsp/backend.rs"))
        .expect("read backend.rs");
    let snapshot_count = src.matches("self.snapshot()?").count();
    assert!(
        snapshot_count >= 4,
        "code_lens / hover / diagnostic / symbol must each go through self.snapshot(); \
         expected >=4 occurrences, found {snapshot_count}"
    );
    // Pin: NO bare `self.index.read()` should remain in the LanguageServer
    // impl block — the snapshot helper is the single read-side entry point.
    // We accept the helper itself (which contains the literal) but no
    // other site.
    let bare = src.matches("self.index.read()").count();
    assert_eq!(
        bare, 1,
        "exactly one `self.index.read()` should remain — inside the snapshot() helper itself; found {bare}"
    );
}

// ── #5: architecture module + god_files bidi-sanitised ──────────────────────

#[test]
fn architecture_module_prefix_is_bidi_sanitised() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    let counter = TokenCounter::new();
    // Path embeds RLO between module segments.
    let evil = "src/admin\u{202E}//legit.rs";
    let files = vec![
        ScannedFile {
            relative_path: evil.into(),
            absolute_path: format!("/tmp/{evil}").into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
        // A second file in the same module so it has >=1 file count.
        ScannedFile {
            relative_path: format!("{}/x.rs", evil.split('/').next().unwrap_or("src")),
            absolute_path: "/tmp/src/x.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
    ];
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, HashMap::new());
    let map = cxpak::intelligence::architecture::build_architecture_map(&idx, 2);
    for module in &map.modules {
        assert!(
            !module
                .prefix
                .chars()
                .any(|c| matches!(c, '\u{202E}' | '\u{202D}')),
            "architecture module prefix `{}` contains raw bidi control char — must be sanitised",
            module.prefix
        );
    }
}

// ── #9: previously-dead handler params now actually filter results ──────────

#[tokio::test]
async fn v1_dead_code_workspace_aliases_focus() {
    use axum::body::Body;
    use axum::http::Request;
    use cxpak::budget::counter::TokenCounter;
    use cxpak::commands::serve::build_router_for_test;
    use cxpak::index::CodebaseIndex;
    use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use cxpak::scanner::ScannedFile;
    use std::collections::HashMap;
    use tower::ServiceExt;
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile {
            relative_path: "src/auth/dead.rs".into(),
            absolute_path: "/tmp/src/auth/dead.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
        ScannedFile {
            relative_path: "src/util/dead.rs".into(),
            absolute_path: "/tmp/src/util/dead.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
    ];
    let mut parses = HashMap::new();
    for f in &files {
        parses.insert(
            f.relative_path.clone(),
            ParseResult {
                symbols: vec![Symbol {
                    name: format!("dead_in_{}", f.relative_path.replace('/', "_")),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn x()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
    }
    let mut content = HashMap::new();
    for f in &files {
        content.insert(f.relative_path.clone(), "fn x() {}".into());
    }
    let idx = CodebaseIndex::build_with_content(files, parses, &counter, content);
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
    let router_focus = build_router_for_test(
        shared.clone(),
        std::sync::Arc::new(std::path::PathBuf::from("/tmp")),
    );

    // workspace alias must filter exactly like focus would.
    let req = Request::builder()
        .method("POST")
        .uri("/dead_code")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&json!({"workspace": "src/auth"})).unwrap(),
        ))
        .unwrap();
    let resp = router_focus.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let symbols = body["dead_symbols"].as_array().expect("dead_symbols array");
    for sym in symbols {
        assert!(
            sym["file"].as_str().unwrap_or("").starts_with("src/auth"),
            "workspace=src/auth filter must drop entries outside that prefix; got {sym}"
        );
    }
}

// ── #12: shared edge_count helper ───────────────────────────────────────────

#[test]
fn dependency_graph_exposes_edge_count_helper() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::index::CodebaseIndex;
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    // Method exists and returns 0 on an empty graph.
    assert_eq!(idx.graph.edge_count(), 0);

    // Inlined formulation MUST agree with the helper — pin so the two
    // never drift independently.
    let inlined: usize = idx.graph.edges.values().map(|v| v.len()).sum();
    assert_eq!(idx.graph.edge_count(), inlined);
}

#[test]
fn edge_count_helper_referenced_in_visual_make_metadata_and_test() {
    // Source-level pin: both renderers (commands/visual.rs and the
    // cross-channel parity test) must call `.edge_count()` rather than
    // re-inlining the lambda.  Spec § Contract 8 invariant.
    let visual_src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/commands/visual.rs"
    ))
    .expect("read visual.rs");
    assert!(
        visual_src.contains(".edge_count()"),
        "src/commands/visual.rs::make_metadata must call .edge_count() (Contract 8)"
    );
    let test_src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/cross_channel_consistency.rs"
    ))
    .expect("read cross_channel_consistency.rs");
    assert!(
        test_src.contains(".edge_count()"),
        "tests/cross_channel_consistency.rs::metadata_edge_count_matches_graph_sum must call .edge_count() (Contract 8)"
    );
}

// ── #13: feature-matrix script exists and is executable ─────────────────────

#[test]
fn feature_matrix_script_present_and_executable() {
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("feature-matrix.sh");
    assert!(
        script.exists(),
        "scripts/feature-matrix.sh must exist for #13"
    );
    let metadata = std::fs::metadata(&script).expect("stat feature-matrix.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "feature-matrix.sh must be executable (mode now {:o})",
            mode
        );
    }
    let body = std::fs::read_to_string(&script).expect("read feature-matrix.sh");
    assert!(
        body.contains("--no-default-features"),
        "must test no-default"
    );
    assert!(body.contains("--all-features"), "must test all-features");
    assert!(
        body.contains("--features plugins"),
        "must explicitly test plugins (default-excluded since v2.1.2)"
    );
}
