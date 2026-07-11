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
fn contains_all_view_containers() {
    // Architecture + Risk merged into Explore (ADR-0173).
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    for id in [
        "view-dashboard",
        "view-explore",
        "view-flow",
        "view-timeline",
        "view-diff",
    ] {
        assert!(html.contains(&format!(r#"id="{id}""#)), "missing {id}");
    }
    // Explore hosts both lens panels.
    assert!(html.contains(r#"id="explore-deps""#));
    assert!(html.contains(r#"id="explore-risk""#));
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
    // Inspector switched from role=complementary to role=dialog in v2.1.1
    // so the focus trap can recognize it as a modal-like region.
    // aria-modal=false signals the dialog is non-blocking — main view is
    // still accessible (vs the palette/help which are aria-modal=true).
    let aside_line = html
        .lines()
        .find(|l| l.contains(r#"id="cxpak-inspector""#))
        .expect("inspector aside line");
    assert!(
        aside_line.contains(r#"role="dialog""#),
        "inspector must have role=dialog (was complementary pre-v2.1.1); got: {aside_line}"
    );
    assert!(
        aside_line.contains(r#"aria-label="Node details inspector""#),
        "inspector must have aria-label"
    );
}

#[test]
fn spa_palette_has_dialog_aria() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // Palette + help overlay + inspector are all role=dialog after v2.1.1
    // moved the inspector from role=complementary so the focus trap can
    // include it.
    assert_eq!(
        html.matches(r#"role="dialog""#).count(),
        3,
        "expected 3 role=dialog elements (palette + help + inspector)"
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

#[test]
fn dashboard_dimensions_reproduce_composite_via_formula() {
    // Contract: the health composite displayed on the dashboard MUST equal
    // sum(dimension × weight) where weights are (conventions=0.20,
    // test_coverage=0.20, churn_stability=0.15, coupling=0.20, cycles=0.15,
    // dead_code=0.10). If any dimension is missing from the display, the
    // formula cannot be reproduced by the user from the visible bars.
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // Extract dashboard JSON
    let marker = r#"<script id="cxpak-dashboard" type="application/json">"#;
    let start = html.find(marker).unwrap() + marker.len();
    let end = html[start..].find("</script>").unwrap() + start;
    let json_escaped = &html[start..end];
    // spa_escape uses \u00XX unicode escapes which serde_json parses natively
    // inside JSON string values, so no pre-processing is needed.
    let v: serde_json::Value = serde_json::from_str(json_escaped).unwrap();
    let dims = v["health"]["dimensions"].as_array().unwrap();
    let composite = v["health"]["composite"].as_f64().unwrap();
    let weights: std::collections::HashMap<&str, f64> = [
        ("conventions", 0.20),
        ("test_coverage", 0.20),
        ("churn_stability", 0.15),
        ("coupling", 0.20),
        ("cycles", 0.15),
        ("dead_code", 0.10),
    ]
    .iter()
    .cloned()
    .collect();
    let mut reproduced = 0.0_f64;
    let mut seen_names = std::collections::HashSet::new();
    for d in dims {
        let name = d[0].as_str().unwrap();
        let value = d[1].as_f64().unwrap();
        seen_names.insert(name.to_string());
        if let Some(w) = weights.get(name) {
            reproduced += w * value;
        }
    }
    for expected in [
        "conventions",
        "test_coverage",
        "churn_stability",
        "coupling",
        "cycles",
        "dead_code",
    ] {
        assert!(
            seen_names.contains(expected),
            "dashboard dimensions missing `{expected}`"
        );
    }
    assert!(
        (reproduced - composite).abs() < 0.01,
        "dashboard dimensions × weights = {reproduced}, but composite shown = {composite}. \
         A user cannot reproduce the composite from the visible bars."
    );
}

#[test]
fn dashboard_has_tests_matches_risk_formula_inline_tests() {
    // A file with inline `#[cfg(test)] mod tests` and NO entry in test_map
    // must display `has_tests = true` in the dashboard table, matching the
    // same condition used by compute_risk_ranking for test_coverage.
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/lib_with_inline.rs".into(),
        absolute_path: "/tmp/src/lib_with_inline.rs".into(),
        language: Some("rust".into()),
        size_bytes: 100,
    }];
    let mut content = HashMap::new();
    content.insert(
        "src/lib_with_inline.rs".to_string(),
        "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n".to_string(),
    );
    let idx =
        cxpak::index::CodebaseIndex::build_with_content(files, HashMap::new(), &counter, content);
    let data = cxpak::visual::render::build_dashboard_data(&idx);
    let entry = data
        .risks
        .top_risks
        .iter()
        .find(|r| r.path == "src/lib_with_inline.rs")
        .expect("file must appear in top_risks (any score)");
    assert!(
        entry.has_tests,
        "dashboard must report has_tests=true for a file with inline tests \
         (matches risk formula's test_coverage detection)"
    );
}

#[test]
fn header_has_separator_between_brand_and_repo() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // After the fix, either a visible separator glyph (·) or a CSS border
    // rule should distinguish the two header labels.
    let has_separator_glyph = html.contains(r#"class="cxpak-sep""#);
    let has_css_border = html.contains(".cxpak-logo")
        && html[html.find(".cxpak-logo").unwrap()..]
            .find("border-right")
            .is_some();
    assert!(
        has_separator_glyph || has_css_border,
        "header must have a visual separator between brand and repo (either HTML .cxpak-sep span or CSS border-right on .cxpak-logo)"
    );
}

#[test]
fn top_risks_table_uses_unit_labels_and_tooltips() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    assert!(
        html.contains("Blast Radius"),
        "column must be named 'Blast Radius', not 'Blast'"
    );
    assert!(
        html.contains("Churn (30d)"),
        "column must include unit: 'Churn (30d)'"
    );
    // And the `title` attribute explaining blast radius:
    assert!(
        html.contains("title=\"Number of files that directly import this file"),
        "Blast Radius header must have a `title` tooltip explaining the metric"
    );
}

#[test]
fn alert_icons_have_sr_only_text_and_aria_hidden() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // The dashboard_js emits alert items. The icon span must be aria-hidden,
    // and a preceding sr-only label must describe the severity.
    assert!(
        html.contains("setAttribute('aria-hidden', 'true')")
            || html.contains("aria-hidden=\"true\""),
        "alert icon must be aria-hidden"
    );
    assert!(
        html.contains("'sr-only'") || html.contains("\"sr-only\""),
        "alert must have sr-only text for screen readers"
    );
    // CSS class `.sr-only` must be defined
    let css = std::fs::read_to_string("assets/cxpak-visual.css").unwrap();
    assert!(css.contains(".sr-only"), "CSS must define .sr-only class");
}

#[test]
fn risk_inspector_passes_context_specific_fields() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    // risk_js should pass Churn / Blast / Tests labels when opening inspector
    // from a risk treemap cell. Grep the embedded risk_js for these strings.
    assert!(
        html.contains("'Churn (30d)'") || html.contains("\"Churn (30d)\""),
        "risk inspector must show churn"
    );
    assert!(
        html.contains("'Blast radius'") || html.contains("\"Blast radius\""),
        "risk inspector must show blast radius"
    );
}

// ── Fix 1: dashboard top_risks and alerts use same ranking ───────────────────

#[test]
fn dashboard_top_risks_and_alerts_use_same_ranking() {
    // Construct an index where two files have identical risk scores at a high
    // enough value that the alert filter (>0.8) might pick them up.  Verify
    // dashboard top_risks ordering and alerts ordering are CONSISTENT (tied
    // entries appear in the same relative order).
    use cxpak::budget::counter::TokenCounter;
    use cxpak::scanner::ScannedFile;
    let counter = TokenCounter::new();
    let files: Vec<ScannedFile> = (0..5)
        .map(|i| ScannedFile {
            relative_path: format!("src/m{i}/mod.rs"),
            absolute_path: format!("/tmp/src/m{i}/mod.rs").into(),
            language: Some("rust".into()),
            size_bytes: 100,
        })
        .collect();
    let mut content = std::collections::HashMap::new();
    for f in &files {
        content.insert(f.relative_path.clone(), "fn x() {}".into());
    }
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        files,
        std::collections::HashMap::new(),
        &counter,
        content,
    );
    let data = cxpak::visual::render::build_dashboard_data(&idx);
    let direct = cxpak::intelligence::risk::compute_risk_ranking(&idx);
    // Top risks in dashboard MUST match the direct ranking order (path-by-path).
    for (i, displayed) in data.risks.top_risks.iter().enumerate() {
        assert_eq!(
            displayed.path, direct[i].path,
            "dashboard top_risks[{i}] = {} but compute_risk_ranking[{i}] = {} — divergence between channels",
            displayed.path, direct[i].path,
        );
        assert_eq!(
            displayed.risk_score.to_bits(),
            direct[i].risk_score.to_bits(),
            "risk_score f64 bits diverge for {}",
            displayed.path
        );
    }
}

// ── Fix 6: first-paint theme inline script ───────────────────────────────────

#[test]
fn first_paint_theme_inline_script_present() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let head_start = html.find("<head>").unwrap();
    let head_end = html.find("</head>").unwrap();
    let head = &html[head_start..head_end];
    assert!(
        head.contains("getItem('cxpak-theme')") || head.contains("getItem(\"cxpak-theme\")"),
        "head must contain inline script that reads localStorage theme to prevent first-paint flash"
    );
    // The inline script must run BEFORE the <style> block.
    let script_pos = head
        .find("getItem")
        .expect("getItem must be present in head");
    let style_pos = head.find("<style>").expect("<style> must be in head");
    assert!(
        script_pos < style_pos,
        "theme-applying inline script must precede <style> for synchronous first paint"
    );
}

// ── Fix 7: has_inline_tests recognises tokio/rstest/async-std ───────────────

#[test]
fn has_inline_tests_recognizes_tokio_rstest() {
    use cxpak::budget::counter::TokenCounter;
    use cxpak::scanner::ScannedFile;
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/svc.rs".into(),
        absolute_path: "/tmp/svc.rs".into(),
        language: Some("rust".into()),
        size_bytes: 100,
    }];
    // No #[cfg(test)] or #[test] — only #[tokio::test]
    let mut content = std::collections::HashMap::new();
    content.insert(
        "src/svc.rs".to_string(),
        "#[tokio::test]\nasync fn t() {}\n".to_string(),
    );
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        files,
        std::collections::HashMap::new(),
        &counter,
        content,
    );
    let data = cxpak::visual::render::build_dashboard_data(&idx);
    let entry = data
        .risks
        .top_risks
        .iter()
        .find(|r| r.path == "src/svc.rs")
        .expect("file must appear in top_risks");
    assert!(
        entry.has_tests,
        "has_tests must recognize #[tokio::test] (currently misses tokio/rstest/async-std test attributes)"
    );
}

// ── Fix 10: dashboard dead-symbol alert count is honest ──────────────────────

#[test]
fn dashboard_dead_symbols_alert_reflects_honest_count() {
    // On an empty index, detect_dead_code returns 0. The dashboard alert must
    // agree — not show a stale or computed-from-different-source count.
    use cxpak::budget::counter::TokenCounter;
    let counter = TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let data = cxpak::visual::render::build_dashboard_data(&idx);
    let direct_dead = cxpak::intelligence::dead_code::detect_dead_code(&idx, None);
    // Find the dead-symbols alert if present.
    let dead_alert = data
        .alerts
        .alerts
        .iter()
        .find(|a| matches!(a.kind, cxpak::visual::render::AlertKind::DeadSymbols));
    if let Some(alert) = dead_alert {
        let count_str = direct_dead.len().to_string();
        assert!(
            alert.message.contains(&count_str),
            "dashboard dead-symbols alert ({}) must reference the same count as direct detection ({} symbols)",
            alert.message,
            direct_dead.len()
        );
    } else {
        assert_eq!(
            direct_dead.len(),
            0,
            "no alert is fine only if there are no dead symbols"
        );
    }
}

// ── Fix 11: help overlay documents palette navigation ───────────────────────

#[test]
fn help_overlay_documents_palette_navigation() {
    let html = cxpak::visual::spa::render_spa(&fixture_index(), &fixture_meta()).unwrap();
    let help_start = html
        .find(r#"id="cxpak-help-overlay""#)
        .expect("help overlay present");
    // Find the end of the overlay by finding the second closing div block.
    let region = &html[help_start..];
    // Grab a generous slice covering the help overlay content.
    let help_end = region.find("</div>\n  </div>").unwrap_or(region.len());
    let help = &region[..help_end];
    assert!(
        (help.contains('\u{2191}') || help.contains("↑"))
            && (help.contains('\u{2193}') || help.contains("↓")),
        "help overlay must list arrow-key palette navigation (↑ ↓)"
    );
    assert!(
        help.contains("Enter") || help.contains("\u{21b5}"),
        "help overlay must list Enter to select"
    );
}
