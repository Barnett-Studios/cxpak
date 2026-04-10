#[cfg(feature = "daemon")]
mod api_v1 {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_router() -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let index = cxpak::commands::serve::build_index(dir.path())
            .unwrap_or_else(|_| cxpak::index::CodebaseIndex::empty());
        let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
        let path = std::sync::Arc::new(dir.path().to_path_buf());
        let router = cxpak::commands::serve::build_router_for_test(shared, path);
        (router, dir)
    }

    fn test_router_with_token(token: &str) -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let index = cxpak::index::CodebaseIndex::empty();
        let shared = std::sync::Arc::new(std::sync::RwLock::new(index));
        let path = std::sync::Arc::new(dir.path().to_path_buf());
        let router = cxpak::commands::serve::build_router_for_test_with_token(
            shared,
            path,
            Some(token.to_string()),
        );
        (router, dir)
    }

    #[tokio::test]
    async fn v1_health_returns_200() {
        let (app, _dir) = test_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/health")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(val.get("total_files").is_some());
    }

    #[tokio::test]
    async fn v1_conventions_returns_profile() {
        let (app, _dir) = test_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/conventions")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(val.get("naming").is_some());
    }

    #[tokio::test]
    async fn v1_auth_rejects_missing_token() {
        let (app, _dir) = test_router_with_token("secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/health")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn v1_auth_accepts_valid_token() {
        let (app, _dir) = test_router_with_token("secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/health")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn v1_briefing_returns_task() {
        let (app, _dir) = test_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/briefing")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"task": "find main entry point"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(val.get("task").is_some());
    }
}
