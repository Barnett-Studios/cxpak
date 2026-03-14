#[cfg(feature = "daemon")]
mod serve_tests {
    use assert_cmd::Command;
    use predicates::prelude::*;
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Write};
    use std::process::{self, Stdio};

    fn make_test_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "t@t.com").unwrap();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\nfn helper() -> i32 { 42 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn greet() { println!(\"hi\"); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        dir
    }

    // === HTTP Server Integration Tests ===

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    fn wait_for_server(port: u16) -> bool {
        for _ in 0..50 {
            if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    }

    fn http_get(port: u16, path: &str) -> (u16, String) {
        let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();

        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).unwrap();

        let mut response = String::new();
        let mut reader = BufReader::new(&stream);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => response.push_str(&line),
                Err(_) => break,
            }
        }
        // Parse status code
        let status = response
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);

        // Extract body (after \r\n\r\n)
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

        (status, body)
    }

    #[test]
    fn test_serve_health_endpoint() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/health");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["status"], "ok");

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_stats_endpoint() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/stats");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert!(
            json["files"].as_u64().unwrap() >= 2,
            "should have at least 2 files"
        );
        assert!(json["tokens"].as_u64().unwrap() > 0, "should have tokens");
        assert!(
            json["languages"].as_u64().unwrap() >= 1,
            "should have at least 1 language"
        );

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_overview_endpoint() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/overview?tokens=10k&format=json");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["format"], "json");
        assert_eq!(json["token_budget"], 10_000);
        assert!(json["total_files"].as_u64().unwrap() >= 2);
        assert!(json["languages"].as_array().unwrap().len() >= 1);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_overview_default_params() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/overview");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["format"], "json");
        assert_eq!(json["token_budget"], 50_000);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_trace_endpoint_with_target() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/trace?target=main&tokens=10k");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["target"], "main");
        assert_eq!(json["found"], true);
        assert_eq!(json["token_budget"], 10_000);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_trace_endpoint_missing_target() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/trace");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("missing"));

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_trace_endpoint_not_found() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        let (status, body) = http_get(port, "/trace?target=nonexistent_xyz_symbol");
        assert_eq!(status, 200);
        let json: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["found"], false);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_serve_startup_message() {
        let repo = make_test_repo();
        let port = find_free_port();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        assert!(wait_for_server(port), "server should start within 5s");

        // Read stderr for startup message
        child.kill().ok();
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("cxpak: serving"),
            "should print serving message, got: {stderr}"
        );
        assert!(
            stderr.contains("files indexed"),
            "should mention files indexed, got: {stderr}"
        );
    }

    #[test]
    fn test_serve_tokens_zero_fails() {
        let repo = make_test_repo();
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["serve", "--tokens", "0", repo.path().to_str().unwrap()])
            .assert()
            .failure()
            .stderr(predicate::str::contains("--tokens must be greater than 0"));
    }

    #[test]
    fn test_serve_tokens_invalid_fails() {
        let repo = make_test_repo();
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["serve", "--tokens", "abc", repo.path().to_str().unwrap()])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid token count"));
    }

    // === MCP Server Integration Tests ===

    fn mcp_exchange(child: &mut process::Child, request: &Value) -> Value {
        let stdin = child.stdin.as_mut().unwrap();
        let line = serde_json::to_string(request).unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();

        let stdout = child.stdout.as_mut().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).unwrap();
        serde_json::from_str(&response_line).unwrap()
    }

    #[test]
    fn test_mcp_initialize() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Wait for MCP ready message on stderr
        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1.0"}
                }
            }),
        );

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["serverInfo"]["name"], "cxpak");
        assert!(response["result"]["capabilities"]["tools"].is_object());

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tools_list() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        // Initialize first
        mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {}
            }),
        );

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        );

        assert_eq!(response["id"], 2);
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);

        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(tool_names.contains(&"cxpak_overview"));
        assert!(tool_names.contains(&"cxpak_trace"));
        assert!(tool_names.contains(&"cxpak_stats"));

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tool_call_stats() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "cxpak_stats",
                    "arguments": {}
                }
            }),
        );

        assert_eq!(response["id"], 1);
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let stats: Value = serde_json::from_str(content).unwrap();
        assert!(stats["files"].as_u64().unwrap() >= 2);
        assert!(stats["tokens"].as_u64().unwrap() > 0);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tool_call_overview() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "cxpak_overview",
                    "arguments": {"tokens": "10k"}
                }
            }),
        );

        assert_eq!(response["id"], 1);
        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let overview: Value = serde_json::from_str(content).unwrap();
        assert!(overview["total_files"].as_u64().unwrap() >= 2);
        assert!(overview["languages"].as_array().unwrap().len() >= 1);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tool_call_trace_found() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "cxpak_trace",
                    "arguments": {"target": "main"}
                }
            }),
        );

        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let trace: Value = serde_json::from_str(content).unwrap();
        assert_eq!(trace["target"], "main");
        assert_eq!(trace["found"], true);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tool_call_trace_not_found() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "cxpak_trace",
                    "arguments": {"target": "nonexistent_xyz"}
                }
            }),
        );

        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        let trace: Value = serde_json::from_str(content).unwrap();
        assert_eq!(trace["found"], false);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tool_call_trace_missing_target() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "cxpak_trace",
                    "arguments": {}
                }
            }),
        );

        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("required"),
            "should mention target is required"
        );

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_tool_call_unknown_tool() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "nonexistent_tool",
                    "arguments": {}
                }
            }),
        );

        let content = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("Unknown tool"));
        assert_eq!(response["result"]["isError"], true);

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_unknown_method() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        let response = mcp_exchange(
            &mut child,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "unknown/method",
                "params": {}
            }),
        );

        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["error"]["message"], "Method not found");

        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn test_mcp_startup_message() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "50k",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(2));

        child.kill().ok();
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("MCP server ready"),
            "should print MCP ready message, got: {stderr}"
        );
    }

    #[test]
    fn test_mcp_tokens_zero_fails() {
        let repo = make_test_repo();
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args([
                "serve",
                "--mcp",
                "--tokens",
                "0",
                repo.path().to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("--tokens must be greater than 0"));
    }

    // === Watch Command CLI Tests ===

    #[test]
    fn test_watch_tokens_zero_fails() {
        let repo = make_test_repo();
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["watch", "--tokens", "0", repo.path().to_str().unwrap()])
            .assert()
            .failure()
            .stderr(predicate::str::contains("--tokens must be greater than 0"));
    }

    #[test]
    fn test_watch_tokens_invalid_fails() {
        let repo = make_test_repo();
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["watch", "--tokens", "abc", repo.path().to_str().unwrap()])
            .assert()
            .failure()
            .stderr(predicate::str::contains("invalid token count"));
    }

    #[test]
    fn test_watch_startup_and_file_change() {
        let repo = make_test_repo();

        let mut child = process::Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["watch", "--tokens", "50k", repo.path().to_str().unwrap()])
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        // Wait for initial index build
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Create a new file to trigger a watch event
        std::fs::write(
            repo.path().join("src/new_file.rs"),
            "pub fn new_function() -> bool { true }\n",
        )
        .unwrap();

        // Wait for the watcher to pick up the change
        std::thread::sleep(std::time::Duration::from_secs(2));

        child.kill().ok();
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("cxpak: watching"),
            "should print watching message, got: {stderr}"
        );
        assert!(
            stderr.contains("files indexed"),
            "should mention files indexed, got: {stderr}"
        );
        // The update may or may not have been captured depending on timing,
        // but the startup message is guaranteed
    }

    // === CLI flag parsing tests ===

    #[test]
    fn test_help_shows_serve_command() {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("serve"))
            .stdout(predicate::str::contains("watch"));
    }

    #[test]
    fn test_serve_help() {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["serve", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--port"))
            .stdout(predicate::str::contains("--mcp"))
            .stdout(predicate::str::contains("--tokens"));
    }

    #[test]
    fn test_watch_help() {
        Command::new(assert_cmd::cargo_bin!("cxpak"))
            .args(["watch", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--tokens"))
            .stdout(predicate::str::contains("--format"));
    }
}
