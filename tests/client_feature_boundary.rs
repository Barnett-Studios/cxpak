//! RED first (cxpak#57): the official Rust client, and the feature boundary that keeps it
//! out of a default `cargo install`.
//!
//! Against `main` this file compiles to nothing and asserts nothing, because `cxpak` has no
//! `client` feature at all — `cargo test --features client` fails with "the package 'cxpak'
//! does not contain this feature: client". That is the RED state.
//!
//! Mutation to re-run on revert: delete `src/client.rs` and its `pub mod client;`, and drop
//! the `client` feature from `Cargo.toml`. This returns to that same error.
//!
//! The boundary is the point of the `cfg`. cxpak has zero rmcp references today and #32
//! already records the default feature set as a kitchen sink; a client that quietly lands in
//! the default build regresses every user who wants an indexer and not an MCP client. Under
//! default features this file is empty, and that emptiness is an assertion in its own right.
#![cfg(feature = "client")]

use cxpak::client::{CxpakClient, RecordedCxpakClient};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The replay client is what every downstream consumer's tests actually construct, so it is
/// the surface that has to survive the move intact — moving it and changing its shape would
/// break them at compile time, which is the good case, or silently, which is not.
#[tokio::test]
async fn recorded_client_replays_a_committed_recording() {
    let mut recordings = HashMap::new();
    recordings.insert("overview".to_string(), json!({"health": {"score": 0.9}}));

    let client = RecordedCxpakClient::new(recordings);
    let got = client
        .call("overview", Value::Null)
        .await
        .expect("a recorded tool must replay rather than miss");

    assert_eq!(
        got["health"]["score"], 0.9,
        "the recorded payload must round-trip verbatim, got: {got}"
    );
}

/// `None` is the miss signal the whole seam is built on: callers map it to
/// `Observation::Skipped` rather than to a verdict. A client that fabricated an empty
/// success here would turn "cxpak was unavailable" into "cxpak found nothing", which is the
/// silent-false-negative this contract exists to prevent.
#[tokio::test]
async fn an_unrecorded_tool_is_a_miss_not_an_empty_success() {
    let client = RecordedCxpakClient::new(HashMap::new());

    assert!(
        client.call("overview", Value::Null).await.is_none(),
        "an unrecorded tool must surface as None, never as an empty-but-present bundle"
    );
}

/// `from_dir` is how the framework loads its committed `conformance/recordings/cxpak/`
/// fixtures. A missing directory is a hard error — that is a misconfiguration, not a miss,
/// and conflating the two would let a mis-pathed test suite report a clean run over nothing.
#[tokio::test]
async fn from_dir_rejects_a_missing_directory_rather_than_returning_an_empty_client() {
    let missing = std::path::Path::new("/nonexistent/cxpak/recordings");

    assert!(
        RecordedCxpakClient::from_dir(missing).is_err(),
        "a missing recordings directory must be an error, not an empty client that \
         silently misses every tool"
    );
}
