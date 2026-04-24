#![cfg(all(feature = "visual", feature = "daemon", feature = "lsp"))]

mod support;
#[allow(unused_imports)]
use support::redact::redact;

use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt;

fn make_fixture_index() -> cxpak::index::CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files: Vec<cxpak::scanner::ScannedFile> = (0..10)
        .map(|i| cxpak::scanner::ScannedFile {
            relative_path: format!("src/mod_{i}.rs"),
            absolute_path: format!("/tmp/src/mod_{i}.rs").into(),
            language: Some("rust".into()),
            size_bytes: ((i + 1) * 100) as u64,
        })
        .collect();
    let mut pr = std::collections::HashMap::new();
    for (i, file) in files.iter().enumerate() {
        pr.insert(
            file.relative_path.clone(),
            cxpak::parser::language::ParseResult {
                symbols: (0..3)
                    .map(|j| cxpak::parser::language::Symbol {
                        name: format!("fn_{i}_{j}"),
                        kind: cxpak::parser::language::SymbolKind::Function,
                        visibility: if j == 0 {
                            cxpak::parser::language::Visibility::Public
                        } else {
                            cxpak::parser::language::Visibility::Private
                        },
                        signature: format!("fn fn_{i}_{j}()"),
                        body: "{}".into(),
                        start_line: j * 4 + 1,
                        end_line: j * 4 + 3,
                    })
                    .collect(),
                imports: if i > 0 {
                    vec![cxpak::parser::language::Import {
                        source: format!("crate::mod_{}", i - 1),
                        names: vec![],
                    }]
                } else {
                    vec![]
                },
                exports: vec![],
            },
        );
    }
    let mut c = std::collections::HashMap::new();
    for f in &files {
        c.insert(f.relative_path.clone(), "fn x(){}".into());
    }
    cxpak::index::CodebaseIndex::build_with_content(files, pr, &counter, c)
}

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    cxpak::visual::render::RenderMetadata {
        repo_name: "test".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: None,
        node_count: 10,
        edge_count: 9,
        cxpak_version: "2.1.0".into(),
    }
}

fn build_router_with_index(idx: cxpak::index::CodebaseIndex) -> axum::Router {
    let shared = std::sync::Arc::new(std::sync::RwLock::new(idx));
    let path = std::sync::Arc::new(std::path::PathBuf::from("."));
    cxpak::commands::serve::build_router_for_test(shared, path)
}

async fn post_v1(app: axum::Router, path: &str, body: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn extract_json_tag(html: &str, tag_id: &str) -> Value {
    let marker = format!(r#"id="{tag_id}" type="application/json">"#);
    let start = html.find(&marker).expect("tag present") + marker.len();
    let end = html[start..].find("</script>").unwrap() + start;
    serde_json::from_str(&html[start..end]).expect("valid JSON")
}

async fn get_v1(app: axum::Router, path: &str) -> Value {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_consistency_spa_v1_mcp_lsp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::health::compute_health(&idx);

    // SPA — dashboard JSON embeds the full HealthQuadrant.
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_dashboard = extract_json_tag(&spa_html, "cxpak-dashboard");
    let spa_composite = spa_dashboard["health"]["composite"].as_f64().unwrap();
    assert_eq!(
        spa_composite.to_bits(),
        expected.composite.to_bits(),
        "SPA health composite drift"
    );

    // v1/health is GET, not POST.
    let v1_body = get_v1(build_router_with_index(make_fixture_index()), "/v1/health").await;
    let v1_composite = v1_body["composite"]
        .as_f64()
        .expect("v1/health must expose composite (Task 12b)");
    assert_eq!(
        v1_composite.to_bits(),
        expected.composite.to_bits(),
        "v1 health drift"
    );

    // LSP cxpak/health (updated in Task 12b).
    let idx2 = make_fixture_index();
    let lsp = cxpak::lsp::methods::handle_custom_method(
        "cxpak/health",
        Value::Null,
        &idx2,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .expect("Some");
    let lsp_composite = lsp["composite"]
        .as_f64()
        .expect("LSP cxpak/health must expose composite (Task 12b)");
    assert_eq!(
        lsp_composite.to_bits(),
        expected.composite.to_bits(),
        "LSP health drift"
    );
}

#[tokio::test]
async fn risk_consistency_spa_v1_mcp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::risk::compute_risk_ranking(&idx);

    // SPA top_risks is first-5 of expected
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_dashboard = extract_json_tag(&spa_html, "cxpak-dashboard");
    let spa_top = spa_dashboard["risks"]["top_risks"].as_array().unwrap();
    for (i, entry) in spa_top.iter().enumerate() {
        let real = &expected[i];
        assert_eq!(entry["path"].as_str().unwrap(), real.path);
        assert_eq!(
            entry["risk_score"].as_f64().unwrap().to_bits(),
            real.risk_score.to_bits()
        );
    }

    // v1/risks returns full list
    let v1_body = post_v1(
        build_router_with_index(make_fixture_index()),
        "/v1/risks",
        serde_json::json!({}),
    )
    .await;
    let v1_risks = v1_body["risks"].as_array().unwrap();
    assert_eq!(v1_risks.len(), expected.len());
    for (i, entry) in v1_risks.iter().enumerate() {
        assert_eq!(entry["path"].as_str().unwrap(), expected[i].path);
        assert_eq!(
            entry["risk_score"].as_f64().unwrap().to_bits(),
            expected[i].risk_score.to_bits()
        );
    }
}

#[tokio::test]
async fn architecture_consistency_spa_v1() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::architecture::build_architecture_map(&idx, 2);

    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_arch = extract_json_tag(&spa_html, "cxpak-explorer");
    let spa_nodes = spa_arch["level1"]["nodes"].as_array().unwrap();
    let spa_prefixes: std::collections::BTreeSet<&str> =
        spa_nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    let expected_prefixes: std::collections::BTreeSet<&str> =
        expected.modules.iter().map(|m| m.prefix.as_str()).collect();
    assert_eq!(spa_prefixes, expected_prefixes);

    let v1_body = post_v1(
        build_router_with_index(make_fixture_index()),
        "/v1/architecture",
        serde_json::json!({}),
    )
    .await;
    let v1_modules = v1_body["modules"].as_array().unwrap();
    let v1_prefixes: std::collections::BTreeSet<&str> = v1_modules
        .iter()
        .filter_map(|m| m["prefix"].as_str())
        .collect();
    assert_eq!(v1_prefixes, expected_prefixes);
}

#[tokio::test]
async fn dead_code_consistency_v1_lsp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::dead_code::detect_dead_code(&idx, None);
    let v1_body = post_v1(
        build_router_with_index(make_fixture_index()),
        "/v1/dead_code",
        serde_json::json!({}),
    )
    .await;
    let v1_count = v1_body["dead_symbols"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(v1_count, expected.len());
    let idx2 = make_fixture_index();
    let lsp = cxpak::lsp::methods::handle_custom_method(
        "cxpak/deadCode",
        Value::Null,
        &idx2,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .expect("Some");
    let lsp_count = lsp
        .as_array()
        .map(|a| a.len())
        .or_else(|| lsp["dead_symbols"].as_array().map(|a| a.len()))
        .unwrap_or(0);
    assert_eq!(lsp_count, expected.len());
}

#[tokio::test]
async fn metadata_node_count_matches_total_files() {
    let idx = make_fixture_index();
    let expected = idx.total_files;
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let meta = extract_json_tag(&spa_html, "cxpak-meta");
    assert_eq!(meta["node_count"].as_u64().unwrap() as usize, expected);
}

#[tokio::test]
async fn metadata_edge_count_matches_graph_sum() {
    let idx = make_fixture_index();
    let expected: usize = idx.graph.edges.values().map(|v| v.len()).sum();
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let meta = extract_json_tag(&spa_html, "cxpak-meta");
    assert_eq!(meta["edge_count"].as_u64().unwrap() as usize, expected);
}
