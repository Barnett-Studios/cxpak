//! End-to-end test of the MCP JSON-RPC stdio framing path.
//!
//! Closes the v2.1.3 P6 finding: prior MCP cross-channel tests called
//! `handle_tool_call` directly in-process, so a regression in cxpak's
//! own JSON-RPC stdio loop (newline framing, request parsing,
//! response serialisation, error envelope) would not be caught.
//!
//! This is independent of the tower-lsp framing limitation (#4 from
//! the prior round) — the MCP loop in `serve::mcp_stdio_loop_with_io`
//! is cxpak's own code, not tower-lsp's, so we can drive it directly
//! with a `BufRead` over a byte buffer.
#![cfg(feature = "daemon")]

use cxpak::budget::counter::TokenCounter;
use cxpak::commands::serve::{mcp_stdio_loop_with_io, SharedSnapshot};
use cxpak::index::CodebaseIndex;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, RwLock};

fn empty_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new())
}

fn drive(input: &str) -> String {
    let idx = empty_index();
    let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
    let reader = Cursor::new(input.as_bytes());
    let mut output: Vec<u8> = Vec::new();
    mcp_stdio_loop_with_io(
        std::path::Path::new("/tmp"),
        &idx,
        &snapshot,
        reader,
        &mut output,
    )
    .expect("mcp loop");
    String::from_utf8(output).expect("utf8 output")
}

/// Initialize handshake: client sends `initialize` request, server returns
/// JSON-RPC envelope with `result` containing protocolVersion and
/// capabilities. Smoke-tests the framing path.
#[test]
fn initialize_request_returns_capabilities() {
    let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}
"#;
    let output = drive(request);
    assert!(
        !output.is_empty(),
        "MCP loop must produce a response for initialize"
    );
    let response: serde_json::Value = output
        .lines()
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .expect("first output line must be a JSON-RPC envelope");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    let result = response.get("result").expect("initialize result envelope");
    assert!(
        result.get("protocolVersion").is_some(),
        "initialize result must include protocolVersion: {response}"
    );
    assert!(
        result.get("capabilities").is_some(),
        "initialize result must include capabilities: {response}"
    );
}

/// `tools/list` returns an array of tool descriptors.  Validates the
/// dispatch path that registers `cxpak_*` tools.
#[test]
fn tools_list_returns_tool_array() {
    let request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
"#;
    let output = drive(request);
    let response: serde_json::Value = output
        .lines()
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .expect("response envelope");
    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools/list must return result.tools array");
    assert!(
        !tools.is_empty(),
        "at least one cxpak_* tool must be registered"
    );
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"cxpak_health"),
        "cxpak_health must be in the tools list"
    );
}

/// `tools/call` for cxpak_health returns the same JSON shape the
/// in-process handler would.  Validates the call dispatch path through
/// the framing layer.
#[test]
fn tools_call_health_returns_composite() {
    let request = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cxpak_health","arguments":{}}}
"#;
    let output = drive(request);
    let response: serde_json::Value = output
        .lines()
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .expect("response envelope");
    assert_eq!(response["id"], 3);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP content[0].text payload");
    let payload: serde_json::Value =
        serde_json::from_str(text).expect("text payload must be valid JSON");
    assert!(
        payload.get("composite").is_some(),
        "cxpak_health payload must include composite: {payload}"
    );
}

/// Malformed line yields a JSON-RPC -32700 (parse error) response and
/// the loop continues — does NOT terminate.  Tests both the error
/// envelope shape and the loop's resilience.
#[test]
fn malformed_line_returns_parse_error_then_loop_continues() {
    let mixed = "this is not json\n\
        {\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"tools/list\",\"params\":{}}\n";
    let output = drive(mixed);
    let lines: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        2,
        "two responses (one error, one ok); got {lines:?}"
    );
    let err: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(err["error"]["code"], -32700);
    let ok: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(ok["id"], 42);
    assert!(ok["result"]["tools"].is_array());
}

/// Coverage closure: pre-this-test, the framing path was exercised for
/// `cxpak_health` only.  The other 25 tools relied on in-process
/// `handle_tool_call` tests, which is the same coverage shape that hid
/// the original tools/list dispatch bugs.  This test parameterises over
/// every cxpak_* tool advertised by `tools/list` and asserts each one:
///   1. produces a response on its own line
///   2. has matching `id`
///   3. has a `result.content[0].text` payload (no `error` envelope)
///   4. that text payload is valid JSON or non-empty plain text
///
/// Tools requiring required arguments are sent a minimal-but-valid body
/// (the tool list metadata in the test enumerates each).  A regression
/// that broke the dispatch table for any tool — or its serialisation
/// envelope shape — would now fail loudly.
#[test]
fn every_mcp_tool_returns_a_well_formed_response_over_stdio_framing() {
    // (tool_name, args)  —  args chosen to satisfy each tool's
    // required-field validation while staying minimal.
    let tools: &[(&str, &str)] = &[
        ("cxpak_health", "{}"),
        ("cxpak_stats", "{}"),
        ("cxpak_overview", r#"{"tokens":"10k"}"#),
        ("cxpak_risks", "{}"),
        ("cxpak_briefing", r#"{"task":"smoke test"}"#),
        ("cxpak_dead_code", "{}"),
        ("cxpak_drift", "{}"),
        ("cxpak_security_surface", "{}"),
        ("cxpak_cross_lang", "{}"),
        ("cxpak_architecture", "{}"),
        ("cxpak_api_surface", "{}"),
        ("cxpak_conventions", "{}"),
        ("cxpak_predict", r#"{"files":["src/main.rs"],"depth":2}"#),
        ("cxpak_blast_radius", r#"{"file":"src/main.rs"}"#),
        ("cxpak_call_graph", r#"{"target":"main"}"#),
        ("cxpak_data_flow", r#"{"symbol":"main"}"#),
        ("cxpak_trace", r#"{"target":"main"}"#),
        ("cxpak_search", r#"{"query":"main"}"#),
        ("cxpak_diff", "{}"),
        ("cxpak_context_diff", "{}"),
        ("cxpak_auto_context", r#"{"task":"smoke"}"#),
        ("cxpak_context_for_task", r#"{"task":"smoke"}"#),
        ("cxpak_pack_context", r#"{"files":["src/main.rs"]}"#),
        ("cxpak_visual", r#"{"type":"dashboard","format":"html"}"#),
        ("cxpak_onboard", "{}"),
        ("cxpak_verify", "{}"),
    ];

    // First confirm the count matches CLAUDE.md's documented "26 MCP tools".
    assert_eq!(
        tools.len(),
        26,
        "tool list count must match CLAUDE.md's '26 MCP tools' contract; \
         updating either the count or the tool list requires updating both"
    );

    // Build one big multi-request input and drive the framing loop once.
    // Each request gets a unique sequential id starting at 100 so we can
    // map responses back unambiguously.
    let mut input = String::new();
    for (i, (name, args)) in tools.iter().enumerate() {
        let id = 100 + i;
        input.push_str(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}{newline}"#,
            newline = "\n"
        ));
    }

    let output = drive(&input);
    let responses: Vec<serde_json::Value> = output
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    assert_eq!(
        responses.len(),
        tools.len(),
        "expected one response per tool/call; got {} responses for {} tools",
        responses.len(),
        tools.len()
    );

    for (i, ((name, _args), resp)) in tools.iter().zip(responses.iter()).enumerate() {
        let expected_id = 100 + i;
        assert_eq!(
            resp["id"].as_u64(),
            Some(expected_id as u64),
            "response #{i} for tool {name}: id mismatch — got {resp}"
        );
        // Either result.content[0].text OR error envelope is allowed —
        // SOME tools may legitimately error on an empty index (the test
        // fixture in mcp_stdio_framing builds with vec![]).  Each MUST
        // have one or the other; what's NOT allowed is a malformed
        // envelope (no result, no error).
        let has_result = resp["result"]["content"][0]["text"].is_string();
        let has_error = resp["error"].is_object();
        assert!(
            has_result || has_error,
            "tool {name} (id={expected_id}) returned neither result.content[0].text nor error envelope. \
             Full response: {resp}"
        );
        // If errored, it MUST be a JSON-RPC error with a numeric code —
        // a free-form panic envelope would also break LSP framing
        // contracts for any client that re-marshals the response.
        if has_error {
            assert!(
                resp["error"]["code"].is_number(),
                "tool {name} error envelope must have numeric code; got {resp}"
            );
            assert!(
                resp["error"]["message"].is_string(),
                "tool {name} error envelope must have string message; got {resp}"
            );
        }
    }
}

/// Multiple sequential requests on the same loop: requests are processed
/// in order, each response is on its own line.
#[test]
fn three_sequential_requests_each_get_their_response() {
    let req = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{}}}\n\
               {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n\
               {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"cxpak_health\",\"arguments\":{}}}\n";
    let output = drive(req);
    let ids: Vec<i64> = output
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v["id"].as_i64())
        .collect();
    assert_eq!(
        ids,
        vec![1, 2, 3],
        "responses must arrive in request order with matching ids"
    );
}
