#![cfg(all(feature = "daemon", feature = "visual"))]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn build_app(idx: cxpak::index::CodebaseIndex) -> axum::Router {
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

fn empty_index() -> cxpak::index::CodebaseIndex {
    cxpak::index::CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &cxpak::budget::counter::TokenCounter::new(),
        std::collections::HashMap::new(),
    )
}

async fn post(app: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn v1_risks_zero_files_returns_empty_envelope() {
    let (status, body) = post(build_app(empty_index()), "/v1/risks", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("risks").is_some());
    assert_eq!(body["risks"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn v1_data_flow_accepts_angle_brackets() {
    let (status, _body) = post(
        build_app(empty_index()),
        "/v1/data_flow",
        serde_json::json!({"symbol": "Vec<String>"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "generics must be allowed in symbol names"
    );
}

#[tokio::test]
async fn v1_data_flow_rejects_path_separator_in_symbol() {
    let (status, body) = post(
        build_app(empty_index()),
        "/v1/data_flow",
        serde_json::json!({"symbol": "../secret"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_param");
}

#[tokio::test]
async fn v1_risks_focus_workspace_normalized() {
    let (status, _body) = post(
        build_app(empty_index()),
        "/v1/risks",
        serde_json::json!({"workspace": "../etc"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_risks_empty_focus_treated_as_none() {
    let (status, _body) = post(
        build_app(empty_index()),
        "/v1/risks",
        serde_json::json!({"focus": ""}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
