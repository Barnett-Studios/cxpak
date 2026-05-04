//! Real-subprocess `cxpak lsp` test — exercises the tower-lsp
//! Content-Length framing layer end-to-end.
//!
//! Closes review-finding #5: every other LSP test calls
//! `methods::handle_custom_method` directly in-process, so a regression
//! in tower-lsp's framing (Content-Length header parsing, response
//! envelope shape, method-not-found JSON-RPC error code) would not be
//! caught.  The framing layer is third-party code we don't own, but our
//! contract with it (request/response shape, error code conventions)
//! is part of the public LSP surface — clients treat a malformed
//! envelope as a fatal protocol violation and disconnect.

#![cfg(feature = "lsp")]

use assert_cmd::cargo_bin;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{self, Stdio};
use std::time::Duration;

fn make_test_repo() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let sig = git2::Signature::now("Test", "t@t.com").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let mut idx = repo.index().unwrap();
    idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    idx.write().unwrap();
    let tree_id = idx.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    dir
}

/// Write a JSON-RPC request as `Content-Length: N\r\n\r\n<body>`.
fn write_lsp_message(stdin: &mut process::ChildStdin, body: &str) {
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .unwrap();
    stdin.write_all(body.as_bytes()).unwrap();
    stdin.flush().unwrap();
}

/// Read one Content-Length-framed JSON-RPC message from a reader.  Bounded
/// by the stream's read timeout (caller installs it on the underlying
/// File via `set_read_timeout`).
fn read_lsp_message<R: Read>(reader: &mut BufReader<R>) -> Option<Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header).ok()?;
        if n == 0 {
            return None; // EOF
        }
        let trimmed = header.trim_end_matches("\r\n");
        if trimmed.is_empty() {
            // End of headers
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Read messages until one arrives whose `id` matches `expected_id`.
/// Skips server-initiated notifications (no id) and unrelated responses.
/// Bounded by 20 messages — anything beyond that is a protocol regression.
fn read_response_for_id<R: Read>(reader: &mut BufReader<R>, expected_id: u64) -> Option<Value> {
    for _ in 0..20 {
        let msg = read_lsp_message(reader)?;
        if msg["id"].as_u64() == Some(expected_id) {
            return Some(msg);
        }
        // Otherwise it's a notification (no id) or an unrelated response —
        // keep reading.  tower-lsp typically emits window/logMessage and
        // similar between requests.
    }
    None
}

fn spawn_lsp(repo: &tempfile::TempDir) -> process::Child {
    process::Command::new(cargo_bin!("cxpak"))
        .args(["lsp", repo.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cxpak binary should spawn for lsp")
}

#[test]
fn lsp_initialize_handshake_returns_capabilities() {
    let repo = make_test_repo();
    let mut child = spawn_lsp(&repo);

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

    // Standard LSP initialize request.  rootUri pointing at the test repo.
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "rootUri": format!("file://{}", repo.path().to_str().unwrap()),
            "capabilities": {}
        }
    });
    write_lsp_message(stdin, &initialize.to_string());

    let resp = read_response_for_id(&mut reader, 1).expect("initialize response");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    let caps = &resp["result"]["capabilities"];
    assert!(
        caps.is_object(),
        "initialize response must include result.capabilities; got: {resp}"
    );

    // Cleanup.
    child.kill().ok();
    child.wait().ok();
}

#[test]
fn lsp_custom_health_method_returns_payload() {
    let repo = make_test_repo();
    let mut child = spawn_lsp(&repo);

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

    // initialize first
    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": format!("file://{}", repo.path().to_str().unwrap()),
                "capabilities": {}
            }
        })
        .to_string(),
    );
    let _init_resp = read_response_for_id(&mut reader, 1).expect("initialize response");

    // initialized notification (no response expected)
    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        })
        .to_string(),
    );

    // cxpak/health custom method.  tower-lsp rejects `params: {}` with
    // -32602 (Invalid params) when the method handler signature is
    // `fn() -> ...` (no arg) — it expects either `params: null` or the
    // field omitted entirely.  Send a minimal valid envelope.
    write_lsp_message(stdin, r#"{"jsonrpc":"2.0","id":2,"method":"cxpak/health"}"#);

    let resp = read_response_for_id(&mut reader, 2).expect("cxpak/health response");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert!(
        resp["error"].is_null(),
        "cxpak/health on a real index must succeed, not error; got: {resp}"
    );
    assert!(
        resp["result"].is_object(),
        "cxpak/health result must be an object; got: {resp}"
    );

    child.kill().ok();
    child.wait().ok();
}

#[test]
fn lsp_unknown_method_returns_method_not_found() {
    let repo = make_test_repo();
    let mut child = spawn_lsp(&repo);

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

    // initialize first so the server is ready to dispatch.
    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": format!("file://{}", repo.path().to_str().unwrap()),
                "capabilities": {}
            }
        })
        .to_string(),
    );
    let _ = read_response_for_id(&mut reader, 1);

    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "this/does/not/exist",
            "params": {}
        })
        .to_string(),
    );

    let resp = read_response_for_id(&mut reader, 99).expect("error response");
    assert_eq!(resp["id"], 99);
    // tower-lsp's spec-compliant code for unknown methods is -32601.
    assert_eq!(
        resp["error"]["code"].as_i64(),
        Some(-32601),
        "unknown method must return JSON-RPC -32601 (Method not found); got: {resp}"
    );

    child.kill().ok();
    child.wait().ok();
}

/// SIGTERM graceful shutdown for cxpak lsp — closes review-finding #3
/// at the runtime level (the source-level fix is in src/lsp/mod.rs).
/// Pre-fix: tower-lsp's `Server::serve` only completes when stdin
/// closes; SIGTERM from the OS would force-kill mid-request.
#[cfg(unix)]
#[test]
fn lsp_sigterm_triggers_graceful_shutdown() {
    let repo = make_test_repo();
    let mut child = spawn_lsp(&repo);

    // Drain stderr on a background thread into a shared String so the
    // main thread can poll for the "ready" banner without blocking on
    // `read()` (the std::process::ChildStderr pipe doesn't expose
    // O_NONBLOCK and a synchronous read could hang past the deadline).
    let stderr = child.stderr.take().expect("stderr pipe");
    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let stderr_buf_drain = std::sync::Arc::clone(&stderr_buf);
    let drain_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Ok(mut buf) = stderr_buf_drain.lock() {
                        buf.push_str(&line);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait until cxpak prints "cxpak lsp: ready" on stderr.  Eliminates
    // the prior fixed-duration sleep race (build_index + signal install
    // could exceed 500ms under parallel-test load on macOS).
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(buf) = stderr_buf.lock() {
            if buf.contains("cxpak lsp: ready") {
                break;
            }
        }
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            let buf = stderr_buf.lock().unwrap().clone();
            panic!(
                "cxpak lsp did not print `ready` banner within 15s — \
                 startup is hung or banner was removed.  Stderr so far: {buf}"
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let pid = child.id();
    let kill_status = process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("kill -TERM should be invocable");
    assert!(
        kill_status.success(),
        "kill -TERM exit status: {kill_status:?}"
    );

    // Wait up to 5s for graceful exit.  Stderr is being drained on the
    // background thread; check the shared buffer for the banner.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Let the stderr-drain thread finish reading any final
                // bytes the child wrote between print and exit.
                drain_handle.join().ok();
                let buf = stderr_buf.lock().unwrap().clone();
                assert!(
                    status.success(),
                    "SIGTERM must trigger clean exit (status 0). \
                     status={status:?} stderr=<<<{buf}>>>"
                );
                assert!(
                    buf.contains("shutting down gracefully"),
                    "stderr should show the graceful-shutdown banner; got: <<<{buf}>>>"
                );
                return;
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    child.kill().ok();
                    child.wait().ok();
                    let buf = stderr_buf.lock().unwrap().clone();
                    panic!(
                        "cxpak lsp did not exit within 5s of SIGTERM — \
                         graceful-shutdown handler is missing or hung. \
                         Stderr so far: {buf}"
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
}
