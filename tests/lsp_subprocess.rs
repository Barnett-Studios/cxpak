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

    // cxpak/health custom method.  Every custom method on this surface now
    // takes its `params` argument (the 3 no-input methods — health,
    // conventions, deadCode — included, for uniformity), so `params: {}` is
    // the single client-safe call shape that works across all 16 methods.
    write_lsp_message(
        stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"cxpak/health","params":{}}"#,
    );

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

/// Regression: the three no-input custom methods (health, conventions,
/// deadCode) must accept `params: {}` like every other custom method, rather
/// than rejecting it with -32602 (which they did when registered without a
/// `params` argument). `{}` is the uniform client-safe call shape across the
/// whole custom surface; a client that sends it to one method and gets a
/// result must get a result from all of them.
#[test]
fn lsp_no_input_custom_methods_accept_empty_params() {
    let repo = make_test_repo();
    let mut child = spawn_lsp(&repo);

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

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
    let _ = read_response_for_id(&mut reader, 1).expect("initialize response");
    write_lsp_message(
        stdin,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    );

    for (id, method) in [
        (10u64, "cxpak/health"),
        (11, "cxpak/conventions"),
        (12, "cxpak/deadCode"),
    ] {
        let req = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{{}}}}"#);
        write_lsp_message(stdin, &req);
        let resp = read_response_for_id(&mut reader, id).expect("response for no-input method");
        assert!(
            resp["error"].is_null(),
            "{method} with params:{{}} must not error (regression: -32602); got: {resp}"
        );
        assert!(
            resp["result"].is_object(),
            "{method} with params:{{}} must return an object result; got: {resp}"
        );
    }

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

/// Regression: SIGTERM arriving while an LSP custom method is in-flight
/// must NOT drop the response.  Pre-fix the LSP shutdown path called
/// `std::process::exit(0)` immediately when the signal future resolved;
/// this aborted tower-lsp's dispatch loop AND any in-flight handler
/// task, so a request whose bytes were in the pipe but whose handler
/// hadn't finished writing the response simply never reached the
/// client (`EOF` on the response read).  That defeated the original
/// motivation for handling SIGTERM at all — "don't drop in-flight
/// responses" — which is the contract `commands/serve.rs` provides via
/// `axum::with_graceful_shutdown`.
///
/// Empirical race-test (matrix in audit notes): with the bug,
/// SIGTERM at delays 0–100ms after sending a ~120ms request all
/// produced EOF.  After the fix (spawn serve as a separate task +
/// configurable grace before exit), 0–300ms all return the full
/// response.
///
/// This test sends a real custom method known to take >0ms
/// (`cxpak/dataFlow` on a small fixture; ~tens of ms on a 4-file
/// repo), SIGTERMs the process immediately afterward, and asserts
/// the response arrives with `id` matching the request.
#[cfg(unix)]
#[test]
fn lsp_sigterm_drains_in_flight_response() {
    let repo = make_test_repo();
    // Add a couple more files so cxpak/dataFlow has something to walk;
    // the default fixture's single main.rs makes the handler near-
    // instant and we'd race past the in-flight window before SIGTERM
    // could find anything to drop.
    std::fs::write(
        repo.path().join("src/util.rs"),
        "pub fn helper() -> i32 { crate::main_helper() }\npub fn main_helper() -> i32 { 7 }\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub mod util;\npub fn entry() { util::helper(); }\n",
    )
    .unwrap();

    let mut child = spawn_lsp(&repo);
    let stderr = child.stderr.take().expect("stderr pipe");
    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let stderr_buf_drain = std::sync::Arc::clone(&stderr_buf);
    let _drain_handle = std::thread::spawn(move || {
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

    // Wait for ready banner.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !stderr_buf.lock().unwrap().contains("cxpak lsp: ready") {
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            panic!(
                "cxpak lsp never printed `ready`. stderr so far: {}",
                stderr_buf.lock().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);

    // Initialize.
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
    let _init = read_response_for_id(&mut reader, 1).expect("initialize response");

    // Send the heavy method, then SIGTERM IMMEDIATELY.  The fix's
    // contract is: handlers complete + write their responses during
    // the grace window before process::exit fires.
    write_lsp_message(
        stdin,
        r#"{"jsonrpc":"2.0","id":42,"method":"cxpak/dataFlow","params":{"symbol":"main"}}"#,
    );
    let pid = child.id();
    process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("kill -TERM should be invocable");

    // Read with a deadline well past the default 1500ms grace.
    let resp = read_response_for_id(&mut reader, 42)
        .expect("in-flight response must arrive — pre-fix this returned EOF");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 42);
    assert!(
        resp["result"].is_object() || resp["result"].is_array(),
        "in-flight cxpak/dataFlow must complete with a real result, not be cut to EOF; got: {resp}"
    );

    // Process should still exit cleanly after the grace window.
    let exit_status = child.wait().expect("child to exit");
    assert!(
        exit_status.success(),
        "exit must be clean (status 0) after grace; got: {exit_status:?}"
    );
}

// ---------------------------------------------------------------------------
// Workspace-root anchoring (#75, #76)
//
// Both defects follow from one absence: the server never establishes an
// ABSOLUTE workspace root. `cxpak lsp` takes `[PATH] [default: .]`, and a
// relative root makes `Url::from_file_path` fail (#75: every symbol gets the
// `file:///unknown` placeholder) and `strip_prefix` fail (#76: every URI
// resolution falls through to a suffix match with no root bound).
//
// These drive the real server over stdio, because both are only reachable
// through the invocation — a unit test that hands the functions an absolute
// root cannot see either.
// ---------------------------------------------------------------------------

/// Send `initialize` + `initialized`, returning nothing. Every test below needs
/// the handshake before the surface answers.
fn handshake(
    stdin: &mut process::ChildStdin,
    reader: &mut BufReader<process::ChildStdout>,
    root: &std::path::Path,
) {
    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": format!("file://{}", root.to_str().unwrap()),
                "capabilities": {}
            }
        })
        .to_string(),
    );
    read_response_for_id(reader, 1).expect("initialize response");
    write_lsp_message(
        stdin,
        &serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}).to_string(),
    );
}

/// #75 — at the DOCUMENTED DEFAULT path, every workspace symbol must carry a URI
/// that names a real file.
///
/// `location.uri` is the only field a client uses to open a symbol, so
/// `file:///unknown` is not a degraded answer: it is a well-formed lie that
/// makes "Go to Symbol in Workspace" list the right names and open none of
/// them. Spawned with NO path argument and `current_dir` set to the repo,
/// which is both the documented default and what an editor produces.
#[test]
fn workspace_symbols_at_the_default_path_carry_openable_uris() {
    let repo = make_test_repo();
    let mut child = process::Command::new(cargo_bin!("cxpak"))
        .arg("lsp") // no PATH argument -> `.`
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cxpak binary should spawn for lsp");

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);
    handshake(stdin, &mut reader, repo.path());

    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "workspace/symbol",
            "params": {"query": "main"}
        })
        .to_string(),
    );
    let resp = read_response_for_id(&mut reader, 2).expect("workspace/symbol response");
    let syms = resp["result"].as_array().cloned().unwrap_or_default();

    // Positive control. Without this the assertion below passes vacuously on a
    // server that returned nothing at all, which is the failure this whole
    // file exists to distinguish from a correct empty answer.
    assert!(
        !syms.is_empty(),
        "the default-path server found no symbol matching `main` — the search itself is broken, \
         so the URI assertion below would prove nothing: {resp}"
    );

    for s in &syms {
        let uri = s["location"]["uri"].as_str().unwrap_or_default();
        assert_ne!(
            uri, "file:///unknown",
            "symbol {:?} carries the placeholder URI: {s}",
            s["name"]
        );
        let path = uri.strip_prefix("file://").unwrap_or_default();
        assert!(
            !path.is_empty() && std::path::Path::new(path).exists(),
            "symbol {:?} has uri {uri:?}, which names no file on disk",
            s["name"]
        );
    }

    child.kill().ok();
    child.wait().ok();
}

/// #76 — a file OUTSIDE the workspace root is not this server's file, even when
/// its path ends in the same segments as an indexed one.
///
/// The resolver's separator bound aligns a match to *a* directory boundary, not
/// to *this workspace's* root, so `/other/src/main.rs` matches an index whose
/// only file is `src/main.rs`. The response is well-formed and the subject is
/// wrong — a lens with another project's token count, and (in the reported
/// case) a dead-code warning on a symbol called two lines below.
#[test]
fn a_file_outside_the_workspace_root_gets_no_answer() {
    let repo = make_test_repo();
    let other = make_test_repo(); // same shape: <other>/src/main.rs
    let mut child = spawn_lsp(&repo);

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);
    handshake(stdin, &mut reader, repo.path());

    let lens_for = |id: u64, p: std::path::PathBuf| {
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/codeLens",
            "params": {"textDocument": {"uri": format!("file://{}", p.to_str().unwrap())}}
        })
        .to_string()
    };

    // The control FIRST, so a server that answers nothing at all cannot pass
    // the real assertion by accident.
    write_lsp_message(stdin, &lens_for(2, repo.path().join("src/main.rs")));
    let inside = read_response_for_id(&mut reader, 2).expect("codeLens response (in-root)");
    assert!(
        !inside["result"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .is_empty(),
        "the server gave no lens for its OWN indexed file, so the out-of-root assertion below \
         would pass vacuously: {inside}"
    );

    write_lsp_message(stdin, &lens_for(3, other.path().join("src/main.rs")));
    let outside = read_response_for_id(&mut reader, 3).expect("codeLens response (out-of-root)");
    assert!(
        outside["result"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .is_empty(),
        "a server rooted at {:?} answered for {:?}, a file in a different project it never \
         indexed: {outside}",
        repo.path(),
        other.path().join("src/main.rs")
    );

    child.kill().ok();
    child.wait().ok();
}

/// #76's second half — a path INSIDE the root that names no indexed file must
/// also get nothing. `<repo>/sub/main.rs` does not exist, but it ends in
/// `main.rs`, and the suffix match does not care.
#[test]
fn a_nonexistent_path_inside_the_root_gets_no_answer() {
    let repo = make_test_repo();
    let mut child = spawn_lsp(&repo);

    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);
    handshake(stdin, &mut reader, repo.path());

    write_lsp_message(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/codeLens",
            "params": {"textDocument": {
                "uri": format!("file://{}", repo.path().join("sub/src/main.rs").to_str().unwrap())
            }}
        })
        .to_string(),
    );
    let resp = read_response_for_id(&mut reader, 2).expect("codeLens response");
    assert!(
        resp["result"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .is_empty(),
        "the server answered for sub/src/main.rs, which does not exist: {resp}"
    );

    child.kill().ok();
    child.wait().ok();
}
