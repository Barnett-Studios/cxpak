//! C1 retrieval: surface-equivalence + determinism + readOnly integration tests.
//!
//! Proves the LSP surface (`cxpak/retrieval`) returns the single core
//! `retrieval::execute` result verbatim (no re-derivation), that the core is
//! byte-deterministic over a real index across the full search→references→expand
//! loop, that results come from cxpak's OWN index (no external language server),
//! and that the retrieval capability carries the `readOnly` annotation on both
//! the catalog and the LSP method. The CLI (`commands::search`), HTTP
//! (`/v1/retrieval`) and MCP (`cxpak_context` `op=retrieval`) surfaces call the
//! same `execute`, so identical output is a structural consequence;
//! `tests/surface_conformance.rs` covers the projection framework for all four
//! declared surfaces.
#![cfg(feature = "lsp")]

use cxpak::index::CodebaseIndex;
use cxpak::intelligence::retrieval;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use serde_json::json;
use std::collections::HashMap;

/// Two-file Rust index with real symbols and a `main.rs → lib.rs` import, so
/// search, references, and expand all have index-derived results.
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

    let mut parse_results = HashMap::new();
    parse_results.insert(
        "src/main.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: "run_search".to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "fn run_search()".to_string(),
                body: "fn run_search() { helper(); }".to_string(),
                start_line: 3,
                end_line: 9,
            }],
            imports: vec![cxpak::parser::language::Import {
                source: "crate::lib".to_string(),
                names: vec!["lib".to_string()],
            }],
            exports: vec![],
        },
    );
    parse_results.insert(
        "src/lib.rs".to_string(),
        ParseResult {
            symbols: vec![
                Symbol {
                    name: "search".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn search()".to_string(),
                    body: "fn search() {}".to_string(),
                    start_line: 1,
                    end_line: 2,
                },
                Symbol {
                    name: "helper".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "fn helper()".to_string(),
                    body: "fn helper() {}".to_string(),
                    start_line: 4,
                    end_line: 5,
                },
            ],
            imports: vec![],
            exports: vec![],
        },
    );

    let mut content = HashMap::new();
    content.insert(
        "src/main.rs".to_string(),
        "mod lib;\nuse crate::lib;\nfn run_search() { helper(); }".to_string(),
    );
    content.insert(
        "src/lib.rs".to_string(),
        "pub fn search() {}\npub fn helper() {}".to_string(),
    );
    CodebaseIndex::build_with_content(files, parse_results, &counter, content)
}

#[test]
fn lsp_retrieval_returns_core_verbatim() {
    let idx = sample_index();
    let root = std::path::Path::new("/tmp");
    let queries = [
        json!({"op": "search", "query": "search"}),
        json!({"op": "references", "symbol": "helper"}),
        json!({"op": "expand", "seeds": ["src/main.rs"], "depth": 2}),
    ];
    for q in queries {
        let op = q.get("op").and_then(|v| v.as_str()).unwrap();
        let core = retrieval::execute(&idx, op, &q).expect("core ok");
        let via_lsp =
            cxpak::lsp::methods::handle_custom_method("cxpak/retrieval", q.clone(), &idx, root)
                .expect("lsp ok")
                .expect("lsp returns Some");
        assert_eq!(
            via_lsp, core,
            "LSP must return the core retrieval result verbatim for {op}"
        );
    }
}

#[test]
fn lsp_retrieval_missing_op_is_internal_error() {
    let idx = sample_index();
    let root = std::path::Path::new("/tmp");
    let err = cxpak::lsp::methods::handle_custom_method(
        "cxpak/retrieval",
        json!({"query": "search"}),
        &idx,
        root,
    );
    assert!(err.is_err(), "missing op must be a JSON-RPC error (-32603)");
}

#[test]
fn retrieval_loop_is_byte_deterministic_over_real_index() {
    // The C1 contract: search → references → expand, chained, byte-identical
    // across runs AND matching a locked expected ordering. Results come from
    // cxpak's own index — no external language server is consulted.
    let idx = sample_index();

    let run_loop = || -> String {
        let s = retrieval::execute(&idx, "search", &json!({"query": "helper"})).unwrap();
        let refs = retrieval::execute(&idx, "references", &json!({"symbol": "helper"})).unwrap();
        let seeds: Vec<&str> = refs["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let ex = retrieval::execute(&idx, "expand", &json!({"seeds": seeds, "depth": 2})).unwrap();
        format!("{s}||{refs}||{ex}")
    };

    let first = run_loop();
    for _ in 0..50 {
        assert_eq!(run_loop(), first, "retrieval loop must be reproducible");
    }

    // Lock the expected ordering: `helper` is referenced in both files, sorted.
    let refs = retrieval::execute(&idx, "references", &json!({"symbol": "helper"})).unwrap();
    assert_eq!(refs["files"], json!(["src/lib.rs", "src/main.rs"]));

    // search for "search" ranks the exact `search` symbol first (own index).
    let s = retrieval::execute(&idx, "search", &json!({"query": "search"})).unwrap();
    assert_eq!(s["hits"][0]["symbol"], json!("search"));
    assert_eq!(s["hits"][0]["path"], json!("src/lib.rs"));
}

#[test]
fn retrieval_capability_and_lsp_method_are_read_only() {
    // readOnly annotation present on BOTH the catalog capability and the LSP
    // method (the plan requires readOnly on the retrieval capabilities/methods).
    let cap = cxpak::capability::catalog()
        .iter()
        .find(|c| c.id == "retrieval")
        .expect("retrieval capability present");
    assert!(
        cap.read_only,
        "catalog retrieval capability must be read-only"
    );
    assert!(
        cxpak::lsp::methods::method_is_read_only("cxpak/retrieval"),
        "cxpak/retrieval LSP method must carry the read-only annotation"
    );
}
