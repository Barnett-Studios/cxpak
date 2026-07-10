//! C3 (ADR-0182) live MCP surface gates.
//!
//! `tests/mcp_tool_budget.rs` checks the capability adapter *in isolation*.
//! This suite asserts the property that matters after C3: the **live**
//! `serve.rs` MCP `tools/list` IS the adapter's ≤8 intent-tool projection, and
//! every one of the 26 former hand-rolled MCP tools is reachable through some
//! `(intent-tool, op)` pair that routes to a real capability core (the
//! "no dropped functionality" gate).

use cxpak::budget::counter::TokenCounter;
use cxpak::capability::adapter::mcp_catalog_tools;
use cxpak::commands::serve::{handle_tool_call, mcp_stdio_loop_with_io, SharedSnapshot};
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A tiny real index: two linked Rust modules (so the dependency graph has
/// edges for the `graph`/`trace`/`blast_radius` ops) plus a SQL table (so the
/// `data` op has a populated `SchemaIndex`).
fn make_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile {
            relative_path: "src/mod_0.rs".into(),
            absolute_path: "/tmp/src/mod_0.rs".into(),
            language: Some("rust".into()),
            size_bytes: 200,
        },
        ScannedFile {
            relative_path: "src/mod_1.rs".into(),
            absolute_path: "/tmp/src/mod_1.rs".into(),
            language: Some("rust".into()),
            size_bytes: 200,
        },
        ScannedFile {
            relative_path: "db/users.sql".into(),
            absolute_path: "/tmp/db/users.sql".into(),
            language: Some("sql".into()),
            size_bytes: 120,
        },
    ];

    let mut pr = HashMap::new();
    pr.insert(
        "src/mod_0.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: "handle_request".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn handle_request()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 3,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    pr.insert(
        "src/mod_1.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: "call_handler".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn call_handler()".into(),
                body: "{ handle_request() }".into(),
                start_line: 1,
                end_line: 3,
            }],
            imports: vec![Import {
                source: "crate::mod_0".into(),
                names: vec!["handle_request".into()],
            }],
            exports: vec![],
        },
    );

    let mut content = HashMap::new();
    content.insert(
        "src/mod_0.rs".to_string(),
        "pub fn handle_request(){}".into(),
    );
    content.insert(
        "src/mod_1.rs".to_string(),
        "pub fn call_handler(){ handle_request() }".into(),
    );
    content.insert(
        "db/users.sql".to_string(),
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);".into(),
    );

    CodebaseIndex::build_with_content(files, pr, &counter, content)
}

fn snapshot() -> SharedSnapshot {
    Arc::new(RwLock::new(None))
}

/// Drive the real `serve.rs` MCP stdio loop with a `tools/list` request and
/// return the advertised tool objects.
fn live_tools_list(idx: &CodebaseIndex) -> Vec<Value> {
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n";
    let mut out = Vec::new();
    mcp_stdio_loop_with_io(
        std::path::Path::new("/tmp"),
        idx,
        &snapshot(),
        &input[..],
        &mut out,
    )
    .unwrap();
    let resp: Value = serde_json::from_slice(&out).unwrap();
    resp["result"]["tools"].as_array().unwrap().clone()
}

#[test]
fn live_tools_list_equals_adapter_projection_within_budget() {
    let idx = make_index();
    let tools = live_tools_list(&idx);

    // ≤8 on the LIVE surface (not just the adapter in isolation).
    assert!(
        tools.len() <= 8,
        "live MCP tools/list must be ≤8; got {}",
        tools.len()
    );

    // The live names, in order, must equal the adapter's projection exactly.
    let live: Vec<String> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let projected: Vec<String> = mcp_catalog_tools().iter().map(|t| t.name.clone()).collect();
    assert_eq!(
        live, projected,
        "live serve.rs tools/list must equal the capability adapter projection"
    );

    // Every intent-tool advertises read-only + a required `op` selector whose
    // enum matches the adapter's ops (C1 readOnly wiring + deterministic order).
    for (t, adapter_tool) in tools.iter().zip(mcp_catalog_tools()) {
        assert_eq!(
            t["annotations"]["readOnlyHint"],
            json!(true),
            "{} must advertise readOnlyHint=true",
            adapter_tool.name
        );
        assert_eq!(t["inputSchema"]["required"][0], "op");
        let enum_ops: Vec<String> = t["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            enum_ops, adapter_tool.ops,
            "{} op enum must equal adapter ops",
            adapter_tool.name
        );
    }
}

/// The enumerated 26 → (intent-tool, op) migration table. This IS the
/// "no dropped functionality" contract, asserted against the live surface.
fn legacy_migration_table() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        // (former cxpak_* tool, intent-tool, op)
        ("cxpak_auto_context", "cxpak_context", "context"),
        ("cxpak_context_diff", "cxpak_review", "review"),
        ("cxpak_overview", "cxpak_context", "overview"),
        ("cxpak_trace", "cxpak_graph", "trace"),
        ("cxpak_diff", "cxpak_review", "diff"),
        ("cxpak_stats", "cxpak_context", "stats"),
        (
            "cxpak_context_for_task",
            "cxpak_context",
            "context_for_task",
        ),
        ("cxpak_pack_context", "cxpak_context", "pack_context"),
        ("cxpak_search", "cxpak_context", "search"),
        ("cxpak_blast_radius", "cxpak_graph", "blast_radius"),
        ("cxpak_api_surface", "cxpak_graph", "api_surface"),
        ("cxpak_verify", "cxpak_review", "verify"),
        ("cxpak_conventions", "cxpak_insight", "conventions"),
        ("cxpak_health", "cxpak_insight", "health"),
        ("cxpak_risks", "cxpak_insight", "risks"),
        ("cxpak_briefing", "cxpak_context", "briefing"),
        ("cxpak_call_graph", "cxpak_graph", "call_graph"),
        ("cxpak_dead_code", "cxpak_graph", "dead_code"),
        ("cxpak_architecture", "cxpak_insight", "architecture"),
        ("cxpak_predict", "cxpak_graph", "predict"),
        ("cxpak_drift", "cxpak_insight", "drift"),
        (
            "cxpak_security_surface",
            "cxpak_insight",
            "security_surface",
        ),
        ("cxpak_data_flow", "cxpak_graph", "data_flow"),
        ("cxpak_cross_lang", "cxpak_graph", "cross_lang"),
        ("cxpak_visual", "cxpak_insight", "visual"),
        ("cxpak_onboard", "cxpak_insight", "onboard"),
    ]
}

/// Rich argument bag covering the union of required params across ops, so a
/// reachability call actually reaches the capability core rather than bouncing
/// off a missing-param guard.
fn rich_args(op: &str) -> Value {
    json!({
        "op": op,
        "task": "trace the request handler",
        "target": "handle_request",
        "symbol": "handle_request",
        "pattern": "handle_request",
        "files": ["src/mod_1.rs"],
        "graph_op": "neighbors",
        "id": "src/mod_1.rs",
        "direction": "both",
        "retrieval_op": "search",
        "query": "handle_request",
        "focus": "src/",
    })
}

#[test]
fn every_legacy_tool_reachable_via_intent_op() {
    let idx = make_index();
    let tools = live_tools_list(&idx);
    let snap = snapshot();

    for (legacy, intent_tool, op) in legacy_migration_table() {
        // 1. The op is advertised under the intent-tool on the live surface.
        let tool = tools
            .iter()
            .find(|t| t["name"] == intent_tool)
            .unwrap_or_else(|| panic!("intent-tool {intent_tool} missing from live surface"));
        let ops: Vec<&str> = tool["inputSchema"]["properties"]["op"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            ops.contains(&op),
            "{legacy} → ({intent_tool}, {op}) but op not advertised; ops={ops:?}"
        );

        // 2. Calling (intent-tool, op) routes to a real capability core: it
        //    returns a tool result, never an unknown-tool / invalid-op error.
        let resp = handle_tool_call(
            Some(json!(1)),
            intent_tool,
            &rich_args(op),
            &idx,
            std::path::Path::new("/tmp"),
            &snap,
        );
        assert_ne!(
            resp["error"]["code"], -32601,
            "{legacy} → ({intent_tool}, {op}) resolved to unknown tool: {resp}"
        );
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| {
                panic!("{legacy} → ({intent_tool},{op}) produced no result: {resp}")
            });
        assert!(
            !text.starts_with("Error: op ") && !text.contains("requires an 'op' argument"),
            "{legacy} → ({intent_tool},{op}) failed op resolution: {text}"
        );
    }
}

#[test]
fn legacy_tool_names_still_route_as_deprecated_aliases() {
    // Backward-compatibility: the 26 removed tool NAMES are undiscoverable but
    // still route to the same core (documented deprecation, see MIGRATION-3.0).
    let idx = make_index();
    let snap = snapshot();
    for (legacy, _intent, _op) in legacy_migration_table() {
        let resp = handle_tool_call(
            Some(json!(1)),
            legacy,
            &rich_args("_ignored_for_legacy_"),
            &idx,
            std::path::Path::new("/tmp"),
            &snap,
        );
        assert_ne!(
            resp["error"]["code"], -32601,
            "legacy alias {legacy} must still route, got: {resp}"
        );
        assert!(
            resp["result"]["content"][0]["text"].is_string(),
            "legacy alias {legacy} must return a tool result: {resp}"
        );
    }
}

#[test]
fn graph_op_surfaces_edge_type_and_confidence_a3() {
    // A3 (ADR-0175) folded in: the `graph` capability op exposes per-edge
    // `edge_type` + `confidence` (`inferred`) on the graph surface.
    let idx = make_index();
    let snap = snapshot();
    let resp = handle_tool_call(
        Some(json!(1)),
        "cxpak_graph",
        &json!({"op": "graph", "graph_op": "neighbors", "id": "src/mod_1.rs", "direction": "both"}),
        &idx,
        std::path::Path::new("/tmp"),
        &snap,
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let core: Value = serde_json::from_str(text).unwrap();
    let neighbors = core["neighbors"].as_array().expect("neighbors array");
    assert!(
        !neighbors.is_empty(),
        "mod_1 depends on mod_0, expected an edge"
    );
    let edge = &neighbors[0];
    assert!(
        edge["edge_type"].is_string(),
        "edge must carry edge_type: {edge}"
    );
    assert!(
        edge["confidence"].is_string(),
        "edge must carry confidence: {edge}"
    );
    assert!(
        edge["inferred"].is_boolean(),
        "edge must carry inferred flag: {edge}"
    );
}

#[test]
fn data_op_surfaces_schema_index_b1_m2() {
    // B1 M2 folded in: the `data` capability is now MCP-surfaced and returns the
    // indexed data layer (`SchemaIndex`), not a stub.
    let idx = make_index();
    let snap = snapshot();
    let resp = handle_tool_call(
        Some(json!(1)),
        "cxpak_data",
        &json!({"op": "data"}),
        &idx,
        std::path::Path::new("/tmp"),
        &snap,
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let core: Value = serde_json::from_str(text).unwrap();
    assert!(
        core["indexed"].is_boolean(),
        "data core must report `indexed`: {core}"
    );
    assert!(
        core["tables"].is_array(),
        "data core must carry a tables array: {core}"
    );
    // The fixture defines a `users` table; if schema detection ran, it appears.
    if core["indexed"] == json!(true) {
        let tables = core["tables"].as_array().unwrap();
        assert!(
            tables.iter().any(|t| t["name"] == "users"),
            "indexed data layer should include the `users` table: {core}"
        );
    }
}
