//! RED first (cxpak#57): the official Rust client, and the feature boundary that keeps it
//! out of a default `cargo install`.
//!
//! Against `main` this file does not compile: `cxpak::client` does not exist. That is the
//! failure — an absent module, not a wrong value. The mutation to re-run on revert is
//! "delete src/client.rs and its `pub mod client;`", which returns this to a compile error.
//!
//! The boundary test is the one that matters. cxpak has zero rmcp today and #32 already
//! records that the default feature set is a kitchen sink; a client that quietly lands in
//! the default build makes that worse for every user who wants an indexer and not an MCP
//! client. `cargo build` must not compile this module, which is why the whole file is
//! gated: under default features it is empty, and that emptiness is the assertion.
#![cfg(feature = "client")]

use cxpak::client::{CxpakClient, RecordedCxpakClient};
use std::collections::HashMap;

/// The replay client is what every consumer's tests actually use, so it is the surface
/// that has to survive the move intact.
#[tokio::test]
async fn recorded_client_round_trips_a_fixture() {
    let mut map = HashMap::new();
    map.insert(
        ("overview".to_string(), String::new()),
        r#"{"health":{"score":0.9}}"#.to_string(),
    );
    let client = RecordedCxpakClient::new(map);
    let got = client
        .call("overview", "")
        .await
        .expect("a recorded op must replay");
    assert!(
        got.contains("\"score\":0.9"),
        "recorded payload must round-trip verbatim, got: {got}"
    );
}

/// An op with no recording is a miss, not a fabricated empty success — the same
/// fail-open-vs-fabricate distinction the server side draws.
#[tokio::test]
async fn an_unrecorded_op_is_an_error_not_an_empty_success() {
    let client = RecordedCxpakClient::new(HashMap::new());
    assert!(
        client.call("overview", "").await.is_err(),
        "an unrecorded op must surface as an error rather than an empty bundle"
    );
}
