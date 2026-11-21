//! Live rmcp client path: `RmcpCxpakClient` spawns the real `cxpak serve --mcp` child,
//! performs the initialize handshake and a `tools/call` round trip over stdio. Guards the
//! rmcp major-version bump: replay clients never touch rmcp, so only this test breaks if the
//! transport, handshake or `call_tool` API drifts.
#![cfg(feature = "client")]

use cxpak::client::{CxpakClient, RmcpCxpakClient};
use serde_json::json;

#[tokio::test]
async fn rmcp_client_round_trips_a_tool_call_against_the_real_server() {
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_cxpak"))
        .parent()
        .expect("cargo bin path has a parent")
        .to_path_buf();
    let mut paths = vec![bin_dir];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let joined = std::env::join_paths(paths).expect("PATH entries are joinable");
    // Single test in this binary: no concurrent env readers.
    std::env::set_var("PATH", joined);

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("lib.rs"), "pub fn f() -> u8 { 1 }\n").expect("write fixture");

    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .expect("git runs")
            .success()
    };
    assert!(git(&["init", "-q"]) && git(&["add", "."]) && git(&["commit", "-qm", "fixture"]));

    let client = RmcpCxpakClient::new(dir.path().to_path_buf());
    let got = client.call("cxpak_health", json!({})).await;
    assert!(
        got.as_ref().is_some_and(|v| v.is_object()),
        "expected a JSON object from the live server, got {got:?}"
    );
}
