#![cfg(feature = "daemon")]
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn build_app() -> axum::Router {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 100,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert("src/main.rs".into(), "fn main(){}".into());
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        files,
        std::collections::HashMap::new(),
        &counter,
        c,
    );
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
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

fn is_stub(body: &Value) -> bool {
    body.get("status")
        .and_then(|s| s.as_str())
        .map(|s| s == "not_implemented" || s == "available")
        .unwrap_or(false)
}

#[tokio::test]
async fn v1_risks_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/risks", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
    assert!(body.get("risks").is_some(), "envelope must have risks key");
}

#[tokio::test]
async fn v1_architecture_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/architecture", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
    assert!(body.get("modules").is_some());
}

#[tokio::test]
async fn v1_predict_missing_files_returns_400() {
    let (status, body) = post(build_app(), "/v1/predict", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "missing_required_param");
}

#[tokio::test]
async fn v1_predict_with_files_ok() {
    let (status, body) = post(
        build_app(),
        "/v1/predict",
        serde_json::json!({"files":["src/main.rs"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_data_flow_missing_symbol_returns_400() {
    let (status, _body) = post(build_app(), "/v1/data_flow", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_data_flow_with_symbol_ok() {
    let (status, body) = post(
        build_app(),
        "/v1/data_flow",
        serde_json::json!({"symbol":"main"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_predict_depth_over_cap_returns_400() {
    let (status, body) = post(
        build_app(),
        "/v1/predict",
        serde_json::json!({"files":["src/main.rs"], "depth": 99}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "depth_exceeds_max");
}

#[tokio::test]
async fn v1_drift_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/drift", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_security_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/security_surface", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_dead_code_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/dead_code", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
    assert!(body.get("dead_symbols").is_some());
}

#[tokio::test]
async fn v1_call_graph_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/call_graph", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_cross_lang_returns_non_stub() {
    let (status, body) = post(build_app(), "/v1/cross_lang", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_stub(&body));
}

#[tokio::test]
async fn v1_predict_rejects_traversal_in_focus() {
    let (status, body) = post(
        build_app(),
        "/v1/predict",
        serde_json::json!({
            "files": ["src/main.rs"],
            "focus": "../../../etc/passwd"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_param");
}

#[tokio::test]
async fn v1_data_flow_rejects_traversal_in_focus() {
    let (status, body) = post(
        build_app(),
        "/v1/data_flow",
        serde_json::json!({
            "symbol": "main",
            "focus": "../../../etc/passwd"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_param");
}
