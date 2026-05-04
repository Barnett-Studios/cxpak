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
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
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

/// Drive the MCP `tools/call` channel and parse the embedded JSON result.
///
/// MCP responses are JSON-RPC envelopes with the actual tool payload
/// pretty-printed inside `result.content[0].text` (per MCP spec).  This
/// helper extracts and re-parses that string so tests can assert
/// byte-identical parity against the SPA / v1 / LSP channels.
fn call_mcp(idx: &cxpak::index::CodebaseIndex, tool: &str, args: Value) -> Value {
    let snapshot: cxpak::commands::serve::SharedSnapshot =
        std::sync::Arc::new(std::sync::RwLock::new(None));
    let envelope = cxpak::commands::serve::handle_tool_call(
        Some(Value::Number(1.into())),
        tool,
        &args,
        idx,
        std::path::Path::new("/tmp"),
        &snapshot,
    );
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("MCP {tool} envelope missing result.content[0].text: {envelope}"))
        .to_string();
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("MCP {tool} content text not JSON ({e}): {text}"))
}

#[tokio::test]
async fn health_consistency_spa_v1_mcp_lsp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::health::compute_health(&idx);

    // P4 hardening: bilateral comparison.  Previously this compared the
    // raw Rust f64 against the JSON-extracted f64 — works in practice
    // because serde_json's Ryu serialiser is bit-stable for normal f64,
    // but the rationale was wrong.  Now we round-trip the EXPECTED
    // value through the same serialiser the SPA uses, then compare bit
    // patterns of the two JSON-derived values.  If anyone enables
    // `arbitrary_precision` or otherwise breaks Ryu bit-stability, both
    // sides would shift identically — the SHARED-CHANNEL invariant
    // ("both renderers serialise the same way") holds.
    let expected_json = serde_json::to_value(expected.composite).unwrap();
    let expected_roundtripped = expected_json.as_f64().unwrap();
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_dashboard = extract_json_tag(&spa_html, "cxpak-dashboard");
    let spa_composite = spa_dashboard["health"]["composite"].as_f64().unwrap();
    assert_eq!(
        spa_composite.to_bits(),
        expected_roundtripped.to_bits(),
        "SPA health composite drift after JSON round-trip — bilateral check"
    );
    // Also assert the round-trip itself didn't lose precision.  This
    // catches a future arbitrary_precision-style change that would
    // otherwise silently let both sides drift in lockstep.
    assert_eq!(
        expected_roundtripped.to_bits(),
        expected.composite.to_bits(),
        "JSON round-trip of composite f64 dropped precision — \
         serializer is no longer bit-stable for HealthScore values"
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

    // MCP cxpak_health — function name has had `_mcp_` in it for two
    // releases but the body never invoked the MCP channel.  Closes that
    // coverage gap with a real MCP dispatch + bit-identical assertion.
    let idx3 = make_fixture_index();
    let mcp = call_mcp(&idx3, "cxpak_health", serde_json::json!({}));
    let mcp_composite = mcp["composite"]
        .as_f64()
        .expect("MCP cxpak_health must expose composite");
    assert_eq!(
        mcp_composite.to_bits(),
        expected.composite.to_bits(),
        "MCP health drift"
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

    // MCP cxpak_risks — function name promised _mcp coverage but the
    // body never invoked the channel.  MCP returns a top-level array
    // (no `risks` envelope), unlike v1 which wraps in `{"risks":[…]}`.
    // The schema divergence between channels is itself a finding worth
    // flagging — but for now this test asserts byte-identical risk
    // scores against the same reference.
    let idx_mcp = make_fixture_index();
    let mcp = call_mcp(&idx_mcp, "cxpak_risks", serde_json::json!({}));
    let mcp_risks = mcp.as_array().unwrap_or_else(|| {
        panic!("MCP cxpak_risks must return a top-level JSON array, got: {mcp}")
    });
    assert!(
        !mcp_risks.is_empty(),
        "MCP cxpak_risks must return at least one entry on a non-empty index"
    );
    for (i, entry) in mcp_risks.iter().enumerate() {
        if i >= expected.len() {
            break;
        }
        assert_eq!(
            entry["path"].as_str().unwrap(),
            expected[i].path,
            "MCP risk path drift at index {i}"
        );
        assert_eq!(
            entry["risk_score"].as_f64().unwrap().to_bits(),
            expected[i].risk_score.to_bits(),
            "MCP risk_score drift at index {i}"
        );
    }
}

#[tokio::test]
async fn architecture_consistency_spa_v1_mcp() {
    let idx = make_fixture_index();
    let expected = cxpak::intelligence::architecture::build_architecture_map(&idx, 2);

    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let spa_arch = extract_json_tag(&spa_html, "cxpak-explorer");
    let spa_nodes = spa_arch["level1"]["nodes"].as_array().unwrap();
    // P2 hardening: assert explicit cardinality BEFORE BTreeSet
    // construction so a future bug that emits duplicate nodes (e.g.,
    // same module appearing twice as separate node objects) is caught.
    // BTreeSet equality alone silently dedupes and would let this slip.
    assert_eq!(
        spa_nodes.len(),
        expected.modules.len(),
        "SPA level1 node array length must equal architecture module count — \
         duplicates would dedupe through BTreeSet and silently pass otherwise"
    );
    let spa_prefixes: std::collections::BTreeSet<&str> =
        spa_nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    assert_eq!(
        spa_prefixes.len(),
        spa_nodes.len(),
        "no duplicate module IDs in SPA level1 nodes — architecture must \
         emit each module exactly once"
    );
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

    // MCP cxpak_architecture — coverage gap previously masked by the
    // `_spa_v1` function name not promising MCP coverage.  Now both
    // promise and assertion are present.
    let idx_mcp = make_fixture_index();
    let mcp = call_mcp(&idx_mcp, "cxpak_architecture", serde_json::json!({}));
    let mcp_modules = mcp["modules"]
        .as_array()
        .expect("MCP cxpak_architecture must return `modules` array");
    let mcp_prefixes: std::collections::BTreeSet<String> = mcp_modules
        .iter()
        .filter_map(|m| m["prefix"].as_str().map(String::from))
        .collect();
    let mcp_refs: std::collections::BTreeSet<&str> =
        mcp_prefixes.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        mcp_refs, expected_prefixes,
        "MCP architecture prefix-set drift"
    );
}

#[tokio::test]
async fn dead_code_consistency_v1_lsp_mcp() {
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

    // MCP cxpak_dead_code — was not in any cross-channel test, despite
    // being a fully-wired dispatcher.  v1, LSP, and now MCP MUST all
    // report the same `total` (cardinality of the dead-symbol set).
    let idx3 = make_fixture_index();
    let mcp = call_mcp(&idx3, "cxpak_dead_code", serde_json::json!({}));
    let mcp_total = mcp["total"]
        .as_u64()
        .expect("MCP cxpak_dead_code must expose `total`") as usize;
    assert_eq!(
        mcp_total,
        expected.len(),
        "MCP dead_code total drift vs reference"
    );
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
    // Both sides MUST go through `DependencyGraph::edge_count()` — the
    // shared helper added in v2.1.3.  Without it the renderer and the
    // test inlined identical lambdas in two files; if the inline logic
    // ever drifted (e.g., to skip a new edge type), the test would
    // continue to pass while the dashboard underreported.
    let idx = make_fixture_index();
    let expected: usize = idx.graph.edge_count();
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let meta = extract_json_tag(&spa_html, "cxpak-meta");
    assert_eq!(meta["edge_count"].as_u64().unwrap() as usize, expected);
}

#[tokio::test]
async fn metadata_health_score_matches_health_cached() {
    // Pre-fix bug (caught during v2.1.0 manual QA): commands/visual.rs and
    // commands/serve.rs both hardcoded `health_score: None` when building the
    // SPA's RenderMetadata, so the SPA header showed "—" while the dashboard
    // tile next to it showed the real composite from `compute_health`.  All
    // other channels (/v1/health, MCP cxpak_health, SPA dashboard data) read
    // from `index.health_cached()`; this test pins the SPA meta to the same
    // single source of truth so the four surfaces can never drift again.
    let idx = make_fixture_index();
    let expected = idx.health_cached().composite;
    assert!(
        expected.is_finite(),
        "compute_health must produce a finite composite for non-empty index, got {expected}"
    );
    // Production path: commands::visual::make_metadata is the canonical
    // builder. We call it directly here (rather than via subprocess) so a
    // future refactor that moves the field can't silently re-introduce the
    // null-default behaviour.
    let meta = cxpak::commands::visual::make_metadata(&idx, std::path::Path::new("."));
    assert_eq!(
        meta.health_score,
        Some(expected),
        "make_metadata must surface health_cached().composite bit-for-bit"
    );
    // And confirm it survives the SPA serialization round-trip.
    let spa_html = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    let serialized = extract_json_tag(&spa_html, "cxpak-meta");
    let actual = serialized["health_score"]
        .as_f64()
        .expect("cxpak-meta.health_score must serialize as a JSON number, not null");
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "SPA meta health_score must round-trip bit-identically; got {actual} (bits {actual_bits:#x}) vs expected {expected} (bits {expected_bits:#x})",
        actual_bits = actual.to_bits(),
        expected_bits = expected.to_bits()
    );
}

/// Source-pin: the MCP cxpak_visual handler MUST route metadata
/// construction through `commands::visual::make_metadata`, NOT inline its
/// own copy.  Pre-fix the handler was a hand-duplicate of make_metadata's
/// body — exactly the divergence pattern that hid the original
/// `health_score: null` bug (one branch updated, the other not).
///
/// Source-pin (rather than calling the handler in-process) because the
/// handler signature is closure-captured inside the MCP dispatch
/// `match tool_name { ... }` arm and is impractical to invoke directly
/// from tests.  A subprocess-driven MCP test (mcp_stdio_framing.rs)
/// covers the runtime parity; this test pins the architectural contract.
#[tokio::test]
async fn mcp_cxpak_visual_routes_through_make_metadata() {
    let source = include_str!("../src/commands/serve.rs");
    let cxpak_visual_marker = "cxpak_visual";
    // Find the actual dispatch arm `"cxpak_visual" => { ... }` (not the
    // tools/list schema string).  The dispatch arm uniquely matches the
    // pattern with `=>` immediately after.
    let idx = source
        .find(r#""cxpak_visual" => {"#)
        .expect("MCP cxpak_visual handler dispatch arm must exist");
    let window = &source[idx..idx + 4000];
    assert!(
        window.contains("commands::visual::make_metadata")
            || window.contains("crate::commands::visual::make_metadata"),
        "MCP `{cxpak_visual_marker}` handler MUST call commands::visual::make_metadata \
         instead of inlining the RenderMetadata construction; pre-this-contract the \
         inline copy let the SPA meta health_score field drift to null while the CLI \
         path was correct.  Window starting at the cxpak_visual marker:\n{window}"
    );
    // And the inline RenderMetadata { ... } construction must NOT exist
    // in this window — if it does, someone re-introduced the duplication.
    let inline_construct = "RenderMetadata {";
    assert!(
        !window.contains(inline_construct),
        "MCP cxpak_visual handler must not inline `RenderMetadata {{ ... }}` — route \
         through make_metadata.  Found inline construction in the handler window."
    );
}
