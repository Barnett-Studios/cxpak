//! B1 graph-query: surface-equivalence + determinism integration tests.
//!
//! Proves the LSP surface (`cxpak/graph`) returns the single core
//! `graph_query::execute` result verbatim (no re-derivation), and that the core
//! is byte-deterministic over a real index. The CLI (`commands::graph`) and HTTP
//! (`/v1/graph`) surfaces call the same `execute`, so identical output is a
//! structural consequence; `tests/surface_conformance.rs` covers the projection
//! framework for all four declared surfaces.
#![cfg(feature = "lsp")]

use cxpak::index::CodebaseIndex;
use cxpak::intelligence::graph_query;
use serde_json::json;

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
