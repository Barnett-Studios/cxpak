//! Integration tests for the MCP server's `cxpak_visual` and `cxpak_onboard`
//! tool handlers.
//!
//! These tests spawn the MCP server as a child process, exchange JSON-RPC
//! messages over stdio, and assert on the response structure.
//!
//! All tests require the `daemon` feature (for the MCP server itself) plus the
//! `visual` feature for the visual/onboard tool handlers.

#[cfg(all(feature = "daemon", feature = "visual"))]
mod mcp_visual_onboard_tests {
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, ChildStdout, Command as StdCommand, Stdio};
    use tempfile::TempDir;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Build a minimal git repository that the MCP server can index quickly.
    fn make_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
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

    struct McpServer {
        child: Child,
        reader: BufReader<ChildStdout>,
        stdin: ChildStdin,
    }

    impl McpServer {
        fn spawn(repo: &TempDir) -> Self {
            let mut child = StdCommand::new(assert_cmd::cargo_bin!("cxpak"))
                .args(["serve", "--mcp", "--tokens", "50k"])
                .arg(repo.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("failed to spawn cxpak serve --mcp");

            let reader = BufReader::new(child.stdout.take().unwrap());
            let stdin = child.stdin.take().unwrap();

            // Allow the server to complete its index build before sending requests.
            std::thread::sleep(std::time::Duration::from_secs(3));

            McpServer {
                child,
                reader,
                stdin,
            }
        }

        fn send(&mut self, msg: &Value) {
            let s = msg.to_string();
            writeln!(self.stdin, "{s}").unwrap();
            self.stdin.flush().unwrap();
        }

        fn recv(&mut self) -> Value {
            let mut line = String::new();
            self.reader.read_line(&mut line).unwrap();
            serde_json::from_str(line.trim())
                .unwrap_or_else(|e| panic!("failed to parse MCP response: {e}\nline: {line}"))
        }

        fn exchange(&mut self, msg: &Value) -> Value {
            self.send(msg);
            self.recv()
        }

        fn initialize(&mut self) -> Value {
            self.exchange(&json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0.0.1"}
                }
            }))
        }
    }

    impl Drop for McpServer {
        fn drop(&mut self) {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }

    // -------------------------------------------------------------------------
    // tools/list — both tools must be present
    // -------------------------------------------------------------------------

    #[test]
    fn mcp_tools_list_includes_visual_and_onboard() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }));

        let tools = resp["result"]["tools"]
            .as_array()
            .expect("result.tools must be an array");

        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

        assert!(
            names.contains(&"cxpak_visual"),
            "tools/list must include cxpak_visual; got: {names:?}"
        );
        assert!(
            names.contains(&"cxpak_onboard"),
            "tools/list must include cxpak_onboard; got: {names:?}"
        );
    }

    #[test]
    fn mcp_tools_list_visual_has_required_params() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }));

        let tools = resp["result"]["tools"].as_array().unwrap();
        let visual_tool = tools
            .iter()
            .find(|t| t["name"].as_str() == Some("cxpak_visual"))
            .expect("cxpak_visual must be present");

        let props = &visual_tool["inputSchema"]["properties"];
        assert!(
            props.get("type").is_some(),
            "cxpak_visual must have 'type' param, got: {props}"
        );
        assert!(
            props.get("format").is_some(),
            "cxpak_visual must have 'format' param, got: {props}"
        );
        assert!(
            props.get("symbol").is_some(),
            "cxpak_visual must have 'symbol' param, got: {props}"
        );
        assert!(
            props.get("files").is_some(),
            "cxpak_visual must have 'files' param, got: {props}"
        );
    }

    #[test]
    fn mcp_tools_list_onboard_has_required_params() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }));

        let tools = resp["result"]["tools"].as_array().unwrap();
        let onboard_tool = tools
            .iter()
            .find(|t| t["name"].as_str() == Some("cxpak_onboard"))
            .expect("cxpak_onboard must be present");

        let props = &onboard_tool["inputSchema"]["properties"];
        assert!(
            props.get("format").is_some(),
            "cxpak_onboard must have 'format' param, got: {props}"
        );
        assert!(
            props.get("focus").is_some(),
            "cxpak_onboard must have 'focus' param, got: {props}"
        );
    }

    // -------------------------------------------------------------------------
    // cxpak_visual — dashboard returns HTML content
    // -------------------------------------------------------------------------

    #[test]
    fn mcp_visual_dashboard_returns_html() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "dashboard", "format": "html" }
            }
        }));

        assert_eq!(resp["id"], 2);
        // The response must not be an error
        assert_ne!(
            resp["result"]["isError"],
            json!(true),
            "cxpak_visual dashboard must not be an error response, got: {}",
            resp
        );

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("result.content[0].text must be a string");

        // Large HTML may be written to file; we accept either:
        // (a) inline HTML starting with <!DOCTYPE html>
        // (b) a message containing "Output written to" (MCP 1 MB limit)
        let is_html = text.contains("<!DOCTYPE html>");
        let is_file_ref = text.contains("Output written to");
        assert!(
            is_html || is_file_ref,
            "cxpak_visual dashboard must return HTML or a file reference, got first 200 chars: {}",
            &text[..text.len().min(200)]
        );
    }

    #[test]
    fn mcp_visual_architecture_returns_json_parseable_content() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "architecture", "format": "json" }
            }
        }));

        assert_eq!(resp["id"], 3);
        assert_ne!(resp["result"]["isError"], json!(true), "must not be error");

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        let _: serde_json::Value = serde_json::from_str(text)
            .expect("cxpak_visual architecture json must return parseable JSON");
    }

    #[test]
    fn mcp_visual_architecture_returns_mermaid_with_graph_header() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "architecture", "format": "mermaid" }
            }
        }));

        assert_eq!(resp["id"], 4);
        assert_ne!(resp["result"]["isError"], json!(true), "must not be error");

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        let first_line = text.lines().next().unwrap_or("");
        assert!(
            first_line.starts_with("graph") || first_line.starts_with("flowchart"),
            "mermaid content must start with graph/flowchart, got: {first_line}"
        );
    }

    // -------------------------------------------------------------------------
    // cxpak_visual — flow without symbol returns error referencing "symbol"
    // -------------------------------------------------------------------------

    #[test]
    fn mcp_visual_flow_without_symbol_returns_error() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "flow", "format": "html" }
            }
        }));

        assert_eq!(resp["id"], 5);
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("response must have text content");

        assert!(
            text.to_lowercase().contains("symbol") || text.contains("Error"),
            "flow without symbol must mention 'symbol' or 'Error', got: {text}"
        );
    }

    #[test]
    fn mcp_visual_flow_with_symbol_does_not_error() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "flow", "format": "html", "symbol": "main" }
            }
        }));

        assert_eq!(resp["id"], 6);
        // The response must be a non-error tool result
        assert_ne!(
            resp["result"]["isError"],
            json!(true),
            "flow with symbol must not return an error result: {}",
            resp
        );
    }

    // -------------------------------------------------------------------------
    // cxpak_visual — diff without files returns error referencing "files"
    // -------------------------------------------------------------------------

    #[test]
    fn mcp_visual_diff_without_files_returns_error() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "diff", "format": "html" }
            }
        }));

        assert_eq!(resp["id"], 7);
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("response must have text content");

        assert!(
            text.to_lowercase().contains("files") || text.contains("Error"),
            "diff without files must mention 'files' or 'Error', got: {text}"
        );
    }

    // -------------------------------------------------------------------------
    // cxpak_onboard — returns JSON with phases array
    // -------------------------------------------------------------------------

    #[test]
    fn mcp_onboard_json_returns_phases_array() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "cxpak_onboard",
                "arguments": { "format": "json" }
            }
        }));

        assert_eq!(resp["id"], 8);
        assert_ne!(
            resp["result"]["isError"],
            json!(true),
            "cxpak_onboard must not return an error"
        );

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        let j: serde_json::Value =
            serde_json::from_str(text).expect("cxpak_onboard json must return parseable JSON");

        assert!(
            j["phases"].is_array(),
            "onboard JSON must have a 'phases' array, got: {}",
            &text[..text.len().min(300)]
        );
        assert!(
            j["total_files"].is_number(),
            "onboard JSON must have 'total_files'"
        );
        assert!(
            j["estimated_reading_time"].is_string(),
            "onboard JSON must have 'estimated_reading_time'"
        );
    }

    #[test]
    fn mcp_onboard_markdown_contains_heading() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "cxpak_onboard",
                "arguments": { "format": "markdown" }
            }
        }));

        assert_eq!(resp["id"], 9);
        assert_ne!(resp["result"]["isError"], json!(true));

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        assert!(
            text.contains("# Codebase Onboarding Map"),
            "onboard markdown must contain the onboarding heading, got first 300 chars: {}",
            &text[..text.len().min(300)]
        );
        assert!(
            text.contains("Phase"),
            "onboard markdown must mention Phase"
        );
    }

    #[test]
    fn mcp_onboard_default_format_is_json() {
        // When no format is specified the server defaults to "json"
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "cxpak_onboard",
                "arguments": {}
            }
        }));

        assert_eq!(resp["id"], 10);
        assert_ne!(resp["result"]["isError"], json!(true));

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        let j: serde_json::Value =
            serde_json::from_str(text).expect("default onboard format must return valid JSON");
        assert!(
            j["phases"].is_array(),
            "default format must have phases array"
        );
    }

    // -------------------------------------------------------------------------
    // cxpak_visual — SVG and C4 formats are non-empty strings
    // -------------------------------------------------------------------------

    #[test]
    fn mcp_visual_architecture_svg_is_non_empty() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "architecture", "format": "svg" }
            }
        }));

        assert_eq!(resp["id"], 11);
        assert_ne!(resp["result"]["isError"], json!(true));

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        assert!(
            text.contains("<svg"),
            "svg response must contain <svg, got first 200: {}",
            &text[..text.len().min(200)]
        );
    }

    #[test]
    fn mcp_visual_architecture_c4_contains_workspace() {
        let repo = make_test_repo();
        let mut srv = McpServer::spawn(&repo);
        srv.initialize();

        let resp = srv.exchange(&json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "cxpak_visual",
                "arguments": { "type": "architecture", "format": "c4" }
            }
        }));

        assert_eq!(resp["id"], 12);
        assert_ne!(resp["result"]["isError"], json!(true));

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("content must be a string");

        assert!(
            text.contains("workspace"),
            "c4 response must contain 'workspace', got first 200: {}",
            &text[..text.len().min(200)]
        );
    }
}
