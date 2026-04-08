//! v1.5.0 "Deep Flow" end-to-end integration test.
//!
//! Builds a small two-language fixture (TypeScript frontend + Python Flask
//! backend) in a tempdir and verifies:
//!
//! 1. Cross-language HTTP boundaries are detected during index build.
//! 2. The detected edges are injected into the dependency graph as
//!    `EdgeType::CrossLanguage(BridgeType::HttpCall)`.
//! 3. `auto_context` surfaces the detected edges in its packed JSON output.

use cxpak::auto_context::{auto_context, AutoContextOpts};
use cxpak::budget::counter::TokenCounter;
use cxpak::index::graph::{BridgeType, EdgeType};
use cxpak::index::CodebaseIndex;
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

/// Build a two-language fixture: a TypeScript file that calls two HTTP
/// routes and two Python Flask handlers for those routes.
fn build_fixture() -> (CodebaseIndex, tempfile::TempDir) {
    let counter = TokenCounter::new();
    let dir = tempfile::TempDir::new().unwrap();

    let ts = dir.path().join("frontend/api.ts");
    std::fs::create_dir_all(ts.parent().unwrap()).unwrap();
    std::fs::write(
        &ts,
        r#"export async function loadUsers() { return fetch("/api/users"); }
export async function createPost(data) { return fetch("/api/posts", { method: "POST", body: data }); }
"#,
    )
    .unwrap();

    let users_py = dir.path().join("backend/users.py");
    std::fs::create_dir_all(users_py.parent().unwrap()).unwrap();
    std::fs::write(
        &users_py,
        "@app.get(\"/api/users\")\ndef get_users():\n    return []\n",
    )
    .unwrap();

    let posts_py = dir.path().join("backend/posts.py");
    std::fs::write(
        &posts_py,
        "@app.post(\"/api/posts\")\ndef create_post():\n    return {}\n",
    )
    .unwrap();

    let files = vec![
        ScannedFile {
            relative_path: "frontend/api.ts".into(),
            absolute_path: ts,
            language: Some("typescript".into()),
            size_bytes: 180,
        },
        ScannedFile {
            relative_path: "backend/users.py".into(),
            absolute_path: users_py,
            language: Some("python".into()),
            size_bytes: 60,
        },
        ScannedFile {
            relative_path: "backend/posts.py".into(),
            absolute_path: posts_py,
            language: Some("python".into()),
            size_bytes: 60,
        },
    ];
    let index = CodebaseIndex::build(files, HashMap::new(), &counter);
    (index, dir)
}

#[test]
fn test_cross_lang_fixture_detects_http_bridges() {
    let (index, _dir) = build_fixture();
    assert!(
        index.cross_lang_edges.len() >= 2,
        "expected at least 2 cross-lang edges, got {}",
        index.cross_lang_edges.len()
    );
    for edge in &index.cross_lang_edges {
        assert_eq!(edge.bridge_type, BridgeType::HttpCall);
        assert_eq!(edge.source_language, "typescript");
        assert_eq!(edge.target_language, "python");
    }
}

#[test]
fn test_cross_lang_fixture_graph_edges() {
    let (index, _dir) = build_fixture();
    let deps = index
        .graph
        .dependencies("frontend/api.ts")
        .expect("frontend/api.ts must have outgoing edges");
    let cross_count = deps
        .iter()
        .filter(|e| matches!(e.edge_type, EdgeType::CrossLanguage(_)))
        .count();
    assert!(
        cross_count >= 2,
        "graph should contain at least 2 CrossLanguage edges from api.ts"
    );
}

#[test]
fn test_cross_lang_fixture_auto_context_surfaces_edges() {
    let (index, _dir) = build_fixture();
    let opts = AutoContextOpts {
        tokens: 20_000,
        focus: None,
        include_tests: false,
        include_blast_radius: false,
        mode: "full".into(),
    };
    let result = auto_context("add error handling to the API", &index, &opts);
    let json = serde_json::to_string(&result.sections).expect("serialize sections");
    assert!(
        json.contains("cross_language"),
        "auto_context output must contain cross_language edges: {json}"
    );
}
