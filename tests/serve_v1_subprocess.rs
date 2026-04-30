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

    fn wait_for_server(port: u16) -> bool {
        for _ in 0..50 {
            if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Send a raw HTTP/1.1 request over a fresh TCP connection. Optionally
    /// attaches an `Authorization: Bearer <token>` header and a JSON body.
    /// Returns `(status_code, body)`.
    fn http_request(
        port: u16,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        body_json: Option<&str>,
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

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
        stream.write_all(req.as_bytes()).unwrap();

        // Read the full response body in one shot rather than line-by-line.
        // The previous BufReader::read_line approach surfaced a flaky
        // status==0 on macOS when the server closes immediately after a
        // small response (e.g. an empty 401 body): partial-buffer drops
        // can erase the status line that did arrive.
        //
        // Important: do NOT shutdown(Write) before reading.  Half-closing
        // here causes hyper on the server side to see FIN mid-parse and
        // abort the request without responding.
        let mut buf = Vec::new();
        let _ = (&stream).read_to_end(&mut buf);
        let response = String::from_utf8_lossy(&buf).into_owned();

        let status = response
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or_else(|| {
                panic!(
                    "could not parse HTTP status from response (read {} bytes): <<<{response}>>>",
                    buf.len()
                )
            });
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
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
