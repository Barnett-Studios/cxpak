//! SPA renderer — composes all six views into one HTML file.

use crate::index::CodebaseIndex;
use crate::visual::layout::{LayoutConfig, LayoutError};
use crate::visual::render::{self, RenderMetadata};
use crate::visual::search_index;

static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js");
static VISUAL_CSS: &str = include_str!("../../assets/cxpak-visual.css");
static SPA_CONTROLLER: &str = include_str!("../../assets/cxpak-spa-controller.js");
/// Client-side palette registry + picker (ADR-0172). Applies CSS custom
/// properties at runtime, so the emitted bytes never change with selection.
static PALETTE_JS: &str = include_str!("../../assets/cxpak-palette.js");

/// Delegates to `render::escape_script_tag` — the single canonical
/// implementation — so SPA and standalone renders always produce identical
/// escaping.
fn spa_escape(json: &str) -> String {
    render::escape_script_tag(json)
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
            level2: std::collections::BTreeMap::new(),
            level3: std::collections::BTreeMap::new(),
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
    // Each per-view JSON below uses `.expect("...is infallible")` because the
    // serialised types are plain `#[derive(serde::Serialize)]` data structures
    // (see render.rs DashboardData / ArchitectureExplorerData / RiskHeatmap and
    // search_index::SearchIndex).  None contains a custom `Serialize` impl that
    // could fail.  An infallible-fallback `unwrap_or_else(|_| "null".into())`
    // would only mask a corrupted build, not handle a real runtime failure.
    let timeline_json =
        match crate::visual::timeline::load_cached_snapshots(std::path::Path::new(".")) {
            Some(snaps) if !snaps.is_empty() => {
                serde_json::to_string(&snaps).expect("TimelineSnapshot serialization is infallible")
            }
            _ => "null".into(),
        };

    // Flow and Diff: always null in SPA default (they require params).
    let flow_json = "null".to_string();
    let diff_json = "null".to_string();

    let dashboard_json = spa_escape(
        &serde_json::to_string(&dashboard_data).expect("DashboardData serialization is infallible"),
    );
    let arch_json = spa_escape(
        &serde_json::to_string(&arch_data)
            .expect("ArchitectureExplorerData serialization is infallible"),
    );
    let risk_json = spa_escape(
        &serde_json::to_string(&risk_data).expect("RiskHeatmap serialization is infallible"),
    );
    let timeline_json = spa_escape(&timeline_json);
    let flow_json = spa_escape(&flow_json);
    let diff_json = spa_escape(&diff_json);
    let search_json = spa_escape(
        &serde_json::to_string(&search).expect("SearchIndex serialization is infallible"),
    );
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
    html.push_str("  <link rel=\"icon\" href=\"data:,\">\n");
    html.push_str("  <title>cxpak \u{2014} ");
    html.push_str(&repo);
    html.push_str("</title>\n");
    // Inline script to read saved theme before CSS paints — prevents flash of
    // wrong-theme on first load when user has a stored preference.
    html.push_str("  <script>\n");
    html.push_str("    (function(){\n");
    html.push_str("      try {\n");
    html.push_str("        var t = localStorage.getItem('cxpak-theme');\n");
    html.push_str("        if (t === 'light' || t === 'dark') document.documentElement.setAttribute('data-theme', t);\n");
    html.push_str("        else if (window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches) document.documentElement.setAttribute('data-theme', 'light');\n");
    html.push_str("      } catch (e) { /* ignore */ }\n");
    html.push_str("    })();\n");
    html.push_str("  </script>\n");
    html.push_str("  <style>");
    html.push_str(VISUAL_CSS);
    html.push_str("</style>\n");
    html.push_str("</head>\n");
    html.push_str("<body>\n");
    html.push_str("  <div id=\"cxpak-app\">\n");
    html.push_str("    <header id=\"cxpak-header\">\n");
    html.push_str("      <span class=\"cxpak-logo\">cxpak</span>\n");
    html.push_str("      <span class=\"cxpak-sep\">\u{b7}</span>\n");
    html.push_str("      <span class=\"cxpak-repo\">");
    html.push_str(&repo);
    html.push_str("</span>\n");
    // Roving-tabindex pattern (WAI-ARIA APG): role=tablist on the
    // container, role=tab on each link.  ONE link has tabindex=0, the
    // rest tabindex=-1.  Arrow keys cycle focus between them — the
    // controller handles ArrowLeft/ArrowRight + Home/End and updates
    // tabindex/aria-selected as focus moves.  Without this, keyboard
    // users had to Tab through every preceding focusable element to
    // reach a non-dashboard view.
    html.push_str("      <nav class=\"cxpak-nav\" role=\"tablist\" aria-label=\"Views\">\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"dashboard\" href=\"#dashboard\" role=\"tab\" aria-selected=\"true\" tabindex=\"0\">Dashboard</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"architecture\" href=\"#architecture\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">Architecture</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"risk\" href=\"#risk\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">Risk</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"flow\" href=\"#flow\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">Flow</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"timeline\" href=\"#timeline\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">Timeline</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"diff\" href=\"#diff\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">Diff</a>\n");
    html.push_str("      </nav>\n");
    html.push_str("      <button class=\"cxpak-theme-toggle\" aria-label=\"Switch to light mode\">\u{2600}</button>\n");
    html.push_str("      <label class=\"cxpak-palette-picker-label\" for=\"cxpak-palette-select\">Palette</label>\n");
    html.push_str("      <select id=\"cxpak-palette-select\" class=\"cxpak-palette-picker\" aria-label=\"Colour palette\"></select>\n");
    html.push_str("      <span class=\"cxpak-freshness\"></span>\n");
    html.push_str("    </header>\n");
    html.push_str("    <noscript>\n");
    html.push_str("      <div style=\"padding:24px 32px;border:1px solid var(--accent-yellow);border-radius:8px;margin:16px;color:var(--text-primary);background:var(--bg-card)\">\n");
    html.push_str("        <strong>JavaScript required.</strong> The cxpak dashboard is a single-page app that renders six interactive views (Dashboard, Architecture, Risk, Flow, Timeline, Diff) entirely in the browser. Without JavaScript the views below remain empty.\n");
    html.push_str("        <br><br>\n");
    html.push_str("        For a JS-free overview of this codebase, run <code>cxpak overview</code> on the command line, which produces a token-budgeted text/markdown report with the same intelligence (PageRank, blast radius, dead code, conventions) backing this dashboard.\n");
    html.push_str("      </div>\n");
    html.push_str("    </noscript>\n");
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
    html.push_str("    <aside id=\"cxpak-inspector\" class=\"cxpak-inspector\" role=\"dialog\" aria-modal=\"false\" aria-label=\"Node details inspector\" hidden>\n");
    html.push_str("      <div class=\"cxpak-inspector-header\">\n");
    html.push_str("        <span class=\"cxpak-inspector-title\">Details</span>\n");
    html.push_str("        <button class=\"cxpak-inspector-close\" aria-label=\"Close inspector\">\u{d7}</button>\n");
    html.push_str("      </div>\n");
    html.push_str("      <div class=\"cxpak-inspector-body\"></div>\n");
    html.push_str("    </aside>\n");
    html.push_str("    <div id=\"cxpak-live\" role=\"status\" aria-live=\"polite\" style=\"position:absolute;left:-9999px;\"></div>\n");
    html.push_str("  </div>\n");
    html.push_str("  <div id=\"cxpak-palette-overlay\" class=\"cxpak-palette-overlay\" role=\"dialog\" aria-modal=\"true\" aria-label=\"Command palette\" hidden>\n");
    html.push_str("    <div class=\"cxpak-palette\">\n");
    html.push_str("      <input id=\"cxpak-palette-input\" class=\"cxpak-palette-input\" type=\"text\" placeholder=\"Search files, symbols, views\u{2026}\" autocomplete=\"off\" role=\"combobox\" aria-autocomplete=\"list\" aria-expanded=\"false\" aria-controls=\"cxpak-palette-results\" aria-label=\"Search files, symbols, and views\" />\n");
    html.push_str("      <div id=\"cxpak-palette-results\" class=\"cxpak-palette-results\" role=\"listbox\" aria-label=\"Palette results\"></div>\n");
    html.push_str("      <div class=\"cxpak-palette-hint\">\n");
    html.push_str("        <span><kbd>\u{2191}\u{2193}</kbd> navigate</span>\n");
    html.push_str("        <span><kbd>\u{21b5}</kbd> select</span>\n");
    html.push_str("        <span><kbd>Esc</kbd> close</span>\n");
    html.push_str("      </div>\n");
    html.push_str("    </div>\n");
    html.push_str("  </div>\n");
    html.push_str("  <div id=\"cxpak-help-overlay\" class=\"cxpak-palette-overlay\" role=\"dialog\" aria-modal=\"true\" aria-label=\"Keyboard shortcuts\" hidden>\n");
    html.push_str("    <div class=\"cxpak-palette\">\n");
    html.push_str("      <div class=\"cxpak-inspector-header\">\n");
    html.push_str("        <span class=\"cxpak-inspector-title\">Keyboard shortcuts</span>\n");
    html.push_str("        <button class=\"cxpak-inspector-close\" aria-label=\"Close help\">\u{d7}</button>\n");
    html.push_str("      </div>\n");
    html.push_str("      <div class=\"cxpak-inspector-body\">\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>Cmd/Ctrl+K</kbd> or <kbd>/</kbd></span><span class=\"cxpak-inspector-value\">Open command palette</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>\u{2191}</kbd> <kbd>\u{2193}</kbd></span><span class=\"cxpak-inspector-value\">Navigate palette items</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>Enter</kbd></span><span class=\"cxpak-inspector-value\">Select palette item</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>1</kbd>\u{2013}<kbd>6</kbd></span><span class=\"cxpak-inspector-value\">Switch to Dashboard / Architecture / Risk / Flow / Timeline / Diff</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>t</kbd></span><span class=\"cxpak-inspector-value\">Toggle dark / light theme</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>?</kbd></span><span class=\"cxpak-inspector-value\">This help overlay</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>Esc</kbd></span><span class=\"cxpak-inspector-value\">Close palette / inspector / help overlay</span></div>\n");
    html.push_str("      </div>\n");
    html.push_str("    </div>\n");
    html.push_str("  </div>\n\n");

    // JSON data blobs — tag IDs match the names each per-view renderer in render.rs
    // expects (dashboard_js → `cxpak-dashboard`, architecture_js → `cxpak-explorer`,
    // risk_js → `cxpak-heatmap`, etc.) so the shared renderers work unchanged.
    html.push_str("  <script id=\"cxpak-dashboard\" type=\"application/json\">");
    html.push_str(&dashboard_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-explorer\" type=\"application/json\">");
    html.push_str(&arch_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-heatmap\" type=\"application/json\">");
    html.push_str(&risk_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-timeline\" type=\"application/json\">");
    html.push_str(&timeline_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-flow\" type=\"application/json\">");
    html.push_str(&flow_json);
    html.push_str("</script>\n");

    html.push_str("  <script id=\"cxpak-diff\" type=\"application/json\">");
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

    // Shared renderer helpers (CX.svgCanvas, CX.tooltip, graph renderer, etc.).
    // common_js is defensive about missing cxpak-data/cxpak-meta tags.
    html.push_str("  <script>");
    html.push_str(render::common_js());
    // SPA provides its own header; suppress the one common_js offers.
    html.push_str("\nCX.header = function() {};\n");
    html.push_str("</script>\n");

    // SPA controller (router, palette, inspector, theme, keyboard, freshness).
    // Runs AFTER common_js so `var CX = window.CX || {}` inherits layout/meta/app
    // helpers defined by common_js.
    html.push_str("  <script>");
    html.push_str(SPA_CONTROLLER);
    html.push_str("</script>\n");

    // Palette registry + picker — runs after the header exists and after the
    // controller so window.CX is present. Applies CSS custom properties at
    // runtime; the emitted bytes are identical for every palette (ADR-0172).
    html.push_str("  <script>");
    html.push_str(PALETTE_JS);
    html.push_str("</script>\n");

    // Per-view renderers, each wrapped in a deferred CX.init.{view} function so it
    // only runs when the router navigates to that view. Before running, CX.app is
    // repointed to the view's section so the renderer's appendChild calls land in
    // the correct container. Idempotency is ensured by CX._initialized[view].
    html.push_str("  <script>\n");
    html.push_str("CX.init = CX.init || {};\n");
    html.push_str("CX._initialized = {};\n");
    html.push_str("function _cxpakRunView(viewName, rendererCode) {\n");
    html.push_str("  if (CX._initialized[viewName]) return;\n");
    html.push_str("  CX._initialized[viewName] = true;\n");
    html.push_str("  var section = document.getElementById('view-' + viewName);\n");
    html.push_str("  if (!section) return;\n");
    html.push_str("  CX.app = section;\n");
    // Views whose primary data blob is null require CLI parameters; show an
    // explanatory empty state rather than silently failing in the renderer.
    html.push_str(
        "  var DATA_KEY = { flow: 'flow', timeline: 'timeline', diff: 'diff' }[viewName];\n",
    );
    html.push_str("  var HINT = {\n");
    html.push_str("    flow: 'Flow view requires a symbol. Run: cxpak visual --visual-type flow --symbol <name>',\n");
    html.push_str("    timeline: 'Timeline view requires cached git snapshots. Run: cxpak visual --visual-type timeline',\n");
    html.push_str("    diff: 'Diff view requires two git refs. Run: cxpak visual --visual-type diff --files <files>',\n");
    html.push_str("  }[viewName];\n");
    html.push_str("  if (DATA_KEY && CX.data[DATA_KEY] === null) {\n");
    html.push_str("    var msg = document.createElement('div');\n");
    html.push_str("    msg.className = 'cxpak-empty-state';\n");
    html.push_str("    msg.textContent = HINT;\n");
    html.push_str("    section.appendChild(msg);\n");
    html.push_str("    return;\n");
    html.push_str("  }\n");
    html.push_str("  try { rendererCode(); } catch (e) { console.error('view ' + viewName + ' render failed', e); }\n");
    html.push_str("}\n");
    html.push_str("</script>\n");

    for (key, js) in [
        ("dashboard", render::dashboard_js()),
        ("architecture", render::architecture_js()),
        ("risk", render::risk_js()),
        ("flow", render::flow_js()),
        ("timeline", render::timeline_js()),
        ("diff", render::diff_js()),
    ] {
        html.push_str(&format!(
            "  <script>\nCX.init['{key}'] = function() {{ _cxpakRunView('{key}', function() {{\n"
        ));
        html.push_str(js);
        html.push_str("\n}); };\n</script>\n");
    }

    html.push_str("</body>\n");
    html.push_str("</html>\n");

    Ok(html)
}
