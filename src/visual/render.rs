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
}
