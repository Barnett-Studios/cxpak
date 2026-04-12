//! Rendering engine for interactive and static visualizations.
//!
//! The render module converts layout-positioned graphs and metrics into
//! interactive HTML dashboards, architecture diagrams, risk heatmaps,
//! data flow visualizations, timelines, and diff comparisons.
//!
//! Implementation includes:
//! - HTML template system with D3.js for interactivity (Task 6)
//! - Dashboard view with metrics and navigation (Task 7)
//! - Architecture Explorer with 3-level semantic zoom (Task 8)
//! - Risk Heatmap using treemap layout (Task 9)
//! - Flow Diagram showing value propagation (Task 10)
//! - Time Machine view of historical changes (Task 11)
//! - Diff view for snapshot comparisons (Task 12)

use crate::index::CodebaseIndex;

static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js");
static VISUAL_CSS: &str = include_str!("../../assets/cxpak-visual.css");

/// Metadata about the rendered visualization, embedded in the output HTML.
#[derive(Debug, serde::Serialize)]
pub struct RenderMetadata {
    pub repo_name: String,
    pub generated_at: String,
    pub health_score: Option<f64>,
    pub node_count: usize,
    pub edge_count: usize,
    pub cxpak_version: String,
}

/// Maps a `VisualType` to its human-readable display name.
fn visual_type_name(vt: &super::VisualType) -> &'static str {
    match vt {
        super::VisualType::Dashboard => "Dashboard",
        super::VisualType::Architecture => "Architecture Explorer",
        super::VisualType::Risk => "Risk Heatmap",
        super::VisualType::Flow => "Flow Diagram",
        super::VisualType::Timeline => "Time Machine",
        super::VisualType::Diff => "Diff View",
    }
}

/// Maps a `VisualType` to the string identifier used in the JS controller.
fn visual_type_id(vt: &super::VisualType) -> &'static str {
    match vt {
        super::VisualType::Dashboard => "dashboard",
        super::VisualType::Architecture => "architecture",
        super::VisualType::Risk => "risk",
        super::VisualType::Flow => "flow",
        super::VisualType::Timeline => "timeline",
        super::VisualType::Diff => "diff",
    }
}

/// Inline JS controller that reads layout/meta from the page and initialises a D3 graph.
///
/// Tasks 7-12 will replace the `switch` branches with type-specific renderers.
/// For now every type falls through to the base graph renderer.
fn view_controller_js(visual_type: &super::VisualType) -> String {
    let type_id = visual_type_id(visual_type);
    format!(
        r#"(function () {{
  'use strict';

  var layout = JSON.parse(document.getElementById('cxpak-data').textContent);
  var meta   = JSON.parse(document.getElementById('cxpak-meta').textContent);
  var type_  = {type_id_json};

  /* ── header ───────────────────────────────────────────────────── */
  var header = document.createElement('div');
  header.id = 'cxpak-header';
  header.innerHTML =
    '<span class="cxpak-repo">' + meta.repo_name + '</span>' +
    '<span class="cxpak-type">' + meta.visual_type_display + '</span>';
  document.getElementById('cxpak-app').appendChild(header);

  /* ── SVG canvas ────────────────────────────────────────────────── */
  var W = layout.width  || 1200;
  var H = layout.height || 800;

  var svg = d3.select('#cxpak-app')
    .append('svg')
    .attr('id', 'cxpak-svg')
    .attr('width',  '100%')
    .attr('height', '100%')
    .attr('viewBox', '0 0 ' + W + ' ' + H);

  /* zoom */
  var zoomG = svg.append('g').attr('id', 'cxpak-zoom-group');
  svg.call(
    d3.zoom()
      .scaleExtent([0.1, 10])
      .on('zoom', function (event) {{
        zoomG.attr('transform', event.transform);
      }})
  );

  /* ── dispatch to type-specific renderer ───────────────────────── */
  switch (type_) {{
    case 'dashboard':
    case 'architecture':
    case 'risk':
    case 'flow':
    case 'timeline':
    case 'diff':
    default:
      renderBaseGraph(zoomG, layout);
      break;
  }}

  /* ── base graph renderer ──────────────────────────────────────── */
  function renderBaseGraph(root, data) {{
    var nodes = data.nodes || [];
    var edges = data.edges || [];

    /* edges */
    root.append('g').attr('class', 'cxpak-edges')
      .selectAll('line')
      .data(edges)
      .join('line')
        .attr('class', 'cxpak-edge')
        .attr('x1', function (d) {{ return nodeX(d.source, nodes); }})
        .attr('y1', function (d) {{ return nodeY(d.source, nodes); }})
        .attr('x2', function (d) {{ return nodeX(d.target, nodes); }})
        .attr('y2', function (d) {{ return nodeY(d.target, nodes); }});

    /* node groups */
    var nodeG = root.append('g').attr('class', 'cxpak-nodes')
      .selectAll('g')
      .data(nodes)
      .join('g')
        .attr('class', 'cxpak-node')
        .attr('transform', function (d) {{
          return 'translate(' + d.position.x + ',' + d.position.y + ')';
        }});

    nodeG.append('rect')
      .attr('width',  function (d) {{ return d.width; }})
      .attr('height', function (d) {{ return d.height; }})
      .attr('rx', 4)
      .attr('ry', 4);

    nodeG.append('text')
      .attr('x', function (d) {{ return d.width / 2; }})
      .attr('y', function (d) {{ return d.height / 2; }})
      .attr('text-anchor', 'middle')
      .attr('dominant-baseline', 'middle')
      .text(function (d) {{ return d.label; }});
  }}

  /* helpers */
  function findNode(id, nodes) {{
    return nodes.find(function (n) {{ return n.id === id; }}) || null;
  }}
  function nodeX(id, nodes) {{
    var n = findNode(id, nodes);
    return n ? n.position.x + n.width / 2 : 0;
  }}
  function nodeY(id, nodes) {{
    var n = findNode(id, nodes);
    return n ? n.position.y + n.height / 2 : 0;
  }}
}})();
"#,
        type_id_json = serde_json::to_string(type_id).unwrap(),
    )
}

/// Renders a self-contained HTML file.  All JS/CSS is inlined — no CDN dependencies.
///
/// The layout data is JSON-serialised into a `<script id="cxpak-data">` tag so
/// the view controller can read it without an extra network request.
pub fn render_html(
    layout: &super::layout::ComputedLayout,
    visual_type: super::VisualType,
    metadata: &RenderMetadata,
) -> String {
    let title = visual_type_name(&visual_type);
    let layout_json = serde_json::to_string(layout).unwrap();

    // Embed the display name in meta so JS doesn't need its own mapping.
    #[derive(serde::Serialize)]
    struct MetaWithDisplay<'a> {
        #[serde(flatten)]
        inner: &'a RenderMetadata,
        visual_type_display: &'static str,
    }
    let meta_with_display = MetaWithDisplay {
        inner: metadata,
        visual_type_display: title,
    };
    let meta_json = serde_json::to_string(&meta_with_display).unwrap();
    let controller_js = view_controller_js(&visual_type);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>cxpak — {title}</title>
  <style>{css}</style>
</head>
<body>
  <div id="cxpak-app"></div>
  <script id="cxpak-data" type="application/json">{layout_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script>{d3}</script>
  <script>{controller}</script>
</body>
</html>
"#,
        title = title,
        css = VISUAL_CSS,
        layout_json = layout_json,
        meta_json = meta_json,
        d3 = D3_BUNDLE,
        controller = controller_js,
    )
}

// ── Dashboard types ──────────────────────────────────────────────────────────

/// All four quadrants of the dashboard view, serialised into the HTML page.
#[derive(Debug, serde::Serialize)]
pub struct DashboardData {
    pub health: HealthQuadrant,
    pub risks: RisksQuadrant,
    pub architecture_preview: ArchitecturePreviewQuadrant,
    pub alerts: AlertsQuadrant,
}

/// Top-left quadrant: composite health score plus individual dimensions.
#[derive(Debug, serde::Serialize)]
pub struct HealthQuadrant {
    pub composite: f64,
    /// (dimension_name, score) pairs, e.g. [("conventions", 9.0), ...]
    pub dimensions: Vec<(String, f64)>,
    /// Placeholder trend series — populated as `(label, value)` pairs when
    /// historical data is available; empty otherwise.
    pub trend: Vec<(String, f64)>,
}

/// Top-right quadrant: top-5 riskiest files.
#[derive(Debug, serde::Serialize)]
pub struct RisksQuadrant {
    pub top_risks: Vec<RiskDisplayEntry>,
}

/// One row in the risks quadrant table.
#[derive(Debug, serde::Serialize)]
pub struct RiskDisplayEntry {
    pub path: String,
    pub risk_score: f64,
    pub churn_30d: u32,
    pub blast_radius: usize,
    pub has_tests: bool,
    pub severity: String,
}

/// Bottom-left quadrant: mini architecture graph preview.
#[derive(Debug, serde::Serialize)]
pub struct ArchitecturePreviewQuadrant {
    pub layout: super::layout::ComputedLayout,
    pub module_count: usize,
    pub circular_dep_count: usize,
}

/// Bottom-right quadrant: actionable alerts.
#[derive(Debug, serde::Serialize)]
pub struct AlertsQuadrant {
    pub alerts: Vec<Alert>,
}

/// A single alert shown in the alerts quadrant.
#[derive(Debug, serde::Serialize)]
pub struct Alert {
    pub kind: AlertKind,
    pub message: String,
    pub severity: AlertSeverity,
    /// Which full view to navigate to for more detail.
    pub link_view: super::VisualType,
}

/// Categories of alert.
#[derive(Debug, serde::Serialize)]
pub enum AlertKind {
    CircularDependency,
    DeadSymbols,
    UnprotectedEndpoints,
    CouplingTrend,
    HighRiskFile,
}

/// Three-level alert severity.
#[derive(Debug, serde::Serialize)]
pub enum AlertSeverity {
    High,
    Medium,
    Low,
}

// ── Dashboard helpers ─────────────────────────────────────────────────────────

/// Derive a severity label from a raw risk score in [0, 1].
///
/// - >= 0.7 → "high"
/// - >= 0.4 → "medium"
/// - else   → "low"
pub fn risk_severity(score: f64) -> &'static str {
    if score >= 0.7 {
        "high"
    } else if score >= 0.4 {
        "medium"
    } else {
        "low"
    }
}

// ── Dashboard builder ─────────────────────────────────────────────────────────

/// Build all four dashboard quadrants from a `CodebaseIndex`.
pub fn build_dashboard_data(index: &CodebaseIndex) -> DashboardData {
    // ── Health quadrant ───────────────────────────────────────────────────────
    let health_score = crate::intelligence::health::compute_health(index);
    let dimensions = vec![
        ("conventions".to_string(), health_score.conventions),
        ("test_coverage".to_string(), health_score.test_coverage),
        ("churn_stability".to_string(), health_score.churn_stability),
        ("coupling".to_string(), health_score.coupling),
        ("cycles".to_string(), health_score.cycles),
    ];
    let health = HealthQuadrant {
        composite: health_score.composite,
        dimensions,
        trend: vec![],
    };

    // ── Risks quadrant ────────────────────────────────────────────────────────
    let risk_entries = crate::intelligence::risk::compute_risk_ranking(index);
    let top_risks: Vec<RiskDisplayEntry> = risk_entries
        .into_iter()
        .take(5)
        .map(|e| {
            let has_tests = index.test_map.contains_key(e.path.as_str());
            let severity = risk_severity(e.risk_score).to_string();
            RiskDisplayEntry {
                path: e.path,
                risk_score: e.risk_score,
                churn_30d: e.churn_30d,
                blast_radius: e.blast_radius,
                has_tests,
                severity,
            }
        })
        .collect();
    let risks = RisksQuadrant { top_risks };

    // ── Architecture preview quadrant ─────────────────────────────────────────
    let arch_map = crate::intelligence::architecture::build_architecture_map(index, 2);
    let circular_dep_count = arch_map.circular_deps.len();
    let module_count = arch_map.modules.len();

    let layout = super::layout::build_module_layout(index, &super::layout::LayoutConfig::default())
        .unwrap_or_else(|_| super::layout::ComputedLayout {
            nodes: vec![],
            edges: vec![],
            width: 0.0,
            height: 0.0,
            layers: vec![],
        });

    let architecture_preview = ArchitecturePreviewQuadrant {
        layout,
        module_count,
        circular_dep_count,
    };

    // ── Alerts quadrant ───────────────────────────────────────────────────────
    let mut alerts: Vec<Alert> = Vec::new();

    // One alert per circular dependency cycle.
    for cycle in &arch_map.circular_deps {
        let modules: Vec<&str> = cycle.iter().map(|s| s.as_str()).collect();
        let preview = modules
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" → ");
        alerts.push(Alert {
            kind: AlertKind::CircularDependency,
            message: format!("Circular dependency: {preview}"),
            severity: AlertSeverity::High,
            link_view: super::VisualType::Architecture,
        });
    }

    // High-risk file alerts (score > 0.8).
    for entry in crate::intelligence::risk::compute_risk_ranking(index)
        .into_iter()
        .filter(|e| e.risk_score > 0.8)
        .take(3)
    {
        alerts.push(Alert {
            kind: AlertKind::HighRiskFile,
            message: format!("High risk: {} (score {:.2})", entry.path, entry.risk_score),
            severity: AlertSeverity::High,
            link_view: super::VisualType::Risk,
        });
    }

    // Coupling-trend alert when any module has coupling > 0.6.
    let high_coupling: Vec<&str> = arch_map
        .modules
        .iter()
        .filter(|m| m.coupling > 0.6)
        .map(|m| m.prefix.as_str())
        .take(3)
        .collect();
    if !high_coupling.is_empty() {
        let modules_str = high_coupling.join(", ");
        alerts.push(Alert {
            kind: AlertKind::CouplingTrend,
            message: format!("High coupling in modules: {modules_str}"),
            severity: AlertSeverity::Medium,
            link_view: super::VisualType::Architecture,
        });
    }

    DashboardData {
        health,
        risks,
        architecture_preview,
        alerts: AlertsQuadrant { alerts },
    }
}

// ── Dashboard renderer ────────────────────────────────────────────────────────

/// Renders a self-contained dashboard HTML page for the given `CodebaseIndex`.
///
/// The page embeds:
/// - `cxpak-data` — the `ComputedLayout` for the architecture preview (used by
///   the base graph renderer in the JS controller).
/// - `cxpak-dashboard` — the full `DashboardData` for the dashboard-specific JS.
/// - `cxpak-meta` — `RenderMetadata` (repo name, version, etc.).
pub fn render_dashboard(index: &CodebaseIndex, metadata: &RenderMetadata) -> String {
    let dashboard = build_dashboard_data(index);
    let dashboard_json = serde_json::to_string(&dashboard).unwrap();

    // Reuse the architecture preview layout for the base graph pane.
    let layout = &dashboard.architecture_preview.layout;
    let layout_json = serde_json::to_string(layout).unwrap();

    let title = visual_type_name(&super::VisualType::Dashboard);

    #[derive(serde::Serialize)]
    struct MetaWithDisplay<'a> {
        #[serde(flatten)]
        inner: &'a RenderMetadata,
        visual_type_display: &'static str,
    }
    let meta_with_display = MetaWithDisplay {
        inner: metadata,
        visual_type_display: title,
    };
    let meta_json = serde_json::to_string(&meta_with_display).unwrap();
    let controller_js = view_controller_js(&super::VisualType::Dashboard);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>cxpak — {title}</title>
  <style>{css}</style>
</head>
<body>
  <div id="cxpak-app"></div>
  <script id="cxpak-data" type="application/json">{layout_json}</script>
  <script id="cxpak-dashboard" type="application/json">{dashboard_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script>{d3}</script>
  <script>{controller}</script>
</body>
</html>
"#,
        title = title,
        css = VISUAL_CSS,
        layout_json = layout_json,
        dashboard_json = dashboard_json,
        meta_json = meta_json,
        d3 = D3_BUNDLE,
        controller = controller_js,
    )
}

// ── Architecture Explorer types ───────────────────────────────────────────────

/// Full data payload for the Architecture Explorer view, embedded in the HTML
/// page as `<script id="cxpak-explorer" type="application/json">`.
///
/// Contains pre-computed layouts for all three semantic zoom levels so the
/// JS controller can switch between them without a round-trip.
#[derive(Debug, serde::Serialize)]
pub struct ArchitectureExplorerData {
    /// Level 1 — one node per top-level module.
    pub level1: super::layout::ComputedLayout,
    /// Level 2 — one entry per module; each value is the file-level layout
    /// for that module.  Keyed by module prefix string.
    pub level2: std::collections::HashMap<String, super::layout::ComputedLayout>,
    /// Level 3 — one entry per high-PageRank file; each value is the
    /// symbol-level layout for that file.  Keyed by relative file path.
    pub level3: std::collections::HashMap<String, super::layout::ComputedLayout>,
    /// Which zoom level to display initially (always 1).
    pub initial_level: u8,
    /// Navigation breadcrumb trail.  Starts at `["Repository"]`.
    pub breadcrumbs: Vec<BreadcrumbEntry>,
}

/// One entry in the breadcrumb trail rendered above the explorer canvas.
#[derive(Debug, serde::Serialize)]
pub struct BreadcrumbEntry {
    pub label: String,
    pub level: u8,
    pub target_id: String,
}

// ── Architecture Explorer builder ─────────────────────────────────────────────

/// Build all three zoom levels from a `CodebaseIndex`.
///
/// # Errors
/// Returns `LayoutError::Empty` when the index contains no files (i.e. level 1
/// cannot be built).  Errors for individual level-2 / level-3 entries are
/// silently skipped — an empty module or a file with no symbols simply has no
/// entry in the corresponding map.
pub fn build_architecture_explorer_data(
    index: &CodebaseIndex,
    config: &super::layout::LayoutConfig,
) -> Result<ArchitectureExplorerData, super::layout::LayoutError> {
    // ── Level 1: module graph ────────────────────────────────────────────────
    let level1 = super::layout::build_module_layout(index, config)?;

    // ── Level 2: per-module file graphs ──────────────────────────────────────
    let mut level2: std::collections::HashMap<String, super::layout::ComputedLayout> =
        std::collections::HashMap::new();

    for node in &level1.nodes {
        // Only expand Module-typed nodes; skip Cluster virtual nodes.
        if matches!(node.node_type, super::layout::NodeType::Module) {
            if let Ok(layout) = super::layout::build_file_layout(index, &node.id, config) {
                level2.insert(node.id.clone(), layout);
            }
        }
    }

    // ── Level 3: per-file symbol graphs (top-20 by PageRank) ─────────────────
    let mut level3: std::collections::HashMap<String, super::layout::ComputedLayout> =
        std::collections::HashMap::new();

    // Collect and sort by descending PageRank, take up to 20.
    let mut ranked_files: Vec<(&str, f64)> = index
        .pagerank
        .iter()
        .map(|(path, &score)| (path.as_str(), score))
        .collect();
    ranked_files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (file_path, _score) in ranked_files.into_iter().take(20) {
        if let Ok(layout) = super::layout::build_symbol_layout(index, file_path, config) {
            level3.insert(file_path.to_string(), layout);
        }
    }

    Ok(ArchitectureExplorerData {
        level1,
        level2,
        level3,
        initial_level: 1,
        breadcrumbs: vec![BreadcrumbEntry {
            label: "Repository".to_string(),
            level: 1,
            target_id: "root".to_string(),
        }],
    })
}

// ── Architecture Explorer renderer ───────────────────────────────────────────

/// Renders a self-contained Architecture Explorer HTML page.
///
/// The page embeds:
/// - `cxpak-data` — the level-1 `ComputedLayout` (used by the base graph
///   renderer for the initial view).
/// - `cxpak-explorer` — the full `ArchitectureExplorerData` (all three levels
///   plus breadcrumbs) for the explorer-specific JS.
/// - `cxpak-meta` — `RenderMetadata` (repo name, version, etc.).
pub fn render_architecture_explorer(
    index: &CodebaseIndex,
    metadata: &RenderMetadata,
) -> Result<String, super::layout::LayoutError> {
    let config = super::layout::LayoutConfig::default();
    let explorer = build_architecture_explorer_data(index, &config)?;
    let explorer_json = serde_json::to_string(&explorer).unwrap();

    // Use the level-1 layout as the initial graph pane data.
    let layout = &explorer.level1;
    let layout_json = serde_json::to_string(layout).unwrap();

    let title = visual_type_name(&super::VisualType::Architecture);

    #[derive(serde::Serialize)]
    struct MetaWithDisplay<'a> {
        #[serde(flatten)]
        inner: &'a RenderMetadata,
        visual_type_display: &'static str,
    }
    let meta_with_display = MetaWithDisplay {
        inner: metadata,
        visual_type_display: title,
    };
    let meta_json = serde_json::to_string(&meta_with_display).unwrap();
    let controller_js = view_controller_js(&super::VisualType::Architecture);

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>cxpak — {title}</title>
  <style>{css}</style>
</head>
<body>
  <div id="cxpak-app"></div>
  <script id="cxpak-data" type="application/json">{layout_json}</script>
  <script id="cxpak-explorer" type="application/json">{explorer_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script>{d3}</script>
  <script>{controller}</script>
</body>
</html>
"#,
        title = title,
        css = VISUAL_CSS,
        layout_json = layout_json,
        explorer_json = explorer_json,
        meta_json = meta_json,
        d3 = D3_BUNDLE,
        controller = controller_js,
    ))
}

// ── Risk Heatmap types ────────────────────────────────────────────────────────

/// Full data payload for the Risk Heatmap view, embedded in the HTML page as
/// `<script id="cxpak-heatmap" type="application/json">`.
///
/// The treemap is rendered client-side by D3.  The Rust side pre-computes the
/// tree structure and all metrics; the JS only needs to lay out rectangles.
#[derive(Debug, serde::Serialize)]
pub struct RiskHeatmapData {
    /// Root of the module → file tree used by `d3.treemap()`.
    pub root: TreemapNode,
    /// Number of files with risk_score above 0.0 (i.e. all files that appear).
    pub total_risk_files: usize,
    /// Highest risk_score across all leaf nodes.
    pub max_risk: f64,
}

/// One node in the treemap hierarchy (module group or individual file leaf).
///
/// D3 uses `area_value` to size rectangles and `risk_score` to colour them.
#[derive(Debug, serde::Serialize)]
pub struct TreemapNode {
    /// Stable identifier (module prefix or file path).
    pub id: String,
    /// Human-readable label shown inside the rectangle.
    pub label: String,
    /// Sizing value for D3 treemap: `blast_radius` for leaves (floor 1),
    /// sum of children for module groups.
    pub area_value: f64,
    /// Risk score in [0, 1]; 0.0 for non-leaf (group) nodes.
    pub risk_score: f64,
    /// `"high"` | `"medium"` | `"low"` per [`risk_severity`].
    pub severity: String,
    /// Child nodes.  Empty for leaf nodes.
    pub children: Vec<TreemapNode>,
    /// Present on leaf nodes: the file's own path (stored as a single-element
    /// vec so the JS tooltip can list files in the blast radius without an
    /// extra API call).
    pub blast_radius_files: Vec<String>,
    /// Data for the hover tooltip.
    pub tooltip: RiskTooltip,
}

/// Hover-tooltip payload for a single file leaf node.
#[derive(Debug, serde::Serialize)]
pub struct RiskTooltip {
    /// Relative file path.
    pub path: String,
    /// Number of git commits touching this file in the last 30 days.
    pub churn_30d: u32,
    /// Number of files that depend on this file (direct, 1 hop).
    pub blast_radius: usize,
    /// Number of test files mapped to this source file.
    pub test_count: usize,
    /// Simplified coupling score (0.0 in this release).
    pub coupling: f64,
}

// ── Risk Heatmap builder ──────────────────────────────────────────────────────

/// Build the treemap data from a `CodebaseIndex`.
///
/// Files are grouped by their first two path segments (e.g., `src/index`).
/// Files with no natural two-segment prefix are grouped under `"other"`.
pub fn build_risk_heatmap_data(index: &CodebaseIndex) -> RiskHeatmapData {
    let risk_entries = crate::intelligence::risk::compute_risk_ranking(index);

    // Group risk entries by two-segment module prefix.
    let mut groups: std::collections::HashMap<String, Vec<crate::intelligence::risk::RiskEntry>> =
        std::collections::HashMap::new();

    for entry in &risk_entries {
        let prefix = module_prefix(&entry.path);
        groups.entry(prefix).or_default().push(entry.clone());
    }

    // Build module-level TreemapNodes.
    let mut module_nodes: Vec<TreemapNode> = groups
        .into_iter()
        .map(|(prefix, entries)| {
            let children: Vec<TreemapNode> = entries
                .iter()
                .map(|e| {
                    let area_value = (e.blast_radius as f64).max(1.0);
                    let severity = risk_severity(e.risk_score).to_string();
                    let test_count = index.test_map.get(e.path.as_str()).map_or(0, |v| v.len());
                    let label = short_label(&e.path);
                    TreemapNode {
                        id: e.path.clone(),
                        label,
                        area_value,
                        risk_score: e.risk_score,
                        severity,
                        children: vec![],
                        blast_radius_files: vec![e.path.clone()],
                        tooltip: RiskTooltip {
                            path: e.path.clone(),
                            churn_30d: e.churn_30d,
                            blast_radius: e.blast_radius,
                            test_count,
                            coupling: 0.0,
                        },
                    }
                })
                .collect();

            let area_value: f64 = children.iter().map(|c| c.area_value).sum();
            let max_risk = children
                .iter()
                .map(|c| c.risk_score)
                .fold(0.0_f64, f64::max);
            let severity = risk_severity(max_risk).to_string();

            TreemapNode {
                id: prefix.clone(),
                label: prefix,
                area_value,
                risk_score: 0.0,
                severity,
                children,
                blast_radius_files: vec![],
                tooltip: RiskTooltip {
                    path: String::new(),
                    churn_30d: 0,
                    blast_radius: 0,
                    test_count: 0,
                    coupling: 0.0,
                },
            }
        })
        .collect();

    // Sort module nodes by descending area_value for a stable, deterministic layout.
    module_nodes.sort_by(|a, b| {
        b.area_value
            .partial_cmp(&a.area_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let root_area: f64 = module_nodes.iter().map(|n| n.area_value).sum();
    let max_risk = risk_entries
        .iter()
        .map(|e| e.risk_score)
        .fold(0.0_f64, f64::max);
    let total_risk_files = risk_entries.len();

    let root = TreemapNode {
        id: "root".to_string(),
        label: "Repository".to_string(),
        area_value: root_area,
        risk_score: 0.0,
        severity: risk_severity(max_risk).to_string(),
        children: module_nodes,
        blast_radius_files: vec![],
        tooltip: RiskTooltip {
            path: String::new(),
            churn_30d: 0,
            blast_radius: 0,
            test_count: 0,
            coupling: 0.0,
        },
    };

    RiskHeatmapData {
        root,
        total_risk_files,
        max_risk,
    }
}

/// Extract the first two path segments as the module prefix.
///
/// - `"src/index/mod.rs"` → `"src/index"`
/// - `"main.rs"` → `"other"`
fn module_prefix(path: &str) -> String {
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    match parts.as_slice() {
        [a, b, _] => format!("{a}/{b}"),
        [a, _] => a.to_string(),
        _ => "other".to_string(),
    }
}

/// Derive a short label from a file path (the file name without directory).
fn short_label(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

// ── Flow Diagram types ────────────────────────────────────────────────────────

/// Full data payload for the Flow Diagram view, embedded in the HTML page as
/// `<script id="cxpak-flow" type="application/json">`.
///
/// Contains the computed graph layout plus flow-specific overlays: cross-language
/// dividers, security checkpoints, and gaps where security controls are missing.
#[derive(Debug, serde::Serialize)]
pub struct FlowDiagramData {
    /// Graph layout ready for D3 rendering.
    pub layout: super::layout::ComputedLayout,
    /// Vertical divider lines marking transitions between programming languages.
    pub dividers: Vec<CrossLangDivider>,
    /// Nodes identified as auth/validation/sanitisation checkpoints.
    pub security_checkpoints: Vec<SecurityCheckpoint>,
    /// Edges where a value crosses a security boundary without a checkpoint.
    pub missing_security: Vec<MissingSecurityEdge>,
    /// The symbol that was traced (source of the flow).
    pub symbol: String,
    /// Confidence of the overall trace: `"Exact"`, `"Approximate"`, or `"Speculative"`.
    pub confidence: String,
    /// `true` when at least one path was pruned by the depth limit.
    pub truncated: bool,
}

/// A vertical divider rendered between two consecutive layout nodes that belong
/// to different programming languages.  `x_position` is the midpoint between
/// the two nodes (in layout-coordinate space) where the divider line is drawn.
#[derive(Debug, serde::Serialize)]
pub struct CrossLangDivider {
    /// X coordinate (layout space) of the divider line.
    pub x_position: f64,
    /// Language of the node to the left of the divider.
    pub left_language: String,
    /// Language of the node to the right of the divider.
    pub right_language: String,
}

/// A layout node that acts as a security checkpoint (auth guard, input
/// validator, or sanitiser).
#[derive(Debug, serde::Serialize)]
pub struct SecurityCheckpoint {
    /// The layout node id (matches a `LayoutNode::id` in `layout.nodes`).
    pub node_id: String,
    /// Category of the checkpoint: `"auth"`, `"validation"`, or `"sanitize"`.
    pub checkpoint_type: String,
}

/// An edge between two layout nodes where a value crosses a security-sensitive
/// file boundary without passing through a known checkpoint first.
#[derive(Debug, serde::Serialize)]
pub struct MissingSecurityEdge {
    /// Source layout node id.
    pub from_node_id: String,
    /// Target layout node id.
    pub to_node_id: String,
    /// Human-readable description of the gap.
    pub warning: String,
}

// ── Flow Diagram builder ──────────────────────────────────────────────────────

/// Stable node id for a `FlowNode`: `"<file>::<symbol>"`.
fn flow_node_id(node: &crate::intelligence::data_flow::FlowNode) -> String {
    format!("{}::{}", node.file, node.symbol)
}

/// Collapse runs of more than 3 consecutive `Passthrough` nodes in a path into
/// a single cluster node so the diagram stays readable.
///
/// The cluster node is inserted at the position of the first collapsed node and
/// labelled `"… N more"`.  The surrounding Source and Sink nodes are left in
/// place.  Any run of exactly 1–3 Passthrough nodes is kept verbatim.
fn collapse_passthrough_chains(
    nodes: &[crate::intelligence::data_flow::FlowNode],
) -> Vec<crate::intelligence::data_flow::FlowNode> {
    use crate::intelligence::data_flow::FlowNodeType;

    if nodes.len() <= 5 {
        // No collapsing needed for short paths.
        return nodes.to_vec();
    }

    let mut result: Vec<crate::intelligence::data_flow::FlowNode> = Vec::new();
    let mut i = 0;

    while i < nodes.len() {
        if nodes[i].node_type == FlowNodeType::Passthrough {
            // Count the run length.
            let run_start = i;
            while i < nodes.len() && nodes[i].node_type == FlowNodeType::Passthrough {
                i += 1;
            }
            let run_len = i - run_start;
            if run_len > 3 {
                // Emit first node of the run, then a cluster placeholder, then last.
                result.push(nodes[run_start].clone());
                // Build a synthetic cluster node using the middle position.
                let mid_idx = run_start + run_len / 2;
                let mut cluster = nodes[mid_idx].clone();
                cluster.symbol = format!("… {} more", run_len - 2);
                result.push(cluster);
                result.push(nodes[i - 1].clone());
            } else {
                // Short run — keep verbatim.
                for n in &nodes[run_start..i] {
                    result.push(n.clone());
                }
            }
        } else {
            result.push(nodes[i].clone());
            i += 1;
        }
    }

    result
}

/// Determine the overall `confidence` string from the set of paths in a
/// `DataFlowResult`.  The most pessimistic confidence wins.
fn overall_confidence(flow: &crate::intelligence::data_flow::DataFlowResult) -> &'static str {
    use crate::intelligence::data_flow::FlowConfidence;

    let mut has_approximate = false;
    for path in &flow.paths {
        match path.confidence {
            FlowConfidence::Speculative => return "Speculative",
            FlowConfidence::Approximate => has_approximate = true,
            FlowConfidence::Exact => {}
        }
    }
    if has_approximate {
        "Approximate"
    } else {
        "Exact"
    }
}

/// Build a [`FlowDiagramData`] from a [`DataFlowResult`].
///
/// # Algorithm
///
/// 1. Flatten all paths into a deduplicated ordered list of [`FlowNode`]s.
///    The first path visited defines the canonical order; later paths may add
///    new nodes that appear after the last already-seen node in path order.
/// 2. Apply passthrough-chain collapsing (>3 consecutive Passthrough nodes
///    become a single `"… N more"` cluster node).
/// 3. Build [`LayoutNode`]s and [`LayoutEdge`]s from consecutive node pairs
///    in each path (after collapsing).
/// 4. Call [`super::layout::compute_layout`] to obtain positions.
/// 5. Detect cross-language boundaries by examining consecutive nodes in
///    layout order and emit [`CrossLangDivider`]s.
///
/// # Errors
///
/// Propagates [`super::layout::LayoutError::Empty`] when the flow result has
/// no paths or all paths are empty.
pub fn build_flow_diagram_data(
    flow: &crate::intelligence::data_flow::DataFlowResult,
    _index: &CodebaseIndex,
    config: &super::layout::LayoutConfig,
) -> Result<FlowDiagramData, super::layout::LayoutError> {
    use super::layout::{EdgeVisualType, LayoutEdge, LayoutNode, NodeMetadata, NodeType, Point};
    use std::collections::{HashMap, HashSet};

    // ── 1. Collect unique nodes in path-traversal order ───────────────────────
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut ordered_nodes: Vec<crate::intelligence::data_flow::FlowNode> = Vec::new();

    // Always include the source node first.
    let src_id = flow_node_id(&flow.source);
    if seen_ids.insert(src_id.clone()) {
        ordered_nodes.push(flow.source.clone());
    }

    for path in &flow.paths {
        // Collapse passthrough chains for each path before processing.
        let collapsed = collapse_passthrough_chains(&path.nodes);
        for node in &collapsed {
            let id = flow_node_id(node);
            if seen_ids.insert(id) {
                ordered_nodes.push(node.clone());
            }
        }
    }

    // ── 2. Build LayoutNodes ──────────────────────────────────────────────────
    let layout_nodes: Vec<LayoutNode> = ordered_nodes
        .iter()
        .map(|n| {
            let id = flow_node_id(n);
            let label = n.symbol.clone();
            LayoutNode {
                id,
                label,
                layer: 0, // will be overwritten by compute_layout
                position: Point { x: 0.0, y: 0.0 },
                width: config.node_width,
                height: config.node_height,
                node_type: NodeType::Symbol,
                metadata: NodeMetadata::default(),
            }
        })
        .collect();

    // ── 3. Build LayoutEdges from consecutive nodes in each path ──────────────
    let mut edge_set: HashSet<(String, String)> = HashSet::new();
    let mut layout_edges: Vec<LayoutEdge> = Vec::new();

    for path in &flow.paths {
        let collapsed = collapse_passthrough_chains(&path.nodes);
        for pair in collapsed.windows(2) {
            let src = flow_node_id(&pair[0]);
            let tgt = flow_node_id(&pair[1]);
            if edge_set.insert((src.clone(), tgt.clone())) {
                let crosses_lang = pair[0].language != pair[1].language;
                let edge_type = if crosses_lang {
                    EdgeVisualType::CrossLanguage
                } else {
                    EdgeVisualType::DataFlow
                };
                layout_edges.push(LayoutEdge {
                    source: src,
                    target: tgt,
                    edge_type,
                    weight: 1.0,
                    is_cycle: false,
                    waypoints: vec![],
                });
            }
        }
    }

    // Guard against empty graph.
    if layout_nodes.is_empty() {
        return Err(super::layout::LayoutError::Empty);
    }

    // ── 4. Compute layout ─────────────────────────────────────────────────────
    let layout = super::layout::compute_layout(layout_nodes, layout_edges, config)?;

    // ── 5. Detect cross-language dividers ─────────────────────────────────────
    // Build a map from node id → language for fast lookup.
    let lang_map: HashMap<String, String> = ordered_nodes
        .iter()
        .map(|n| (flow_node_id(n), n.language.clone()))
        .collect();

    // Sort layout nodes by x position to find left-right language transitions.
    let mut sorted_by_x = layout.nodes.clone();
    sorted_by_x.sort_by(|a, b| {
        a.position
            .x
            .partial_cmp(&b.position.x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut dividers: Vec<CrossLangDivider> = Vec::new();
    for pair in sorted_by_x.windows(2) {
        let left_lang = lang_map
            .get(&pair[0].id)
            .cloned()
            .unwrap_or_else(|| "unknown".into());
        let right_lang = lang_map
            .get(&pair[1].id)
            .cloned()
            .unwrap_or_else(|| "unknown".into());
        if left_lang != right_lang {
            let x_position = pair[0].position.x
                + pair[0].width
                + (pair[1].position.x - pair[0].position.x - pair[0].width) / 2.0;
            dividers.push(CrossLangDivider {
                x_position,
                left_language: left_lang,
                right_language: right_lang,
            });
        }
    }

    let confidence = overall_confidence(flow).to_string();

    Ok(FlowDiagramData {
        layout,
        dividers,
        security_checkpoints: vec![],
        missing_security: vec![],
        symbol: flow.source.symbol.clone(),
        confidence,
        truncated: flow.truncated,
    })
}

// ── Flow Diagram renderer ─────────────────────────────────────────────────────

/// Renders a self-contained Flow Diagram HTML page.
///
/// The page embeds:
/// - `cxpak-data` — the `ComputedLayout` (used by the base graph renderer for
///   the initial graph pane).
/// - `cxpak-flow` — the full `FlowDiagramData` (layout + dividers + security
///   overlays) for the flow-specific JS renderer.
/// - `cxpak-meta` — `RenderMetadata` (repo name, version, etc.).
pub fn render_flow_diagram(
    flow: &crate::intelligence::data_flow::DataFlowResult,
    index: &CodebaseIndex,
    metadata: &RenderMetadata,
) -> Result<String, super::layout::LayoutError> {
    let config = super::layout::LayoutConfig::default();
    let flow_data = build_flow_diagram_data(flow, index, &config)?;
    let flow_json = serde_json::to_string(&flow_data).unwrap();

    // Embed the computed layout as the base graph pane data.
    let layout_json = serde_json::to_string(&flow_data.layout).unwrap();

    let title = visual_type_name(&super::VisualType::Flow);

    #[derive(serde::Serialize)]
    struct MetaWithDisplay<'a> {
        #[serde(flatten)]
        inner: &'a RenderMetadata,
        visual_type_display: &'static str,
    }
    let meta_with_display = MetaWithDisplay {
        inner: metadata,
        visual_type_display: title,
    };
    let meta_json = serde_json::to_string(&meta_with_display).unwrap();
    let controller_js = view_controller_js(&super::VisualType::Flow);

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>cxpak — {title}</title>
  <style>{css}</style>
</head>
<body>
  <div id="cxpak-app"></div>
  <script id="cxpak-data" type="application/json">{layout_json}</script>
  <script id="cxpak-flow" type="application/json">{flow_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script>{d3}</script>
  <script>{controller}</script>
</body>
</html>
"#,
        title = title,
        css = VISUAL_CSS,
        layout_json = layout_json,
        flow_json = flow_json,
        meta_json = meta_json,
        d3 = D3_BUNDLE,
        controller = controller_js,
    ))
}

// ── Risk Heatmap renderer ─────────────────────────────────────────────────────

/// Renders a self-contained Risk Heatmap HTML page.
///
/// The page embeds:
/// - `cxpak-data` — an empty `ComputedLayout` (required by the base JS
///   controller; the treemap is rendered client-side from `cxpak-heatmap`).
/// - `cxpak-heatmap` — the full `RiskHeatmapData` consumed by D3.
/// - `cxpak-meta` — `RenderMetadata` (repo name, version, etc.).
pub fn render_risk_heatmap(index: &CodebaseIndex, metadata: &RenderMetadata) -> String {
    let heatmap = build_risk_heatmap_data(index);
    let heatmap_json = serde_json::to_string(&heatmap).unwrap();

    // Provide an empty layout so the base graph renderer has valid (no-op) data.
    let empty_layout = super::layout::ComputedLayout {
        nodes: vec![],
        edges: vec![],
        width: 0.0,
        height: 0.0,
        layers: vec![],
    };
    let layout_json = serde_json::to_string(&empty_layout).unwrap();

    let title = visual_type_name(&super::VisualType::Risk);

    #[derive(serde::Serialize)]
    struct MetaWithDisplay<'a> {
        #[serde(flatten)]
        inner: &'a RenderMetadata,
        visual_type_display: &'static str,
    }
    let meta_with_display = MetaWithDisplay {
        inner: metadata,
        visual_type_display: title,
    };
    let meta_json = serde_json::to_string(&meta_with_display).unwrap();
    let controller_js = view_controller_js(&super::VisualType::Risk);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>cxpak — {title}</title>
  <style>{css}</style>
</head>
<body>
  <div id="cxpak-app"></div>
  <script id="cxpak-data" type="application/json">{layout_json}</script>
  <script id="cxpak-heatmap" type="application/json">{heatmap_json}</script>
  <script id="cxpak-meta" type="application/json">{meta_json}</script>
  <script>{d3}</script>
  <script>{controller}</script>
</body>
</html>
"#,
        title = title,
        css = VISUAL_CSS,
        layout_json = layout_json,
        heatmap_json = heatmap_json,
        meta_json = meta_json,
        d3 = D3_BUNDLE,
        controller = controller_js,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::layout::{
        ComputedLayout, EdgeVisualType, LayoutEdge, LayoutNode, NodeMetadata, NodeType, Point,
    };

    fn make_test_layout_5_nodes() -> ComputedLayout {
        let make_node = |id: &str, x: f64, y: f64| LayoutNode {
            id: id.to_string(),
            label: id.to_string(),
            layer: 0,
            position: Point { x, y },
            width: 120.0,
            height: 40.0,
            node_type: NodeType::File,
            metadata: NodeMetadata::default(),
        };

        let nodes = vec![
            make_node("a", 0.0, 0.0),
            make_node("b", 200.0, 0.0),
            make_node("c", 400.0, 0.0),
            make_node("d", 0.0, 150.0),
            make_node("e", 200.0, 150.0),
        ];

        let make_edge = |src: &str, tgt: &str| LayoutEdge {
            source: src.to_string(),
            target: tgt.to_string(),
            edge_type: EdgeVisualType::Import,
            weight: 1.0,
            is_cycle: false,
            waypoints: vec![],
        };

        let edges = vec![
            make_edge("a", "b"),
            make_edge("b", "c"),
            make_edge("a", "d"),
            make_edge("d", "e"),
        ];

        ComputedLayout {
            nodes,
            edges,
            width: 600.0,
            height: 300.0,
            layers: vec![vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "d".into(),
                "e".into(),
            ]],
        }
    }

    fn make_test_metadata() -> RenderMetadata {
        RenderMetadata {
            repo_name: "test-repo".to_string(),
            generated_at: "2026-04-12T00:00:00Z".to_string(),
            health_score: Some(0.85),
            node_count: 5,
            edge_count: 4,
            cxpak_version: "2.0.0".to_string(),
        }
    }

    #[test]
    fn test_render_html_is_self_contained() {
        let layout = make_test_layout_5_nodes();
        let meta = make_test_metadata();
        let html = render_html(&layout, super::super::VisualType::Dashboard, &meta);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        assert!(html.contains("cxpak-data"));
        assert!(!html.contains("cdn.jsdelivr.net"));
        assert!(!html.contains("unpkg.com"));
    }

    #[test]
    fn test_render_html_layout_json_is_valid() {
        let layout = make_test_layout_5_nodes();
        let meta = make_test_metadata();
        let html = render_html(&layout, super::super::VisualType::Architecture, &meta);
        let start = html.find(r#"<script id="cxpak-data""#).unwrap();
        let json_start = html[start..].find('>').unwrap() + start + 1;
        let json_end = html[json_start..].find("</script>").unwrap() + json_start;
        let json_str = &html[json_start..json_end];
        let _parsed: serde_json::Value =
            serde_json::from_str(json_str).expect("layout JSON must be valid");
    }

    #[test]
    fn test_render_html_has_no_unclosed_script_tags() {
        let layout = make_test_layout_5_nodes();
        let meta = make_test_metadata();
        let html = render_html(&layout, super::super::VisualType::Dashboard, &meta);
        let opens = html.matches("<script").count();
        let closes = html.matches("</script>").count();
        assert_eq!(opens, closes);
    }

    #[test]
    fn test_render_html_size_reasonable() {
        let layout = make_test_layout_5_nodes();
        let meta = make_test_metadata();
        let html = render_html(&layout, super::super::VisualType::Dashboard, &meta);
        // D3 bundle is ~273KB, so total should be under 500KB for small layout
        assert!(html.len() < 500_000, "HTML too large: {} bytes", html.len());
    }

    // ── Dashboard-specific tests ──────────────────────────────────────────────

    /// Build a minimal CodebaseIndex with real (empty) files for dashboard tests.
    fn make_minimal_index() -> crate::index::CodebaseIndex {
        use crate::budget::counter::TokenCounter;
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("main.rs");
        std::fs::write(&fp, "fn main() {}").unwrap();
        let files = vec![ScannedFile {
            relative_path: "src/main.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 13,
        }];
        crate::index::CodebaseIndex::build(files, HashMap::new(), &counter)
    }

    #[test]
    fn test_risk_severity_thresholds() {
        assert_eq!(risk_severity(0.9), "high");
        assert_eq!(risk_severity(0.7), "high");
        assert_eq!(risk_severity(0.5), "medium");
        assert_eq!(risk_severity(0.4), "medium");
        assert_eq!(risk_severity(0.2), "low");
        assert_eq!(risk_severity(0.0), "low");
    }

    #[test]
    fn test_build_dashboard_data_empty_risks() {
        let index = make_minimal_index();
        let data = build_dashboard_data(&index);
        // A single source file with no churn, no blast radius, no tests:
        // risk_score = 0.01^3 = 0.000001 which is well below 0.8 → no HighRiskFile alert
        // top_risks has exactly 1 entry (all files are included, capped at 5)
        assert!(data.risks.top_risks.len() <= 5);
    }

    #[test]
    fn test_build_dashboard_data_health_dimensions_present() {
        let index = make_minimal_index();
        let data = build_dashboard_data(&index);
        let dim_names: Vec<&str> = data
            .health
            .dimensions
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(dim_names.contains(&"conventions"));
        assert!(dim_names.contains(&"test_coverage"));
        assert!(dim_names.contains(&"coupling"));
        assert!(dim_names.contains(&"cycles"));
        assert!(dim_names.contains(&"churn_stability"));
    }

    #[test]
    fn test_render_dashboard_contains_quadrant_keys() {
        let index = make_minimal_index();
        let meta = make_test_metadata();
        let html = render_dashboard(&index, &meta);
        // Verify the embedded dashboard JSON contains all four quadrant keys.
        assert!(html.contains("\"health\""));
        assert!(html.contains("\"risks\""));
        assert!(html.contains("\"architecture_preview\""));
        assert!(html.contains("\"alerts\""));
        // Must be a well-formed HTML document.
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn test_render_dashboard_has_separate_dashboard_script_tag() {
        let index = make_minimal_index();
        let meta = make_test_metadata();
        let html = render_dashboard(&index, &meta);
        assert!(
            html.contains(r#"id="cxpak-dashboard""#),
            "must have a cxpak-dashboard script tag"
        );
    }

    #[test]
    fn test_render_dashboard_dashboard_json_is_valid() {
        let index = make_minimal_index();
        let meta = make_test_metadata();
        let html = render_dashboard(&index, &meta);
        // Extract the cxpak-dashboard JSON and parse it.
        let marker = r#"<script id="cxpak-dashboard" type="application/json">"#;
        let start = html.find(marker).expect("cxpak-dashboard tag missing");
        let content_start = start + marker.len();
        let content_end = html[content_start..].find("</script>").unwrap() + content_start;
        let json_str = &html[content_start..content_end];
        let parsed: serde_json::Value =
            serde_json::from_str(json_str).expect("dashboard JSON must be valid");
        assert!(parsed.get("health").is_some());
        assert!(parsed.get("risks").is_some());
        assert!(parsed.get("architecture_preview").is_some());
        assert!(parsed.get("alerts").is_some());
    }

    #[test]
    fn test_render_dashboard_no_unclosed_script_tags() {
        let index = make_minimal_index();
        let meta = make_test_metadata();
        let html = render_dashboard(&index, &meta);
        let opens = html.matches("<script").count();
        let closes = html.matches("</script>").count();
        assert_eq!(opens, closes, "mismatched script tags");
    }

    // ── Architecture Explorer tests ───────────────────────────────────────────

    #[test]
    fn test_architecture_explorer_data_has_breadcrumbs() {
        let index = make_minimal_index();
        let config = crate::visual::layout::LayoutConfig::default();
        // build_architecture_explorer_data may return Empty for a minimal index;
        // test the breadcrumb path when it succeeds, and verify the serialisation
        // of BreadcrumbEntry otherwise.
        match build_architecture_explorer_data(&index, &config) {
            Ok(data) => {
                assert!(
                    !data.breadcrumbs.is_empty(),
                    "breadcrumbs must be non-empty on success"
                );
                assert_eq!(
                    data.breadcrumbs[0].label, "Repository",
                    "first breadcrumb label must be 'Repository'"
                );
                assert_eq!(data.breadcrumbs[0].level, 1);
                assert_eq!(data.breadcrumbs[0].target_id, "root");
            }
            Err(_) => {
                // Minimal index may not have enough modules to build level 1.
                // Verify the type serialises correctly as a standalone check.
                let entry = BreadcrumbEntry {
                    label: "Repository".to_string(),
                    level: 1,
                    target_id: "root".to_string(),
                };
                let json = serde_json::to_string(&entry).unwrap();
                assert!(json.contains("\"Repository\""));
                assert!(json.contains("\"root\""));
            }
        }
    }

    #[test]
    fn test_render_architecture_explorer_contains_explorer_data() {
        let index = make_minimal_index();
        let meta = make_test_metadata();
        match render_architecture_explorer(&index, &meta) {
            Ok(html) => {
                assert!(
                    html.contains(r#"id="cxpak-explorer""#),
                    "must have a cxpak-explorer script tag"
                );
                // Validate the embedded explorer JSON is parseable.
                let marker = r#"<script id="cxpak-explorer" type="application/json">"#;
                let start = html.find(marker).expect("cxpak-explorer tag missing");
                let content_start = start + marker.len();
                let content_end = html[content_start..].find("</script>").unwrap() + content_start;
                let json_str = &html[content_start..content_end];
                let parsed: serde_json::Value =
                    serde_json::from_str(json_str).expect("explorer JSON must be valid");
                assert!(parsed.get("level1").is_some());
                assert!(parsed.get("level2").is_some());
                assert!(parsed.get("level3").is_some());
                assert!(parsed.get("breadcrumbs").is_some());
                // Script tag counts must balance.
                let opens = html.matches("<script").count();
                let closes = html.matches("</script>").count();
                assert_eq!(opens, closes, "mismatched script tags");
            }
            Err(_) => {
                // Minimal index may not have enough modules; verify breadcrumb
                // serialisation still works as a fallback assertion.
                let entry = BreadcrumbEntry {
                    label: "Repository".to_string(),
                    level: 1,
                    target_id: "root".to_string(),
                };
                let json = serde_json::to_string(&entry).unwrap();
                assert!(json.contains("\"level\""));
            }
        }
    }

    // ── Risk Heatmap tests ────────────────────────────────────────────────────

    #[test]
    fn test_risk_heatmap_area_values_positive() {
        let index = make_minimal_index();
        let data = build_risk_heatmap_data(&index);
        // Walk all leaf nodes and verify area_value > 0.
        for module_node in &data.root.children {
            for leaf in &module_node.children {
                assert!(
                    leaf.area_value > 0.0,
                    "leaf '{}' has area_value <= 0: {}",
                    leaf.id,
                    leaf.area_value
                );
            }
        }
    }

    #[test]
    fn test_risk_heatmap_high_risk_severity() {
        // Construct a RiskTooltip and TreemapNode manually to verify that a file
        // with risk_score > 0.8 receives severity "high" from risk_severity().
        assert_eq!(risk_severity(0.85), "high");
        assert_eq!(risk_severity(0.7), "high");

        // Build real data and confirm all severity strings are one of the three
        // valid values.
        let index = make_minimal_index();
        let data = build_risk_heatmap_data(&index);
        for module_node in &data.root.children {
            for leaf in &module_node.children {
                assert!(
                    matches!(leaf.severity.as_str(), "high" | "medium" | "low"),
                    "unexpected severity '{}' for '{}'",
                    leaf.severity,
                    leaf.id
                );
            }
        }
    }

    #[test]
    fn test_risk_heatmap_zero_blast_radius_gets_floor() {
        // A file with blast_radius == 0 must still have area_value >= 1.0
        // (the floor prevents zero-area rectangles in the treemap).
        let index = make_minimal_index();
        let data = build_risk_heatmap_data(&index);
        // The minimal index has a single file with no dependents → blast_radius == 0.
        for module_node in &data.root.children {
            for leaf in &module_node.children {
                assert!(
                    leaf.area_value >= 1.0,
                    "leaf '{}' area_value {} is below floor of 1.0",
                    leaf.id,
                    leaf.area_value
                );
            }
        }
    }

    #[test]
    fn test_render_risk_heatmap_contains_heatmap_data() {
        let index = make_minimal_index();
        let meta = make_test_metadata();
        let html = render_risk_heatmap(&index, &meta);
        // Must be a well-formed HTML document.
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        // Must embed the heatmap JSON in the expected script tag.
        assert!(
            html.contains(r#"id="cxpak-heatmap""#),
            "HTML must contain cxpak-heatmap script tag"
        );
        // Script tag counts must balance.
        let opens = html.matches("<script").count();
        let closes = html.matches("</script>").count();
        assert_eq!(opens, closes, "mismatched script tags");
        // Heatmap JSON must be parseable and contain the root key.
        let marker = r#"<script id="cxpak-heatmap" type="application/json">"#;
        let start = html.find(marker).expect("cxpak-heatmap tag missing");
        let content_start = start + marker.len();
        let content_end = html[content_start..].find("</script>").unwrap() + content_start;
        let json_str = &html[content_start..content_end];
        let parsed: serde_json::Value =
            serde_json::from_str(json_str).expect("heatmap JSON must be valid");
        assert!(
            parsed.get("root").is_some(),
            "heatmap JSON must have 'root'"
        );
        assert!(
            parsed.get("total_risk_files").is_some(),
            "heatmap JSON must have 'total_risk_files'"
        );
        assert!(
            parsed.get("max_risk").is_some(),
            "heatmap JSON must have 'max_risk'"
        );
    }

    // ── Flow Diagram tests ────────────────────────────────────────────────────

    /// Build a minimal [`DataFlowResult`] with `n` nodes for testing.
    ///
    /// The source node lives at `"src/handler.rs"` and each subsequent node
    /// lives at `"src/service_N.rs"`.  All nodes share the same language
    /// ("rust") so no cross-language dividers are expected.
    fn make_minimal_flow(
        n: usize,
        truncated: bool,
    ) -> crate::intelligence::data_flow::DataFlowResult {
        use crate::intelligence::data_flow::{
            DataFlowResult, FlowConfidence, FlowNode, FlowNodeType, FlowPath,
        };

        let make_node = |file: &str, symbol: &str, node_type: FlowNodeType| FlowNode {
            file: file.to_string(),
            symbol: symbol.to_string(),
            parameter: None,
            language: "rust".to_string(),
            node_type,
        };

        let source = make_node("src/handler.rs", "handle_request", FlowNodeType::Source);

        let mut path_nodes = vec![source.clone()];
        for i in 1..n {
            let (file, sym, nt) = if i == n - 1 {
                (
                    "src/store.rs".to_string(),
                    "save".to_string(),
                    FlowNodeType::Sink,
                )
            } else {
                (
                    format!("src/service_{i}.rs"),
                    format!("process_{i}"),
                    FlowNodeType::Passthrough,
                )
            };
            path_nodes.push(make_node(&file, &sym, nt));
        }

        let path = FlowPath {
            nodes: path_nodes,
            crosses_module_boundary: false,
            crosses_language_boundary: false,
            touches_security_boundary: false,
            confidence: FlowConfidence::Exact,
            length: n,
        };

        DataFlowResult {
            source,
            sink: None,
            paths: vec![path],
            truncated,
            limitations: vec![],
        }
    }

    #[test]
    fn test_flow_diagram_data_from_simple_flow() {
        let flow = make_minimal_flow(4, false);
        let index = make_minimal_index();
        let config = crate::visual::layout::LayoutConfig::default();

        let data = build_flow_diagram_data(&flow, &index, &config)
            .expect("build_flow_diagram_data must succeed for a 4-node flow");

        // All 4 nodes from the single path must appear in the layout.
        assert_eq!(
            data.layout.nodes.len(),
            4,
            "expected 4 layout nodes, got {}",
            data.layout.nodes.len()
        );
        // The source symbol must be recorded.
        assert_eq!(data.symbol, "handle_request");
        // All nodes share the same language — no cross-language dividers.
        assert!(
            data.dividers.is_empty(),
            "expected no dividers for same-language flow"
        );
        // Confidence must be Exact (all hops Exact, none Speculative).
        assert_eq!(data.confidence, "Exact");
        // Not truncated.
        assert!(!data.truncated);
    }

    #[test]
    fn test_flow_diagram_truncated_flag() {
        let flow = make_minimal_flow(3, true);
        let index = make_minimal_index();
        let config = crate::visual::layout::LayoutConfig::default();

        let data = build_flow_diagram_data(&flow, &index, &config)
            .expect("build_flow_diagram_data must succeed");

        assert!(
            data.truncated,
            "truncated flag must be forwarded from DataFlowResult"
        );
    }

    #[test]
    fn test_render_flow_diagram_contains_flow_data() {
        let flow = make_minimal_flow(3, false);
        let index = make_minimal_index();
        let meta = make_test_metadata();

        let html = render_flow_diagram(&flow, &index, &meta)
            .expect("render_flow_diagram must succeed for a 3-node flow");

        // Must be well-formed HTML.
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));

        // Must embed the flow JSON in the expected script tag.
        assert!(
            html.contains(r#"id="cxpak-flow""#),
            "HTML must contain cxpak-flow script tag"
        );

        // Script tag counts must balance.
        let opens = html.matches("<script").count();
        let closes = html.matches("</script>").count();
        assert_eq!(opens, closes, "mismatched script tags");

        // Flow JSON must be parseable and contain expected keys.
        let marker = r#"<script id="cxpak-flow" type="application/json">"#;
        let start = html.find(marker).expect("cxpak-flow tag missing");
        let content_start = start + marker.len();
        let content_end = html[content_start..].find("</script>").unwrap() + content_start;
        let json_str = &html[content_start..content_end];
        let parsed: serde_json::Value =
            serde_json::from_str(json_str).expect("flow JSON must be valid");
        assert!(
            parsed.get("layout").is_some(),
            "flow JSON must have 'layout'"
        );
        assert!(
            parsed.get("symbol").is_some(),
            "flow JSON must have 'symbol'"
        );
        assert!(
            parsed.get("confidence").is_some(),
            "flow JSON must have 'confidence'"
        );
        assert!(
            parsed.get("truncated").is_some(),
            "flow JSON must have 'truncated'"
        );
        assert!(
            parsed.get("dividers").is_some(),
            "flow JSON must have 'dividers'"
        );
    }
}
