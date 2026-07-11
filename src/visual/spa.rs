//! SPA renderer — composes the three-mode UI (Overview, Explore, History)
//! into one HTML file. Explore merges the former Architecture + Risk views
//! under a lens toggle; Flow and Diff are param-only and live on the
//! standalone render path, not the SPA nav (ADR-0192).

use crate::index::CodebaseIndex;
use crate::visual::layout::{LayoutConfig, LayoutError};
use crate::visual::render::{self, RenderMetadata};
use crate::visual::search_index;

static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js");
static VISUAL_CSS: &str = include_str!("../../assets/cxpak-visual.css");
static SPA_CONTROLLER: &str = include_str!("../../assets/cxpak-spa-controller.js");
/// Client-side palette registry + picker (ADR-0191). Applies CSS custom
/// properties at runtime, so the emitted bytes never change with selection.
static PALETTE_JS: &str = include_str!("../../assets/cxpak-palette.js");

/// Wires the Explore lens toggle: swaps panel visibility and picks the initial
/// lens (Risk by default; Dependencies when a drill-down param is present).
static EXPLORE_LENS_JS: &str = r#"
function _cxSetLens(lens) {
  var isDeps = lens === 'deps';
  var d = document.getElementById('explore-deps');
  var r = document.getElementById('explore-risk');
  if (d) d.hidden = !isDeps;
  if (r) r.hidden = isDeps;
  _exSection.querySelectorAll('.cxpak-lens-btn').forEach(function(b) {
    var on = b.getAttribute('data-lens') === lens;
    b.classList.toggle('active', on);
    b.setAttribute('aria-selected', on ? 'true' : 'false');
  });
  var live = document.getElementById('cxpak-live');
  if (live) live.textContent = isDeps ? 'Dependencies lens' : 'Risk lens';
  // Showing the Risk treemap: re-fit it to the now-visible panel. Its resize
  // handler ignores zero-size (hidden) events, so a window resize that happened
  // while this lens was hidden left the treemap un-fitted — heal it on show.
  if (!isDeps) { try { window.dispatchEvent(new Event('resize')); } catch (e) {} }
}
_exSection.querySelectorAll('.cxpak-lens-btn').forEach(function(b) {
  b.onclick = function() { _cxSetLens(b.getAttribute('data-lens')); };
});
// The route selects the lens: an explicit ?lens=deps or a file/module drill →
// Dependencies; ?lens=risk → Risk; a plain #explore names nothing.
function _cxLensFromRoute() {
  if (CX.state.lens === 'deps' || CX.state.file || CX.state.module) return 'deps';
  if (CX.state.lens === 'risk') return 'risk';
  return null;
}
_cxSetLens(_cxLensFromRoute() || 'risk');
// Explore inits once, so re-apply the lens on RE-navigation — otherwise a later
// #architecture / #risk / palette deep-link would keep the stale lens. Only
// override when the route names one; a plain #explore preserves the user's toggle.
CX.update = CX.update || {};
CX.update['explore'] = function() {
  var want = _cxLensFromRoute();
  if (want) _cxSetLens(want);
};
"#;

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

/// Render the SPA with no embedded timeline (the default, deterministic form).
/// Never reads the live `.cxpak/timeline/` cache — that would make the emitted
/// bytes depend on git history and break the golden fixture. Callers that want
/// a live timeline compute snapshots and pass them to
/// [`render_spa_with_timeline`].
pub fn render_spa(index: &CodebaseIndex, metadata: &RenderMetadata) -> Result<String, LayoutError> {
    render_spa_with_timeline(index, metadata, None)
}

/// Render the SPA, embedding `timeline` snapshots when provided. The snapshots
/// are injected by the caller (CLI/serve) rather than read from disk here, so
/// the fixture path stays byte-deterministic.
pub fn render_spa_with_timeline(
    index: &CodebaseIndex,
    metadata: &RenderMetadata,
    timeline: Option<&[crate::visual::timeline::TimelineSnapshot]>,
) -> Result<String, LayoutError> {
    let cfg = LayoutConfig::default();

    let dashboard_data = render::build_dashboard_data(index);
    // An empty index produces LayoutError::Empty from the module layout step.
    // That is not a bug — it simply means there are no files to visualise.
    // The SPA must still render every view container so the controller can
    // boot cleanly; the Explore graph lens will display an empty graph.
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

    // Timeline: use the caller-injected snapshots; null when none supplied.
    // Each per-view JSON below uses `.expect("...is infallible")` because the
    // serialised types are plain `#[derive(serde::Serialize)]` data structures
    // (see render.rs DashboardData / ArchitectureExplorerData / RiskHeatmap and
    // search_index::SearchIndex).  None contains a custom `Serialize` impl that
    // could fail.  An infallible-fallback `unwrap_or_else(|_| "null".into())`
    // would only mask a corrupted build, not handle a real runtime failure.
    // timeline_js consumes the {steps, current_index, health_sparkline} view-model
    // (TimeMachineData), NOT a raw Vec<TimelineSnapshot> — injecting the snapshots
    // verbatim leaves `tl.steps` undefined and the view falls back to its
    // "insufficient git history" empty state. Build the same view-model the
    // standalone Time Machine renderer uses (render::build_time_machine_data).
    let timeline_json = match timeline {
        Some(snaps) if !snaps.is_empty() => {
            match render::build_time_machine_data(snaps.to_vec(), &cfg) {
                Ok(tm) => {
                    serde_json::to_string(&tm).expect("TimeMachineData serialization is infallible")
                }
                // Empty/degenerate history → null → the view's own empty state.
                Err(_) => "null".into(),
            }
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
    // Three-mode information architecture (ADR-0192): Overview / Explore /
    // History. Flow and Diff were removed from the SPA nav — both are null in
    // every SPA render (they need CLI --symbol / --files params) so they only
    // ever showed a permanent empty state; they remain available via the
    // standalone `cxpak visual --visual-type flow|diff` render path. Internal
    // view keys and hashes stay `dashboard`/`timeline` so deep-links and the
    // shared renderers are unaffected; only the display labels changed.
    html.push_str("      <nav class=\"cxpak-nav\" role=\"tablist\" aria-label=\"Views\">\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"dashboard\" href=\"#dashboard\" role=\"tab\" aria-selected=\"true\" tabindex=\"0\">Overview</a>\n");
    // Explore merges the former Architecture + Risk tabs under one mode with a
    // Dependencies|Risk lens toggle (ADR-0192). Legacy #architecture / #risk
    // hashes redirect here (see the controller's parseHash).
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"explore\" href=\"#explore\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">Explore</a>\n");
    html.push_str("        <a class=\"cxpak-nav-link\" data-view=\"timeline\" href=\"#timeline\" role=\"tab\" aria-selected=\"false\" tabindex=\"-1\">History</a>\n");
    html.push_str("      </nav>\n");
    html.push_str("      <label class=\"cxpak-palette-picker-label\" for=\"cxpak-palette-select\">Palette</label>\n");
    html.push_str("      <div id=\"cxpak-palette-swatches\" class=\"cxpak-palette-swatches\" aria-hidden=\"true\"></div>\n");
    html.push_str("      <select id=\"cxpak-palette-select\" class=\"cxpak-palette-picker\" aria-label=\"Colour palette\"></select>\n");
    html.push_str("      <span class=\"cxpak-freshness\"></span>\n");
    html.push_str("    </header>\n");
    html.push_str("    <noscript>\n");
    html.push_str("      <div style=\"padding:24px 32px;border:1px solid var(--accent-yellow);border-radius:8px;margin:16px;color:var(--text-primary);background:var(--bg-card)\">\n");
    html.push_str("        <strong>JavaScript required.</strong> The cxpak dashboard is a single-page app that renders three interactive views (Overview, Explore, History) entirely in the browser. Without JavaScript the views below remain empty.\n");
    html.push_str("        <br><br>\n");
    html.push_str("        For a JS-free overview of this codebase, run <code>cxpak overview</code> on the command line, which produces a token-budgeted text/markdown report with the same intelligence (PageRank, blast radius, dead code, conventions) backing this dashboard.\n");
    html.push_str("      </div>\n");
    html.push_str("    </noscript>\n");
    html.push_str("    <main id=\"cxpak-main\">\n");
    html.push_str("      <section id=\"view-dashboard\" class=\"cxpak-view\"></section>\n");
    // Explore hosts both lenses. The Risk panel is visible by default so the
    // treemap's clientWidth measurement is non-zero at render time; the init
    // flips to the Dependencies lens when a drill-down param is present.
    html.push_str("      <section id=\"view-explore\" class=\"cxpak-view\" hidden>\n");
    html.push_str(
        "        <div class=\"cxpak-lens-toggle\" role=\"tablist\" aria-label=\"Explore lens\">\n",
    );
    html.push_str("          <button class=\"cxpak-lens-btn\" data-lens=\"deps\" role=\"tab\" aria-selected=\"false\">Dependencies</button>\n");
    html.push_str("          <button class=\"cxpak-lens-btn active\" data-lens=\"risk\" role=\"tab\" aria-selected=\"true\">Risk</button>\n");
    html.push_str("        </div>\n");
    html.push_str("        <div id=\"explore-deps\" class=\"cxpak-lens-panel\" hidden></div>\n");
    html.push_str("        <div id=\"explore-risk\" class=\"cxpak-lens-panel\"></div>\n");
    html.push_str("      </section>\n");
    // History mode (nav label "History"; internal key/hash stay `timeline`).
    html.push_str("      <section id=\"view-timeline\" class=\"cxpak-view\" hidden></section>\n");
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
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>1</kbd>\u{2013}<kbd>3</kbd></span><span class=\"cxpak-inspector-value\">Switch to Overview / Explore / History</span></div>\n");
    html.push_str("        <div class=\"cxpak-inspector-row\"><span class=\"cxpak-inspector-label\"><kbd>p</kbd></span><span class=\"cxpak-inspector-value\">Prove the focused risk score (Overview)</span></div>\n");
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
    // runtime; the emitted bytes are identical for every palette (ADR-0191).
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

    // Flow and Diff are not wired into the SPA (removed from the 3-mode nav —
    // they are param-only, always-null views); their renderers stay live on the
    // standalone `cxpak visual --visual-type flow|diff` path. Their JSON tags are
    // still emitted (null) so the controller's data-tag bootstrap finds them.
    for (key, js) in [
        ("dashboard", render::dashboard_js()),
        ("timeline", render::timeline_js()),
    ] {
        html.push_str(&format!(
            "  <script>\nCX.init['{key}'] = function() {{ _cxpakRunView('{key}', function() {{\n"
        ));
        html.push_str(js);
        html.push_str("\n}); };\n</script>\n");
    }

    // Explore mode: renders both lenses into their panels, reusing the
    // architecture (Dependencies) and risk (Risk) renderers verbatim, each
    // scoped to its own panel via CX.app. A lens toggle swaps visibility
    // (encoding-only; both stay in the DOM). Default lens = Risk, unless a
    // drill-down param (file/module) or ?lens=deps selects Dependencies.
    html.push_str(
        "  <script>\nCX.init['explore'] = function() { _cxpakRunView('explore', function() {\n",
    );
    html.push_str("var _exSection = document.getElementById('view-explore');\n");
    html.push_str("CX.app = document.getElementById('explore-deps');\n(function() {\n");
    html.push_str(render::architecture_js());
    html.push_str("\n})();\n");
    html.push_str("CX.app = document.getElementById('explore-risk');\n(function() {\n");
    html.push_str(render::risk_js());
    html.push_str("\n})();\n");
    html.push_str("CX.app = _exSection;\n");
    html.push_str(EXPLORE_LENS_JS);
    html.push_str("\n}); };\n</script>\n");

    html.push_str("</body>\n");
    html.push_str("</html>\n");

    Ok(html)
}
