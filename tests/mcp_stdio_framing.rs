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
