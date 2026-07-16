//! Real-subprocess `cxpak serve --mcp` freshness test (issue #18, ADR-0200).
//!
//! Promotes the ad hoc `mcp_driver.py` stale-repro (Phase 0, REPRO.md) to a
//! committed Rust regression guard: spawns the real `cxpak serve --mcp`
//! binary against a real git repo, drives the real newline-delimited
//! JSON-RPC stdio protocol, makes a real filesystem edit, and asserts a warm
//! session's `tools/call` reflects it through the real `notify` `FileWatcher`
//! -- the exact repro (`node src/a.rs`: `out_degree 0` before the edit,
//! `out_degree 1` after, same session, no restart).
//!
//! The in-process regression test
//! (`commands::serve::tests::run_mcp_warm_session_reflects_edit`) exercises
//! the same wiring deterministically and faster; this test additionally
//! covers the real CLI dispatch (`main.rs` -> `run_mcp`) and the real stdio
//! framing, which an in-process test can't reach. The FS-event debounce
//! itself is inherently platform-timed, so both the readiness poll below and
//! the process-exit poll are bounded (not fixed sleeps).
#![cfg(feature = "daemon")]

use assert_cmd::cargo_bin;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{self, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// See `commands::serve::tests::make_watcher_test_repo` (serve.rs) for why
/// `b.rs` imports `a.rs`'s `helper` fn specifically via
/// `use crate::a::helper;` -- a leaf-item import is what
/// `resolve_rust_import` (index/graph.rs) resolves to a graph edge; a bare
/// `use crate::a;` (whole-module) does not.
fn make_test_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "pub fn helper() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/b.rs"),
        "use crate::a::helper;\npub fn go() { helper(); }\n",
    )
    .unwrap();
    let repo = git2::Repository::init(dir.path()).expect("git2 init");
    let mut index = repo.index().expect("repo index");
    for rel in ["src/a.rs", "src/b.rs"] {
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

/// Background reader: forwards each newline-delimited JSON-RPC response onto
/// `tx` as it arrives, so the test can bound every wait with `recv_timeout`
/// instead of blocking indefinitely on a hung or slow server.
fn spawn_reader(stdout: process::ChildStdout) -> mpsc::Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if tx.send(v).is_err() {
                break; // test side dropped the receiver
            }
        }
    });
    rx
}

fn write_line(stdin: &mut process::ChildStdin, body: &Value) {
    stdin.write_all(body.to_string().as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

/// Bounded wait for process exit -- `Child::wait` has no built-in timeout, and
/// a hang here (a leaked/un-joined background thread) must fail the test
/// rather than the whole suite.
fn wait_bounded(child: &mut process::Child, timeout: Duration) -> Option<process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Real end-to-end: a warm `cxpak serve --mcp` session reflects a
/// post-startup edit -- issue #18's exact repro, driven through the real
/// binary and the real stdio protocol.
#[test]
fn mcp_subprocess_warm_session_reflects_edit() {
    let repo = make_test_repo();
    let path = repo.path().to_path_buf();

    let mut child = process::Command::new(cargo_bin!("cxpak"))
        .args(["serve", "--mcp", path.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cxpak binary should spawn for serve --mcp");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let rx = spawn_reader(stdout);

    write_line(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    );
    let init = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("initialize must answer promptly, off the index-build path");
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "cxpak");

    let call_node_a = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "cxpak_graph",
            "arguments": {"op": "graph", "graph_op": "node", "id": "src/a.rs"}
        }
    });

    // Poll (bounded) until the background build is ready -- before that, the
    // same tool call returns the "indexing in progress" retry text rather
    // than JSON.
    let mut node: Option<Value> = None;
    let build_deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < build_deadline {
        write_line(&mut stdin, &call_node_a);
        let resp = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("tools/call must answer");
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text");
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            node = Some(v);
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let before = node.expect("background build did not become ready within 30s");
    assert_eq!(before["exists"], true, "before edit: {before}");
    assert_eq!(before["out_degree"], 0, "before edit: {before}");

    // The build reaching `Ready` only proves the *build* thread is done;
    // `spawn_mcp_watcher` polls the same cell independently (every
    // `WATCHER_WAIT_POLL`) before it installs its own `FileWatcher`. Give it
    // a moment -- an edit that lands before the watcher exists produces no
    // event to miss, so the assertion below would hang until `edit_deadline`
    // for a reason unrelated to #18. This is the FS-event timing tradeoff
    // this test documents (see module doc comment).
    std::thread::sleep(Duration::from_millis(500));

    // Real edit -- the exact #18 repro: a.rs gains an out-edge.
    std::fs::write(path.join("src/c.rs"), "pub fn cee() {}\n").unwrap();
    std::fs::write(
        path.join("src/a.rs"),
        "use crate::c::cee;\npub fn helper() { cee(); }\n",
    )
    .unwrap();

    // Bounded poll -- same warm session, no restart -- for the watcher's
    // debounced republish to land.
    let mut after = before.clone();
    let edit_deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < edit_deadline {
        write_line(&mut stdin, &call_node_a);
        let resp = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("tools/call must answer");
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text");
        after = serde_json::from_str(text).expect("tool result JSON");
        if after["out_degree"] == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        after["out_degree"], 1,
        "warm MCP session must reflect the edit, not serve the stale \
         startup index (issue #18): {after}"
    );

    // Clean shutdown: closing stdin is the documented EOF path -- `run_mcp`
    // joins both background threads before the process exits.
    drop(stdin);
    let status = wait_bounded(&mut child, Duration::from_secs(10))
        .expect("cxpak serve --mcp must exit promptly on stdin EOF, not hang");
    assert!(
        status.success(),
        "cxpak serve --mcp must exit cleanly on stdin EOF"
    );
}
