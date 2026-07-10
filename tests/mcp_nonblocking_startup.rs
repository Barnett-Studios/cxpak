//! Task R0 — non-blocking MCP startup (lazy index + cache off the handshake path).
//!
//! Proves the concurrency contract end-to-end against the real serve.rs stdio
//! loop and the real background-build thread:
//!
//! 1. `spawn_mcp_index_build` runs `build_index` on a background thread and
//!    flips the shared readiness cell `Building` -> `Ready` without blocking.
//! 2. The `initialize` handshake answers well-formed and fast (< 2s) *while the
//!    background build is still in flight* — indexing is off the handshake path.
//! 3. A `tools/call` issued before the index is ready returns a graceful
//!    JSON-RPC `result` (retry status), never a session-killing protocol error.
//! 4. Once ready, the same `tools/call` returns normal results.
#![cfg(feature = "daemon")]

use cxpak::commands::serve::{
    mcp_stdio_loop_readiness, spawn_mcp_index_build, IndexReadiness, SharedReadiness,
    SharedSnapshot,
};
use serde_json::Value;
use std::io::Cursor;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// A committed git repo with a handful of Rust files — enough for `build_index`
/// (which walks git-tracked files) to produce a non-empty index.
fn make_git_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "mod lib;\nfn main() { lib::hello(); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn hello() -> u32 { 42 }\n",
    )
    .unwrap();
    let repo = git2::Repository::init(dir.path()).expect("git2 init");
    let mut index = repo.index().expect("repo index");
    for rel in ["src/main.rs", "src/lib.rs"] {
        index.add_path(std::path::Path::new(rel)).expect("git add");
    }
    index.write().expect("index write");
    let tree_oid = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_oid).expect("find tree");
    let sig = git2::Signature::now("t", "t@t").expect("sig");
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .expect("initial commit");
    dir
}

fn drive(readiness: &SharedReadiness, path: &std::path::Path, input: &str) -> Vec<Value> {
    let snapshot: SharedSnapshot = Arc::new(RwLock::new(None));
    let mut out: Vec<u8> = Vec::new();
    mcp_stdio_loop_readiness(
        path,
        readiness,
        &snapshot,
        Cursor::new(input.as_bytes()),
        &mut out,
    )
    .expect("mcp loop");
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// End-to-end: background build flips readiness, handshake stays fast, and the
/// before/after tool-call behavior matches the R0 contract.
#[test]
fn background_build_publishes_ready_and_handshake_never_blocks() {
    let repo = make_git_repo();
    let path = repo.path().to_path_buf();

    // Start in `Building`, then kick the real background build.
    let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
    let handle = spawn_mcp_index_build(&path, Arc::clone(&readiness));

    // The `initialize` handshake must answer immediately and well-formed even
    // though the background build may still be running. Measured against a tight
    // bound: indexing is off the handshake path, so this cannot block on it.
    let start = Instant::now();
    let init = drive(
        &readiness,
        &path,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "initialize must return < 2s while indexing (took {elapsed:?})"
    );
    assert_eq!(init[0]["id"], 1);
    assert_eq!(init[0]["result"]["serverInfo"]["name"], "cxpak");
    assert!(init[0]["result"]["capabilities"]["tools"].is_object());

    // Wait (bounded) for the background build to publish `Ready`.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = matches!(&*readiness.read().unwrap(), IndexReadiness::Ready(_));
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background build did not become ready within 30s"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().expect("build thread joins cleanly");

    // After ready, a tool call returns real results (stats reflects 2 files).
    let call = drive(
        &readiness,
        &path,
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"cxpak_context\",\"arguments\":{\"op\":\"stats\"}}}\n",
    );
    assert_eq!(call[0]["id"], 2);
    assert!(call[0]["error"].is_null(), "ready tool call must not error");
    let text = call[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result text");
    let stats: Value = serde_json::from_str(text).expect("stats JSON");
    assert_eq!(stats["files"], 2, "stats must reflect the 2 indexed files");
}

/// Deterministic: a tool call before readiness returns a graceful `result`, and
/// `initialize`/`tools/list` answer instantly regardless of index state — no
/// dependence on wall-clock timing or a real build.
#[test]
fn before_ready_tool_call_returns_graceful_status() {
    let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Building));
    let responses = drive(
        &readiness,
        std::path::Path::new("/tmp"),
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"cxpak_context\",\"arguments\":{\"op\":\"stats\"}}}\n",
        ),
    );
    assert_eq!(responses.len(), 3);
    // initialize + tools/list answer while Building.
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "cxpak");
    assert!(responses[1]["result"]["tools"].is_array());
    // tools/call before ready: a `result` (not `error`) carrying a retry status.
    assert!(
        responses[2]["error"].is_null(),
        "before-ready tool call must not be a protocol error"
    );
    let text = responses[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("indexing in progress"),
        "expected a retry status, got: {text}"
    );
}

/// A failed background build surfaces a clear status on tool calls rather than
/// crashing the server: the loop still runs and returns a `result`.
#[test]
fn failed_build_surfaces_status_without_crashing() {
    let readiness: SharedReadiness = Arc::new(RwLock::new(IndexReadiness::Failed(
        "scanner exploded".into(),
    )));
    let responses = drive(
        &readiness,
        std::path::Path::new("/tmp"),
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"cxpak_context\",\"arguments\":{\"op\":\"stats\"}}}\n",
    );
    assert!(responses[0]["error"].is_null());
    let text = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("indexing failed"), "got: {text}");
    assert!(text.contains("scanner exploded"), "got: {text}");
}
