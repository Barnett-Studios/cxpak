#![cfg(all(feature = "daemon", feature = "visual"))]

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn empty_app() -> axum::Router {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

#[tokio::test]
async fn v1_health_exposes_composite_field() {
    // /v1/health is a GET route (existing behavior) — not POST.
    let req = Request::builder()
        .method("GET")
        .uri("/v1/health")
        .body(Body::empty())
        .unwrap();
    let resp = empty_app().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        body.get("composite").is_some(),
        "v1/health must expose composite: {body}"
    );
}

#[test]
#[cfg(feature = "lsp")]
fn lsp_health_exposes_composite_field() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let result = cxpak::lsp::methods::handle_custom_method(
        "cxpak/health",
        serde_json::Value::Null,
        &idx,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .unwrap();
    assert!(
        result.get("composite").is_some(),
        "LSP cxpak/health must expose composite: {result}"
    );
}
