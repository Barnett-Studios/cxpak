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
        r#"id="cxpak-dashboard""#,
        r#"id="cxpak-explorer""#,
        r#"id="cxpak-heatmap""#,
        r#"id="cxpak-timeline""#,
        r#"id="cxpak-flow""#,
        r#"id="cxpak-diff""#,
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
    let marker = r#"<script id="cxpak-flow" type="application/json">"#;
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

#[test]
fn spa_inlines_empty_favicon_to_suppress_404() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert!(
        html.contains(r#"<link rel="icon" href="data:,">"#),
        "SPA must inline empty favicon to suppress console 404"
    );
}

#[test]
fn spa_dashboard_nav_is_spa_aware() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // The navTo function must detect SPA mode before falling back to filename-based nav.
    // Grep the inlined dashboard_js for the SPA-mode guard.
    assert!(
        html.contains("CX.pushHash") && html.contains("window.CX.navigate"),
        "dashboard_js navTo must check for CX.pushHash/CX.navigate before using filename navigation"
    );
}

#[test]
fn spa_inspector_has_aria_attributes() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert!(
        html.contains(r#"role="complementary""#),
        "inspector must have role=complementary"
    );
    assert!(
        html.contains(r#"aria-label="Node details inspector""#),
        "inspector must have aria-label"
    );
}

#[test]
fn spa_palette_has_dialog_aria() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // Palette and help overlay both are modal dialogs.
    assert_eq!(
        html.matches(r#"role="dialog""#).count(),
        2,
        "expected 2 role=dialog elements (palette + help)"
    );
    assert!(
        html.contains(r#"aria-label="Command palette""#),
        "palette must have aria-label"
    );
    assert!(
        html.contains(r#"aria-label="Keyboard shortcuts""#),
        "help overlay must have aria-label"
    );
}

#[test]
fn spa_inspector_aside_no_aria_live() {
    // Per ARIA spec (and Fix I4 in the v2.1.0 quality review), aria-live
    // belongs on a dedicated #cxpak-live region, not the inspector aside.
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let aside_idx = html
        .find(r#"<aside id="cxpak-inspector""#)
        .expect("aside present");
    let aside_end = html[aside_idx..].find('>').unwrap() + aside_idx;
    let aside_open = &html[aside_idx..=aside_end];
    assert!(
        !aside_open.contains("aria-live"),
        "inspector aside should not have aria-live (use #cxpak-live region instead)"
    );
}

#[test]
fn repo_name_is_html_escaped_in_title_and_span() {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 10,
    }];
    let mut content = HashMap::new();
    content.insert("src/main.rs".into(), "fn main() {}".into());
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content);
    let mut meta = fixture_meta();
    meta.repo_name = "<script>alert('xss')</script> & special \"chars\"".into();
    let html = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    // The raw payload must NOT appear unescaped in the HTML output.
    assert!(
        !html.contains("<script>alert('xss')</script>"),
        "raw script tag leaked"
    );
    // Escaped form must appear in the title.
    assert!(
        html.contains(
            "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt; &amp; special &quot;chars&quot;"
        ),
        "escaped repo_name not present in HTML output"
    );
}
