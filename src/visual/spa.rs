//! SPA renderer — composes all six views into one HTML file.

use crate::index::CodebaseIndex;
use crate::visual::layout::{LayoutConfig, LayoutError};
use crate::visual::render::{self, RenderMetadata};
use crate::visual::search_index;

static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js");
static VISUAL_CSS: &str = include_str!("../../assets/cxpak-visual.css");
static SPA_CONTROLLER: &str = include_str!("../../assets/cxpak-spa-controller.js");

/// Escapes JSON for safe embedding inside an HTML `<script type="application/json">` block.
///
/// Replaces every `<`, `>`, `&`, and `=` with the corresponding JSON `\u00XX` Unicode escape.
/// All five chars (`<`, `>`, `&`, `=`, plus `</script>` substring elimination) are covered:
/// the per-character `<` and `>` escapes alone make `</script>` impossible to form, so the
/// `escape_script_tag` substring rewrites are no longer needed. Unicode escapes are valid
/// JSON per RFC 8259 §7 and decoded transparently by all JSON parsers.
fn spa_escape(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + (json.len() / 16));
    for ch in json.chars() {
        match ch {
            '<' => out.push_str(r"\u003c"),
            '>' => out.push_str(r"\u003e"),
            '&' => out.push_str(r"\u0026"),
            '=' => out.push_str(r"\u003d"),
            other => out.push(other),
        }
    }
    out
}

/// Escapes a string for safe inclusion in HTML element text content.
/// Replaces `&`, `<`, `>`, `"`, `'` with their named or numeric entities.
fn escape_html_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn render_spa(index: &CodebaseIndex, metadata: &RenderMetadata) -> Result<String, LayoutError> {
    let cfg = LayoutConfig::default();

    let dashboard_data = render::build_dashboard_data(index);
    // An empty index produces LayoutError::Empty from the module layout step.
    // That is not a bug — it simply means there are no files to visualise.
    // The SPA must still render all six view containers so the controller can
    // boot cleanly; the architecture view will display an empty graph.
    let arch_data = match render::build_architecture_explorer_data(index, &cfg) {
        Ok(d) => d,
        Err(LayoutError::Empty) => render::ArchitectureExplorerData {
            level1: crate::visual::layout::ComputedLayout {
                nodes: vec![],
                edges: vec![],
                width: 0.0,
                height: 0.0,
                layers: vec![],
            },
            level2: std::collections::HashMap::new(),
            level3: std::collections::HashMap::new(),
            initial_level: 1,
            breadcrumbs: vec![render::BreadcrumbEntry {
                label: "Repository".to_string(),
                level: 1,
                target_id: "root".to_string(),
            }],
        },
        Err(e) => return Err(e),
    };
    let risk_data = render::build_risk_heatmap_data(index);
    let search = search_index::build_search_index(index);

    // Timeline: attempt to load cached snapshots; null when absent.
    let timeline_json =
        match crate::visual::timeline::load_cached_snapshots(std::path::Path::new(".")) {
            Some(snaps) if !snaps.is_empty() => {
                serde_json::to_string(&snaps).unwrap_or_else(|_| "null".into())
            }
            _ => "null".into(),
        };

    // Flow and Diff: always null in SPA default (they require params).
    let flow_json = "null".to_string();
    let diff_json = "null".to_string();

    let dashboard_json =
        spa_escape(&serde_json::to_string(&dashboard_data).unwrap_or_else(|_| "null".into()));
    let arch_json =
        spa_escape(&serde_json::to_string(&arch_data).unwrap_or_else(|_| "null".into()));
    let risk_json =
        spa_escape(&serde_json::to_string(&risk_data).unwrap_or_else(|_| "null".into()));
    let timeline_json = spa_escape(&timeline_json);
    let flow_json = spa_escape(&flow_json);
    let diff_json = spa_escape(&diff_json);
    let search_json = spa_escape(&serde_json::to_string(&search).unwrap_or_else(|_| "[]".into()));
    let meta_json = spa_escape(
        &serde_json::to_string(metadata).expect("RenderMetadata serialization is infallible"),
    );
    let repo = escape_html_text(&metadata.repo_name);

    // Build the HTML via string concatenation to avoid format! choking on CSS/JS
    // brace characters. The JSON data blobs and asset files are appended directly.
    let mut html = String::with_capacity(512 * 1024);

    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html lang=\"en\" data-theme=\"dark\">\n");
    html.push_str("<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <title>cxpak \u{2014} ");
    html.push_str(&repo);
    html.push_str("</title>\n");
    html.push_str("  <style>");
    html.push_str(VISUAL_CSS);
    html.push_str("</style>\n");
    html.push_str("</head>\n");
    html.push_str("<body>\n");
    html.push_str("  <div id=\"cxpak-app\">\n");
    html.push_str("    <header id=\"cxpak-header\">\n");
    html.push_str("      <span class=\"cxpak-logo\">cxpak</span>\n");
    html.push_str("      <span class=\"cxpak-repo\">");
    html.push_str(&repo);
    html.push_str("</span>\n");
    html.push_str("      <nav class=\"cxpak-nav\">\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"dashboard\" href=\"#dashboard\" tabindex=\"0\">Dashboard</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"architecture\" href=\"#architecture\">Architecture</a>\n");
    html.push_str(
        "        <a class=\"cxpak-nav-link\" data-view=\"risk\" href=\"#risk\">Risk</a>\n",
    );
    html.push_str(
        "        <a class=\"cxpak-nav-link\" data-view=\"flow\" href=\"#flow\">Flow</a>\n",
    );
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"timeline\" href=\"#timeline\">Timeline</a>\n");
    html.push_str(
        "        <a class=\"cxpak-nav-link\" data-view=\"diff\" href=\"#diff\">Diff</a>\n",
    );
    html.push_str("      </nav>\n");
    html.push_str("      <button class=\"cxpak-theme-toggle\" aria-label=\"Switch to light mode\">\u{263a}</button>\n");
    html.push_str("      <span class=\"cxpak-freshness\"></span>\n");
    html.push_str("    </header>\n");
    html.push_str("    <main id=\"cxpak-main\">\n");
    html.push_str("      <section id=\"view-dashboard\" class=\"cxpak-view\"></section>\n");
    html.push_str(
        "      <section id=\"view-architecture\" class=\"cxpak-view\" hidden></section>\n",
    );
    html.push_str("      <section id=\"view-risk\" class=\"cxpak-view\" hidden></section>\n");
    html.push_str("      <section id=\"view-flow\" class=\"cxpak-view\" hidden></section>\n");
    html.push_str("      <section id=\"view-timeline\" class=\"cxpak-view\" hidden></section>\n");
    html.push_str("      <section id=\"view-diff\" class=\"cxpak-view\" hidden></section>\n");
    html.push_str("    </main>\n");
    html.push_str("    <aside id=\"cxpak-inspector\" class=\"cxpak-inspector\" hidden>\n");
    html.push_str("      <div class=\"cxpak-inspector-header\">\n");
    html.push_str("        <span class=\"cxpak-inspector-title\">Details</span>\n");
    html.push_str("        <button class=\"cxpak-inspector-close\" aria-label=\"Close inspector\">\u{d7}</button>\n");
    html.push_str("      </div>\n");
    html.push_str("      <div class=\"cxpak-inspector-body\"></div>\n");
    html.push_str("    </aside>\n");
    html.push_str("    <div id=\"cxpak-live\" role=\"status\" aria-live=\"polite\" style=\"position:absolute;left:-9999px;\"></div>\n");
    html.push_str("  </div>\n");
    html.push_str("  <div id=\"cxpak-palette-overlay\" class=\"cxpak-palette-overlay\" hidden>\n");
    html.push_str("    <div class=\"cxpak-palette\">\n");
    html.push_str("      <input id=\"cxpak-palette-input\" class=\"cxpak-palette-input\" type=\"text\" placeholder=\"Search files, symbols, views\u{2026}\" autocomplete=\"off\" />\n");
    html.push_str("      <div id=\"cxpak-palette-results\" class=\"cxpak-palette-results\" role=\"listbox\"></div>\n");
    html.push_str("      <div class=\"cxpak-palette-hint\">\n");
    html.push_str("        <span><kbd>\u{2191}\u{2193}</kbd> navigate</span>\n");
    html.push_str("        <span><kbd>\u{21b5}</kbd> select</span>\n");
    html.push_str("        <span><kbd>Esc</kbd> close</span>\n");
    html.push_str("      </div>\n");
    html.push_str("    </div>\n");
    html.push_str("  </div>\n");
    html.push_str("  <div id=\"cxpak-help-overlay\" hidden></div>\n\n");

    // JSON data blobs — each escaped to prevent </script> injection.
    html.push_str("  <script id=\"cxpak-dashboard-data\" type=\"application/json\">");
    html.push_str(&dashboard_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-architecture-data\" type=\"application/json\">");
    html.push_str(&arch_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-risk-data\" type=\"application/json\">");
    html.push_str(&risk_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-timeline-data\" type=\"application/json\">");
    html.push_str(&timeline_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-flow-data\" type=\"application/json\">");
    html.push_str(&flow_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-diff-data\" type=\"application/json\">");
    html.push_str(&diff_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-meta\" type=\"application/json\">");
    html.push_str(&meta_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-search-index\" type=\"application/json\">");
    html.push_str(&search_json);
    html.push_str("</script>\n\n");

    // Asset bundles inlined — no CDN references.
    html.push_str("  <script>");
    html.push_str(D3_BUNDLE);
    html.push_str("</script>\n");

    html.push_str("  <script>");
    html.push_str(SPA_CONTROLLER);
    html.push_str("</script>\n");

    html.push_str("</body>\n");
    html.push_str("</html>\n");

    Ok(html)
}
