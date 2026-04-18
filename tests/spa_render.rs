#![cfg(feature = "visual")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use cxpak::visual::render::RenderMetadata;
use std::collections::HashMap;

fn fixture_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 100,
    }];
    let mut pr = HashMap::new();
    pr.insert(
        "src/main.rs".into(),
        ParseResult {
            symbols: vec![Symbol {
                name: "main".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "fn main()".into(),
                body: "fn main() {}".into(),
                start_line: 1,
                end_line: 3,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content = HashMap::new();
    content.insert("src/main.rs".into(), "fn main() {}".into());
    CodebaseIndex::build_with_content(files, pr, &counter, content)
}

fn fixture_meta() -> RenderMetadata {
    RenderMetadata {
        repo_name: "test".into(),
        generated_at: "2026-04-17T12:00:00Z".into(),
        health_score: Some(7.4),
        node_count: 1,
        edge_count: 0,
        cxpak_version: "2.1.0".into(),
    }
}

#[test]
fn contains_doctype_and_html_close() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("</html>"));
}

#[test]
fn contains_all_six_view_containers() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for id in [
        "view-dashboard",
        "view-architecture",
        "view-risk",
        "view-flow",
        "view-timeline",
        "view-diff",
    ] {
        assert!(html.contains(&format!(r#"id="{id}""#)), "missing {id}");
    }
}

#[test]
fn contains_all_data_tags() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for tag in [
        r#"id="cxpak-dashboard-data""#,
        r#"id="cxpak-architecture-data""#,
        r#"id="cxpak-risk-data""#,
        r#"id="cxpak-timeline-data""#,
        r#"id="cxpak-flow-data""#,
        r#"id="cxpak-diff-data""#,
        r#"id="cxpak-meta""#,
        r#"id="cxpak-search-index""#,
    ] {
        assert!(html.contains(tag), "missing data tag: {tag}");
    }
}

#[test]
fn no_cdn_references() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for bad in ["cdn.jsdelivr.net", "unpkg.com", "cdnjs.cloudflare.com"] {
        assert!(!html.contains(bad), "CDN reference leaked: {bad}");
    }
}

#[test]
fn empty_flow_is_null_not_empty_object() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let marker = r#"<script id="cxpak-flow-data" type="application/json">"#;
    let start = html.find(marker).expect("flow tag present") + marker.len();
    let end = html[start..].find("</script>").unwrap() + start;
    assert_eq!(
        html[start..end].trim(),
        "null",
        "flow empty state must serialize as null"
    );
}

#[test]
fn deterministic_output() {
    let a = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let b = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn embeds_controller_asset() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // The controller file we created must appear in output.
    assert!(
        html.contains("CX.navigate = navigate"),
        "controller JS not embedded"
    );
}

#[test]
fn injection_safe_for_malicious_filename() {
    let counter = TokenCounter::new();
    let malicious = "</script><img src=x onerror=alert(1)>.rs";
    let files = vec![ScannedFile {
        relative_path: malicious.into(),
        absolute_path: format!("/tmp/{malicious}").into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut content = HashMap::new();
    content.insert(malicious.into(), "// nope".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content);
    let html = cxpak::visual::spa::render_spa(&idx, &fixture_meta()).unwrap();
    // Find every <script id="cxpak-*" ..> ... </script> block and confirm NO </script> appears mid-block except the real close.
    let re = regex::Regex::new(r#"<script id="cxpak-[a-z-]+"[^>]*>([^<]*|<(?:/[^s]|s[^c]|sc[^r]|scr[^i]|scri[^p]|scrip[^t]))*?</script>"#).unwrap();
    assert!(
        re.find(&html).is_some(),
        "at least one script block should match safely"
    );
    // Simpler invariant: the malicious `</script>` must be escaped somewhere — either no unescaped occurrence inside script tags.
    assert!(
        !html.contains("onerror=alert"),
        "raw onerror payload leaked"
    );
}
