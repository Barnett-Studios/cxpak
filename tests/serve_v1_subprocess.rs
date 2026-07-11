//! Real-subprocess `cxpak serve` tests for the `/v1/*` API and process
//! lifecycle.  The existing `tests/serve_test.rs` covers the legacy
//! `/health`, `/stats`, `/overview`, `/trace` endpoints in subprocess mode and
//! exercises the bearer-auth helpers in-process via `tower::ServiceExt`, but
//! three subprocess gaps remained as of v2.1.0:
//!
//!   1. `/v1/*` routes had no real-binary HTTP coverage.  Auth, header
//!      handling, and route wiring were only validated via in-process Router
//!      tests, which bypass the network stack and the binary's startup path.
//!   2. Bearer-token authentication had no real-binary roundtrip — the
//!      timing-safe comparator was unit-tested but never exercised against a
//!      real `Authorization: Bearer …` header arriving over a TCP socket.
//!   3. SIGTERM graceful shutdown had no test at all.  Every other test
//!      tears the child down with `child.kill()` (SIGKILL on Unix), which
//!      bypasses the shutdown handler.  Containerised environments
//!      (kubectl, systemd, docker stop) and most CI process killers send
//!      SIGTERM, not SIGINT — without a SIGTERM handler the process is
//!      force-killed by the kernel after the grace period and any in-flight
//!      request is dropped mid-write.
//!
//! The SIGTERM test is `#[cfg(unix)]`; Windows does not have SIGTERM and the
//! `serve.rs` shutdown path falls back to Ctrl-C only on that platform.

#![cfg(feature = "daemon")]

mod v1_subprocess_tests {
    use assert_cmd::cargo_bin;
    use serde_json::Value;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::process::{self, Stdio};
    use std::time::{Duration, Instant};

    fn make_test_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() { println!(\"hi\"); }\nfn helper() -> i32 { 42 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
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

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    /// Block until the server can actually *serve* a request, not merely
    /// until the port is connectable.
    ///
    /// `cxpak serve` builds its index BEFORE binding the port
    /// (`build_index` at commands/serve.rs:1086, `TcpListener::bind` at
    /// :1138), so TCP-connectability already implies the index is built.
    /// But the listener is bound *before* `axum::serve(...)` starts its
    /// accept loop and installs the SIGTERM handler via
    /// `.with_graceful_shutdown(...)` (:1166).  In that window the port
    /// accepts the connection at the kernel level but no task pulls the
    /// request off the socket, so a raw `read_to_end` returns 0 bytes /
    /// times out, and a SIGTERM hits the kernel default disposition
    /// (force-kill) instead of the graceful handler.
    ///
    /// Polling `GET /health` until it returns a parseable HTTP 200 proves
    /// the accept loop is live and `axum::serve` is executing — which means
    /// the signal handler is installed and the index is queryable.  `/health`
    /// is unauthenticated (it sits outside the `/v1/*` auth layer) and
    /// state-free, so it is the correct readiness probe regardless of
    /// whether the server was launched with `--token`.
    ///
    /// Mirrors the readiness-poll fix already applied to the LSP subprocess
    /// test (tests/lsp_subprocess.rs ~line 290), which replaced a fixed
    /// sleep that lost the same build_index + signal-install race under
    /// parallel-test load.
    fn wait_for_server(port: u16) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some((200, _)) = try_http_request(port, "GET", "/health", None, None) {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// One raw HTTP/1.1 request attempt over a fresh TCP connection.
    ///
    /// Returns `Some((status, body))` only when a complete, well-formed
    /// response was read (status line parsed and, when the server declares
    /// `Content-Length`, the full body received).  Returns `None` on any
    /// transient failure — connect refused/reset, write error, read timeout,
    /// a 0-byte / short read, or an unparseable status line.  Distinguishing
    /// "transient nothing" from "real response" is what lets both the
    /// readiness poll and the public `http_request` retry deterministically
    /// instead of panicking on a partial read.
    ///
    /// The request always sends `Connection: close`, so the server closes
    /// the socket after the response and a read-to-EOF is well-defined even
    /// absent a `Content-Length`.
    fn try_http_request(
        port: u16,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        body_json: Option<&str>,
    ) -> Option<(u16, String)> {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).ok()?;
        // 30s read timeout: heavy /v1/* handlers (briefing, auto_context-
        // backed, security_surface) routinely take 1-3s on cold caches,
        // and Linux CI runners are slower than local macOS.  Generous
        // enough to never bite a real success path while still bounding a
        // genuinely-stuck server.
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .ok()?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .ok()?;

        let mut req = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Connection: close\r\n"
        );
        if let Some(token) = bearer {
            req.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        if let Some(body) = body_json {
            req.push_str("Content-Type: application/json\r\n");
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
            req.push_str("\r\n");
            req.push_str(body);
        } else {
            req.push_str("\r\n");
        }
        stream.write_all(req.as_bytes()).ok()?;
        stream.flush().ok()?;

        // Read until the header terminator (CRLFCRLF) is seen, then — if the
        // server declared a Content-Length — read exactly that many body
        // bytes; otherwise read to EOF (the server half-closes after the
        // response because we sent `Connection: close`).
        //
        // Important: do NOT shutdown(Write) before reading.  Half-closing
        // makes hyper on the server side see FIN mid-parse and abort the
        // request without responding.
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            // Stop once we have full headers AND (Content-Length satisfied
            // OR the peer closed).  Computing this each iteration keeps the
            // read deterministic without over-reading.
            if let Some(header_end) = find_header_end(&buf) {
                if let Some(len) = content_length(&buf[..header_end]) {
                    if buf.len() >= header_end + len {
                        break;
                    }
                }
            }
            match (&stream).read(&mut chunk) {
                Ok(0) => break, // EOF — server closed (Connection: close)
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break, // timeout / reset — fall through to parse
            }
        }

        // A complete response must at minimum contain the header terminator.
        // Anything short of that is a transient partial read → None so the
        // caller retries rather than panicking on a half-line.
        let header_end = find_header_end(&buf)?;
        let response = String::from_utf8_lossy(&buf).into_owned();
        let status = response
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())?;
        // If Content-Length was declared, require the full body before
        // accepting the response as complete.
        if let Some(len) = content_length(&buf[..header_end]) {
            if buf.len() < header_end + len {
                return None;
            }
        }
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        Some((status, body))
    }

    /// Index of the first byte after the `\r\n\r\n` header terminator, or
    /// `None` if the headers are not yet complete.
    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
    }

    /// Parse the `Content-Length` header value from a raw header block
    /// (case-insensitive header name).  `None` when absent or unparseable.
    fn content_length(headers: &[u8]) -> Option<usize> {
        let text = String::from_utf8_lossy(headers);
        for line in text.lines() {
            if let Some((name, value)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("content-length") {
                    return value.trim().parse::<usize>().ok();
                }
            }
        }
        None
    }

    /// Send a raw HTTP/1.1 request and return `(status_code, body)`,
    /// retrying once on a transient empty/short read before panicking.
    /// Optionally attaches an `Authorization: Bearer <token>` header and a
    /// JSON body.
    fn http_request(
        port: u16,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        body_json: Option<&str>,
    ) -> (u16, String) {
        // Up to 3 attempts: under parallel-test load the accept queue can be
        // momentarily drained or a connection reset between requests, even
        // after wait_for_server proved the server serves.  A transient empty
        // read returns None from try_http_request; one or two quick retries
        // cover that window without masking a genuinely-down server.
        for attempt in 0..3 {
            if let Some(result) = try_http_request(port, method, path, bearer, body_json) {
                return result;
            }
            if attempt < 2 {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        panic!(
            "no complete HTTP response from 127.0.0.1:{port} after 3 attempts; \
             method={method} path={path} — server is unreachable, hung, or \
             returned a truncated response"
        );
    }

    fn spawn_serve(repo: &tempfile::TempDir, port: u16, token: Option<&str>) -> process::Child {
        let mut cmd = process::Command::new(cargo_bin!("cxpak"));
        cmd.args([
            "serve",
            "--port",
            &port.to_string(),
            "--tokens",
            "50k",
            repo.path().to_str().unwrap(),
        ]);
        if let Some(t) = token {
            cmd.args(["--token", t]);
        }
        cmd.stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("cxpak binary should spawn")
    }

    // ─── /v1/health: auth gating ────────────────────────────────────────────

    #[test]
    fn v1_health_no_token_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, None);
        assert!(wait_for_server(port), "server did not start within 5s");

        let (status, body) = http_request(port, "GET", "/v1/health", None, None);
        assert_eq!(
            status, 200,
            "GET /v1/health without --token must be reachable from loopback"
        );
        let json: Value =
            serde_json::from_str(&body).expect("body should be JSON when status is 200");
        // /v1/health returns the cached index-health snapshot, not a
        // {"status":"ok"} ping (that's /health).  Verify the snapshot
        // structure is intact.
        assert!(
            json["composite"].is_number(),
            "/v1/health must return composite score; got: {body}"
        );
        assert!(
            json["dimensions"].is_object(),
            "/v1/health must return per-dimension scores; got: {body}"
        );
        assert!(
            json["total_files"].as_u64().is_some(),
            "/v1/health must return total_files; got: {body}"
        );

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn v1_health_with_token_no_bearer_returns_401() {
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, Some("supersecret-bearer-token"));
        assert!(wait_for_server(port));

        let (status, _) = http_request(port, "GET", "/v1/health", None, None);
        assert_eq!(
            status, 401,
            "/v1/* with --token but no Authorization header must be 401"
        );

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn v1_health_with_token_correct_bearer_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "supersecret-bearer-token";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        let (status, body) = http_request(port, "GET", "/v1/health", Some(token), None);
        assert_eq!(status, 200, "correct bearer token must be accepted");
        let json: Value = serde_json::from_str(&body).unwrap();
        assert!(
            json["composite"].is_number() && json["dimensions"].is_object(),
            "/v1/health body should carry the index health snapshot; got: {body}"
        );

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn v1_health_with_token_wrong_bearer_returns_401() {
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, Some("the-real-secret"));
        assert!(wait_for_server(port));

        let (status, _) = http_request(port, "GET", "/v1/health", Some("a-different-token"), None);
        assert_eq!(status, 401, "wrong bearer token must be rejected");

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn v1_health_with_token_same_length_wrong_bearer_returns_401() {
        // Same byte length as the configured token. Verifies the constant-time
        // comparator's wrong-byte path (different length is short-circuited).
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, Some("aaaaaaaaaaaaaaaa"));
        assert!(wait_for_server(port));

        let (status, _) = http_request(port, "GET", "/v1/health", Some("bbbbbbbbbbbbbbbb"), None);
        assert_eq!(
            status, 401,
            "same-length wrong bearer must still be rejected"
        );

        child.kill().ok();
        child.wait().ok();
    }

    // ─── /v1/conventions (auth-gated POST) ──────────────────────────────────

    #[test]
    fn v1_conventions_with_correct_bearer_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "topsecret-conventions";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        let (status, body) = http_request(port, "POST", "/v1/conventions", Some(token), Some("{}"));
        assert_eq!(
            status, 200,
            "POST /v1/conventions with correct bearer should be 200; body was: {body}"
        );
        // The body is a ConventionProfile JSON object — only check that
        // it parses and is an object.  Schema details belong to the in-process
        // tests; this test owns the binary + network roundtrip.
        let json: Value = serde_json::from_str(&body)
            .expect("body should parse as JSON when /v1/conventions returns 200");
        assert!(
            json.is_object(),
            "/v1/conventions response should be a JSON object"
        );

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn v1_conventions_without_bearer_returns_401() {
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, Some("topsecret-conventions"));
        assert!(wait_for_server(port));

        let (status, _) = http_request(port, "POST", "/v1/conventions", None, Some("{}"));
        assert_eq!(
            status, 401,
            "POST /v1/* without bearer when --token is set must be 401"
        );

        child.kill().ok();
        child.wait().ok();
    }

    // ─── Auth gate covers EVERY /v1/* route (not just /v1/health & /v1/conventions) ──

    /// Coverage closure: prior to this test, only /v1/health and
    /// /v1/conventions had subprocess coverage.  The other 10 v1 routes
    /// relied on in-process Router tests, which is the same coverage shape
    /// that hid the original SIGTERM-only-on-ctrl-c gap.  Parameterising
    /// over every POST route asserts the auth_layer (route_layer at
    /// commands/serve.rs:421) wraps every handler uniformly — a regression
    /// that excluded one route from the layer would otherwise be silent.
    #[test]
    fn every_v1_post_route_rejects_missing_bearer() {
        let routes = [
            "/v1/risks",
            "/v1/architecture",
            "/v1/predict",
            "/v1/drift",
            "/v1/security_surface",
            "/v1/dead_code",
            "/v1/call_graph",
            "/v1/data_flow",
            "/v1/cross_lang",
            "/v1/briefing",
        ];
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, Some("auth-gate-secret"));
        assert!(wait_for_server(port));

        for route in routes.iter() {
            let (status, _) = http_request(port, "POST", route, None, Some("{}"));
            assert_eq!(
                status, 401,
                "POST {route} must return 401 when --token is set and no bearer is sent"
            );
        }

        child.kill().ok();
        child.wait().ok();
    }

    /// Same matrix, with WRONG bearer.  Distinct from the missing-bearer
    /// case because the auth path takes a different branch (extract_bearer
    /// returns Some, then check_auth's constant-time compare returns false)
    /// — both branches must end at 401.
    #[test]
    fn every_v1_post_route_rejects_wrong_bearer() {
        let routes = [
            "/v1/risks",
            "/v1/architecture",
            "/v1/predict",
            "/v1/drift",
            "/v1/security_surface",
            "/v1/dead_code",
            "/v1/call_graph",
            "/v1/data_flow",
            "/v1/cross_lang",
            "/v1/briefing",
        ];
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, Some("the-real-secret"));
        assert!(wait_for_server(port));

        for route in routes.iter() {
            let (status, _) =
                http_request(port, "POST", route, Some("a-different-token"), Some("{}"));
            assert_eq!(
                status, 401,
                "POST {route} with wrong bearer must return 401"
            );
        }

        child.kill().ok();
        child.wait().ok();
    }

    /// Positive path for every route that doesn't require specific body
    /// fields.  Routes with required typed bodies (predict/files,
    /// call_graph/target, data_flow/symbol) are covered by their own tests
    /// below with valid payloads.  This proves the route + handler are
    /// wired and the auth_layer doesn't accidentally block the success
    /// path — pre-this-test, only /v1/health and /v1/conventions had
    /// subprocess proof of "auth passes → handler runs".
    #[test]
    fn every_v1_post_route_returns_2xx_with_correct_bearer() {
        // Routes that accept POST with empty `{}` body and produce a
        // structured response (do not require a typed payload).  Routes
        // requiring specific fields — predict (files), call_graph (target),
        // data_flow (symbol), briefing (task) — are tested separately
        // below with valid payloads.
        let routes = [
            "/v1/risks",
            "/v1/architecture",
            "/v1/drift",
            "/v1/security_surface",
            "/v1/dead_code",
            "/v1/cross_lang",
        ];
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "live-token-1234";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        for route in routes.iter() {
            let (status, body) = http_request(port, "POST", route, Some(token), Some("{}"));
            assert!(
                (200..300).contains(&status),
                "POST {route} with correct bearer should be 2xx; got {status}, body: {body}"
            );
            // Body must be parseable JSON — proves the handler ran to
            // completion and the response went through the JSON serialisers.
            let parsed: Result<Value, _> = serde_json::from_str(&body);
            assert!(
                parsed.is_ok(),
                "POST {route} response body must be JSON; got: {body}"
            );
        }

        child.kill().ok();
        child.wait().ok();
    }

    /// /v1/predict requires a `files` array.  Test it with a real file from
    /// the index.
    #[test]
    fn v1_predict_with_correct_bearer_and_valid_body_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "predict-token";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        let body = r#"{"files":["src/main.rs"],"depth":2}"#;
        let (status, response_body) =
            http_request(port, "POST", "/v1/predict", Some(token), Some(body));
        assert!(
            (200..300).contains(&status),
            "/v1/predict with files array should be 2xx; got {status}, body: {response_body}"
        );
        let parsed: Value =
            serde_json::from_str(&response_body).expect("/v1/predict body must be JSON");
        assert!(
            parsed.is_object(),
            "/v1/predict response should be a JSON object"
        );

        child.kill().ok();
        child.wait().ok();
    }

    /// /v1/call_graph accepts an optional `target`; with no target it should
    /// still return a structured response (not error).
    #[test]
    fn v1_call_graph_with_correct_bearer_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "callgraph-token";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        let body = r#"{"target":"main"}"#;
        let (status, response_body) =
            http_request(port, "POST", "/v1/call_graph", Some(token), Some(body));
        assert!(
            (200..300).contains(&status),
            "/v1/call_graph should be 2xx; got {status}, body: {response_body}"
        );

        child.kill().ok();
        child.wait().ok();
    }

    /// /v1/briefing requires a `task` field.
    #[test]
    fn v1_briefing_with_correct_bearer_and_task_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "briefing-token";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        let body = r#"{"task":"investigate why main returns early"}"#;
        let (status, response_body) =
            http_request(port, "POST", "/v1/briefing", Some(token), Some(body));
        assert!(
            (200..300).contains(&status),
            "/v1/briefing with task should be 2xx; got {status}, body: {response_body}"
        );

        child.kill().ok();
        child.wait().ok();
    }

    /// /v1/data_flow needs a `symbol`.
    #[test]
    fn v1_data_flow_with_correct_bearer_returns_200() {
        let repo = make_test_repo();
        let port = find_free_port();
        let token = "dataflow-token";
        let mut child = spawn_serve(&repo, port, Some(token));
        assert!(wait_for_server(port));

        let body = r#"{"symbol":"main","depth":3}"#;
        let (status, response_body) =
            http_request(port, "POST", "/v1/data_flow", Some(token), Some(body));
        assert!(
            (200..300).contains(&status),
            "/v1/data_flow should be 2xx; got {status}, body: {response_body}"
        );

        child.kill().ok();
        child.wait().ok();
    }

    // ─── SIGTERM graceful shutdown (Unix only) ──────────────────────────────

    /// Verifies that `cxpak serve` exits cleanly when sent SIGTERM, draining
    /// in-flight work via `axum::serve(...).with_graceful_shutdown(...)`.
    /// Pre-fix the shutdown handler only awaited `tokio::signal::ctrl_c()`
    /// (SIGINT), so SIGTERM force-killed the process via the kernel default
    /// disposition — no graceful drain.
    #[cfg(unix)]
    #[test]
    fn sigterm_triggers_graceful_shutdown() {
        let repo = make_test_repo();
        let port = find_free_port();
        let mut child = spawn_serve(&repo, port, None);
        assert!(wait_for_server(port), "server did not start within 5s");

        let pid = child.id();
        // `kill -TERM` rather than a libc dep: keeps dev-dependencies
        // unchanged.  The cxpak binary is its own process (Command::spawn
        // does not put it in the parent's session/group), so a pid-targeted
        // kill reaches exactly the cxpak process.
        let kill_status = process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("kill -TERM should be invocable");
        assert!(
            kill_status.success(),
            "kill -TERM exit status: {kill_status:?}"
        );

        // Wait up to 5s for graceful exit.  `try_wait` reaps and caches the
        // status; if SIGTERM had no handler the kernel default would terminate
        // the process with ExitStatus::signal() == Some(15) which `success()`
        // reports as false.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Drain stderr now that the process has exited.  The
                    // graceful-shutdown branch prints "shutting down
                    // gracefully..." after the signal future resolves.
                    let mut stderr_buf = String::new();
                    if let Some(mut stderr) = child.stderr.take() {
                        let _ = stderr.read_to_string(&mut stderr_buf);
                    }
                    assert!(
                        status.success(),
                        "SIGTERM must trigger clean exit (status 0). \
                         status={status:?} stderr=<<<{stderr_buf}>>>"
                    );
                    assert!(
                        stderr_buf.contains("shutting down gracefully"),
                        "stderr should show the graceful-shutdown banner; got: <<<{stderr_buf}>>>"
                    );
                    return;
                }
                Ok(None) => {
                    if Instant::now() > deadline {
                        child.kill().ok();
                        child.wait().ok();
                        panic!(
                            "process did not exit within 5s of SIGTERM — \
                             graceful-shutdown handler is missing or hung"
                        );
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("try_wait failed: {e}"),
            }
        }
    }
}
