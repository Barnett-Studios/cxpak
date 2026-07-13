#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use std::collections::HashMap;

fn empty_index() -> CodebaseIndex {
    CodebaseIndex::build_with_content(vec![], HashMap::new(), &TokenCounter::new(), HashMap::new())
}

fn fixture_metadata() -> cxpak::visual::render::RenderMetadata {
    cxpak::visual::render::RenderMetadata {
        repo_name: "t".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: None,
        node_count: 0,
        edge_count: 0,
        cxpak_version: "2.1.0".into(),
    }
}

#[test]
fn spa_renders_with_zero_files_index() {
    let idx = empty_index();
    let html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    // Three-mode IA (ADR-0192): Flow + Diff removed from the SPA nav.
    for id in ["view-dashboard", "view-explore", "view-timeline"] {
        assert!(html.contains(&format!(r#"id="{id}""#)));
    }
}

#[test]
fn spa_all_tags_escaped_for_malicious_filename() {
    let counter = TokenCounter::new();
    let evil = r"src/</script><img src=x onerror=alert(1)>.rs";
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: evil.into(),
        absolute_path: format!("/tmp/{evil}").into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut c = HashMap::new();
    c.insert(evil.into(), "".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, c);
    let html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();

    let mut cursor = 0usize;
    while let Some(open_start) = html[cursor..].find(r#"<script id="cxpak-"#) {
        let abs_open_start = cursor + open_start;
        let open_end = html[abs_open_start..].find('>').unwrap() + abs_open_start + 1;
        let close = html[open_end..].find("</script>").unwrap() + open_end;
        let interior = &html[open_end..close];
        assert!(
            !interior.contains("onerror=alert"),
            "raw XSS payload leaked into {interior}"
        );
        cursor = close + "</script>".len();
    }

    assert!(
        !html.contains(r#"</script><img src=x onerror=alert(1)>"#),
        "payload leaked"
    );
}

#[test]
fn health_gauge_renders_zero_composite() {
    let idx = empty_index();
    let data = cxpak::visual::render::build_dashboard_data(&idx);
    assert!(data.health.composite.is_finite());
    let spa_html = cxpak::visual::spa::render_spa(&idx, &fixture_metadata()).unwrap();
    let tag = r#"id="cxpak-dashboard" type="application/json">"#;
    let start = spa_html.find(tag).unwrap() + tag.len();
    let end = spa_html[start..].find("</script>").unwrap() + start;
    let json: serde_json::Value = serde_json::from_str(&spa_html[start..end]).unwrap();
    assert!(
        json["health"]["composite"].is_f64() || json["health"]["composite"].is_i64(),
        "composite must be numeric even at boundary"
    );
}

#[test]
fn search_index_empty_symbols_file_has_no_error_marker() {
    let counter = TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/empty.rs".into(),
        absolute_path: "/tmp/src/empty.rs".into(),
        language: Some("rust".into()),
        size_bytes: 5,
    }];
    let mut pr = HashMap::new();
    pr.insert(
        "src/empty.rs".into(),
        cxpak::parser::language::ParseResult {
            symbols: vec![],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut c = HashMap::new();
    c.insert("src/empty.rs".into(), "//\n".into());
    let idx = CodebaseIndex::build_with_content(files, pr, &counter, c);
    let entries = cxpak::visual::search_index::build_search_index(&idx);
    let entry = entries.iter().find(|e| e.label == "src/empty.rs").unwrap();
    assert!(
        !entry.detail.contains("parse error"),
        "empty-symbols file must NOT be marked as parse error: {}",
        entry.detail
    );
}
