//! B1 graph-query: surface-equivalence + determinism integration tests.
//!
//! Proves the LSP surface (`cxpak/graph`) returns the single core
//! `graph_query::execute` result verbatim (no re-derivation), and that the core
//! is byte-deterministic over a real index. The CLI (`commands::graph`) and HTTP
//! (`/v1/graph`) surfaces call the same `execute`, so identical output is a
//! structural consequence; `tests/surface_conformance.rs` covers the projection
//! framework for all four declared surfaces.
#![cfg(all(feature = "daemon", feature = "lsp"))]

use axum::body::Body;
use axum::http::Request;
use cxpak::index::CodebaseIndex;
use cxpak::intelligence::graph_query;
use serde_json::json;
use tower::ServiceExt;

/// A tiny two-file Rust index where `main.rs` imports `lib.rs`, so the graph has
/// a real edge to query.
fn sample_index() -> CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![
        cxpak::scanner::ScannedFile {
            relative_path: "src/main.rs".into(),
            absolute_path: "/tmp/src/main.rs".into(),
            language: Some("rust".into()),
            size_bytes: 64,
        },
        cxpak::scanner::ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: "/tmp/src/lib.rs".into(),
            language: Some("rust".into()),
            size_bytes: 64,
        },
    ];
    let mut content = std::collections::HashMap::new();
    content.insert(
        "src/main.rs".to_string(),
        "mod lib;\nfn main(){}".to_string(),
    );
    content.insert("src/lib.rs".to_string(), "pub fn helper(){}".to_string());
    CodebaseIndex::build_with_content(files, std::collections::HashMap::new(), &counter, content)
}

#[test]
fn lsp_graph_returns_core_verbatim() {
    let idx = sample_index();
    let root = std::path::Path::new("/tmp");
    let queries = [
        json!({"op": "node", "id": "src/main.rs"}),
        json!({"op": "neighbors", "id": "src/main.rs", "direction": "both"}),
        json!({"op": "path", "from": "src/main.rs", "to": "src/lib.rs"}),
        json!({"op": "subgraph", "seeds": ["src/main.rs"], "depth": 2}),
    ];
    for q in queries {
        let op = q.get("op").and_then(|v| v.as_str()).unwrap();
        let core = graph_query::execute(&idx.graph, op, &q).expect("core ok");
        let via_lsp =
            cxpak::lsp::methods::handle_custom_method("cxpak/graph", q.clone(), &idx, root)
                .expect("lsp ok")
                .expect("lsp returns Some");
        assert_eq!(
            via_lsp, core,
            "LSP must return the core result verbatim for {op}"
        );
    }
}

#[test]
fn lsp_graph_missing_op_is_internal_error() {
    let idx = sample_index();
    let root = std::path::Path::new("/tmp");
    let err = cxpak::lsp::methods::handle_custom_method(
        "cxpak/graph",
        json!({"id": "src/main.rs"}),
        &idx,
        root,
    );
    assert!(err.is_err(), "missing op must be a JSON-RPC error (-32603)");
}

#[test]
fn core_is_byte_deterministic_over_real_index() {
    let idx = sample_index();
    let q = json!({"op": "subgraph", "seeds": ["src/main.rs", "src/lib.rs"], "depth": 3});
    let first =
        serde_json::to_string(&graph_query::execute(&idx.graph, "subgraph", &q).unwrap()).unwrap();
    for _ in 0..25 {
        let again =
            serde_json::to_string(&graph_query::execute(&idx.graph, "subgraph", &q).unwrap())
                .unwrap();
        assert_eq!(
            again, first,
            "subgraph output must be byte-identical every run"
        );
    }
}

// ---------------------------------------------------------------------------
// N20.5 — cross-surface parity for `nodes` and `subgraph.unknown_seeds`
// (ADR-0202, issue #20). CLI, HTTP `/v1/graph`, LSP `cxpak/graph`, and MCP
// `cxpak_graph` all route through the single `graph_query::execute` core, so
// this is a structural consequence, not independent logic — the test proves
// it holds rather than re-deriving it. Compared on PARSED `serde_json::Value`
// (never raw bytes): HTTP returns compact JSON, CLI/MCP pretty-print, so a
// byte comparison would fail on whitespace alone despite identical content.

/// A real on-disk git repo with a genuine cross-file `use crate::` edge (the
/// CLI leg re-scans from disk; the ADR-0203 `main.rs` self-package gap means
/// a bare `mod` declaration resolves no edges, so the import must be
/// `crate::`-qualified for `main.rs` to be a node at all).
fn fixture_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "mod helper_mod;\nuse crate::helper_mod::helper;\nfn main() { helper(); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/helper_mod.rs"), "pub fn helper() {}\n").unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();

    dir
}

/// Run `cxpak graph <op> [args...] <path>` as a real subprocess and parse
/// stdout as JSON.
fn cli_graph(path: &std::path::Path, extra_args: &[&str]) -> serde_json::Value {
    let mut args: Vec<&str> = vec!["graph"];
    args.extend_from_slice(extra_args);
    let path_str = path.to_str().unwrap();
    args.push(path_str);
    let output = std::process::Command::new(assert_cmd::cargo_bin!("cxpak"))
        .args(&args)
        .output()
        .expect("cxpak graph subprocess must run");
    assert!(
        output.status.success(),
        "cxpak graph {extra_args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("cxpak graph {extra_args:?} stdout not JSON ({e})"))
}

/// Drive the MCP `tools/call` channel and parse the embedded JSON payload out
/// of the JSON-RPC envelope (`result.content[0].text`, per MCP spec).
fn call_mcp_graph(idx: &CodebaseIndex, args: serde_json::Value) -> serde_json::Value {
    let snapshot: cxpak::commands::serve::SharedSnapshot =
        std::sync::Arc::new(std::sync::RwLock::new(None));
    let envelope = cxpak::commands::serve::handle_tool_call(
        Some(json!(1)),
        "cxpak_graph",
        &args,
        idx,
        std::path::Path::new("/tmp"),
        &snapshot,
    );
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("MCP cxpak_graph envelope missing result.content[0].text: {envelope}")
        });
    serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("MCP cxpak_graph content text not JSON ({e}): {text}"))
}

async fn call_http_graph(idx: CodebaseIndex, body: serde_json::Value) -> serde_json::Value {
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
    let path = std::sync::Arc::new(std::path::PathBuf::from("/tmp"));
    let app = cxpak::commands::serve::build_router_for_test(shared, path);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/graph")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn nodes_op_is_identical_across_cli_http_lsp_mcp() {
    let repo = fixture_repo();
    let idx = cxpak::commands::serve::build_index(repo.path()).expect("index builds");

    let cli = cli_graph(repo.path(), &["nodes"]);
    let http = call_http_graph(idx.clone(), json!({"op": "nodes"})).await;
    let lsp = cxpak::lsp::methods::handle_custom_method(
        "cxpak/graph",
        json!({"op": "nodes"}),
        &idx,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .expect("Some");
    let mcp = call_mcp_graph(&idx, json!({"op": "graph", "graph_op": "nodes"}));

    assert_eq!(cli, http, "CLI vs HTTP `nodes` parity");
    assert_eq!(http, lsp, "HTTP vs LSP `nodes` parity");
    assert_eq!(lsp, mcp, "LSP vs MCP `nodes` parity");
    // Sanity: this isn't an accidental all-empty pass.
    assert!(
        !cli["nodes"].as_array().unwrap().is_empty(),
        "fixture must have at least one node"
    );
}

#[tokio::test]
async fn subgraph_unknown_seeds_is_identical_across_cli_http_lsp_mcp() {
    let repo = fixture_repo();
    let idx = cxpak::commands::serve::build_index(repo.path()).expect("index builds");

    let cli = cli_graph(
        repo.path(),
        &["subgraph", "--seeds", "totally/bogus.xyz,src/main.rs"],
    );
    let http = call_http_graph(
        idx.clone(),
        json!({"op": "subgraph", "seeds": ["totally/bogus.xyz", "src/main.rs"], "depth": 1}),
    )
    .await;
    let lsp = cxpak::lsp::methods::handle_custom_method(
        "cxpak/graph",
        json!({"op": "subgraph", "seeds": ["totally/bogus.xyz", "src/main.rs"], "depth": 1}),
        &idx,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .expect("Some");
    let mcp = call_mcp_graph(
        &idx,
        json!({"op": "graph", "graph_op": "subgraph", "seeds": ["totally/bogus.xyz", "src/main.rs"], "depth": 1}),
    );

    assert_eq!(cli, http, "CLI vs HTTP `subgraph` parity");
    assert_eq!(http, lsp, "HTTP vs LSP `subgraph` parity");
    assert_eq!(lsp, mcp, "LSP vs MCP `subgraph` parity");
    assert_eq!(
        cli["unknown_seeds"],
        json!(["totally/bogus.xyz"]),
        "bogus seed must land in unknown_seeds on every surface"
    );
    assert!(
        !cli["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "totally/bogus.xyz"),
        "the bogus seed must never be emitted as a node"
    );
}

#[test]
fn graph_capability_description_mentions_nodes_and_unknown_seeds() {
    let graph_cap = cxpak::capability::catalog()
        .iter()
        .find(|c| c.id == "graph")
        .expect("graph capability is registered");
    assert!(
        graph_cap.summary.contains("nodes"),
        "graph capability description must mention the `nodes` op: {}",
        graph_cap.summary
    );
    assert!(
        graph_cap.summary.contains("unknown_seeds"),
        "graph capability description must mention `unknown_seeds`: {}",
        graph_cap.summary
    );
}
