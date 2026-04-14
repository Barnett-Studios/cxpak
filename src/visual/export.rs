//! Multi-format export for visualizations.
//!
//! The export module serializes computed visualizations into various output formats:
//! Mermaid (diagram syntax), SVG (vector graphics), PNG (raster images),
//! C4 (model notation), and JSON (programmatic access).

use super::layout::ComputedLayout;
use super::render::RenderMetadata;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("SVG rendering failed: {0}")]
    SvgRender(String),
    #[error("PNG rasterization failed: {0}")]
    PngRaster(String),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Converts a node ID to a safe Mermaid identifier.
/// Replaces `/`, `.`, `-` with `_` and truncates to 32 chars.
fn mermaid_id(id: &str) -> String {
    let escaped: String = id
        .chars()
        .map(|c| match c {
            '/' | '.' | '-' => '_',
            c => c,
        })
        .collect();
    if escaped.len() > 32 {
        escaped.chars().take(32).collect()
    } else {
        escaped
    }
}

/// Renders a `ComputedLayout` as a Mermaid `graph TD` diagram.
pub fn to_mermaid(layout: &ComputedLayout) -> String {
    let mut out = String::from("graph TD\n");

    for node in &layout.nodes {
        let mid = mermaid_id(&node.id);
        // Escape label double-quotes by replacing with single-quotes
        let label = node.label.replace('"', "'");
        out.push_str(&format!("    {}[\"{}\"]\n", mid, label));
    }

    for edge in &layout.edges {
        let src = mermaid_id(&edge.source);
        let tgt = mermaid_id(&edge.target);
        out.push_str(&format!("    {} --> {}\n", src, tgt));
        if edge.is_cycle {
            out.push_str(&format!(
                "    style {} fill:#ff4444,stroke:#cc0000,color:#fff\n",
                src
            ));
            out.push_str(&format!(
                "    style {} fill:#ff4444,stroke:#cc0000,color:#fff\n",
                tgt
            ));
        }
    }

    out
}

/// Escapes special XML characters in attribute values and text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Renders a `ComputedLayout` as a pure SVG string (no JS, no interactivity).
pub fn to_svg(layout: &ComputedLayout, metadata: &RenderMetadata) -> String {
    let vw = layout.width.max(1.0);
    let vh = layout.height.max(1.0);

    // Build SVG header.
    // Colors use named constants to avoid raw-string-delimiter conflicts.
    let bg_color = "#1a1a2e";
    let node_fill = "#3a4a5a";
    let node_stroke = "#5a7a9a";
    let text_fill = "#e0e0e0";
    let edge_normal = "#5a7a9a";
    let edge_cycle = "#ff4444";

    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {vw} {vh}\" width=\"{vw}\" height=\"{vh}\">\n  <title>{repo}</title>\n  <rect width=\"{vw}\" height=\"{vh}\" fill=\"{bg}\"/>\n",
        vw = vw,
        vh = vh,
        repo = xml_escape(&metadata.repo_name),
        bg = bg_color,
    );

    for node in &layout.nodes {
        let x = node.position.x;
        let y = node.position.y;
        let w = node.width;
        let h = node.height;
        let label = xml_escape(&node.label);
        let cx = x + w / 2.0;
        let cy = y + h / 2.0 + 5.0;

        out.push_str(&format!(
            "  <rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"4\" ry=\"4\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1\"/>\n  <text x=\"{cx}\" y=\"{cy}\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"12\" fill=\"{text}\">{label}</text>\n",
            x = x,
            y = y,
            w = w,
            h = h,
            cx = cx,
            cy = cy,
            label = label,
            fill = node_fill,
            stroke = node_stroke,
            text = text_fill,
        ));
    }

    // Build a quick lookup: id -> position centre
    let centres: std::collections::HashMap<&str, (f64, f64)> = layout
        .nodes
        .iter()
        .map(|n| {
            (
                n.id.as_str(),
                (n.position.x + n.width / 2.0, n.position.y + n.height / 2.0),
            )
        })
        .collect();

    for edge in &layout.edges {
        let stroke = if edge.is_cycle {
            edge_cycle
        } else {
            edge_normal
        };

        if !edge.waypoints.is_empty() {
            // Polyline through waypoints
            let mut pts = String::new();
            if let Some(&(sx, sy)) = centres.get(edge.source.as_str()) {
                pts.push_str(&format!("{},{} ", sx, sy));
            }
            for wp in &edge.waypoints {
                pts.push_str(&format!("{},{} ", wp.x, wp.y));
            }
            if let Some(&(tx, ty)) = centres.get(edge.target.as_str()) {
                pts.push_str(&format!("{},{}", tx, ty));
            }
            out.push_str(&format!(
                "  <polyline points=\"{pts}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"1.5\"/>\n",
                pts = pts.trim(),
                stroke = stroke,
            ));
        } else if let (Some(&(x1, y1)), Some(&(x2, y2))) = (
            centres.get(edge.source.as_str()),
            centres.get(edge.target.as_str()),
        ) {
            out.push_str(&format!(
                "  <line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{stroke}\" stroke-width=\"1.5\"/>\n",
                x1 = x1,
                y1 = y1,
                x2 = x2,
                y2 = y2,
                stroke = stroke,
            ));
        }
    }

    out.push_str("</svg>\n");
    out
}

/// Rasterizes a `ComputedLayout` to a PNG byte vector using resvg.
#[cfg(feature = "visual")]
pub fn to_png(
    layout: &ComputedLayout,
    metadata: &RenderMetadata,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ExportError> {
    use resvg::{tiny_skia, usvg};

    let svg_str = to_svg(layout, metadata);
    let opt = usvg::Options::default();
    let tree =
        usvg::Tree::from_str(&svg_str, &opt).map_err(|e| ExportError::SvgRender(e.to_string()))?;

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| ExportError::PngRaster("failed to allocate pixmap".into()))?;

    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap
        .encode_png()
        .map_err(|e| ExportError::PngRaster(e.to_string()))
}

/// Renders a `ComputedLayout` as a C4 model in Structurizr DSL format.
pub fn to_c4(layout: &ComputedLayout, metadata: &RenderMetadata) -> String {
    let repo = &metadata.repo_name;
    let mut out = format!(
        "workspace {{\n  model {{\n    softwareSystem \"{}\" {{\n",
        repo
    );

    // Emit containers
    for node in &layout.nodes {
        let label = node.label.replace('"', "'");
        out.push_str(&format!("      container \"{}\" {{}}\n", label));
    }

    // Emit relationships using Structurizr-style variable names
    let var_name = |label: &str| -> String {
        label
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>()
    };

    // Build id -> label map
    let id_to_label: std::collections::HashMap<&str, &str> = layout
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();

    for edge in &layout.edges {
        if let (Some(src_label), Some(tgt_label)) = (
            id_to_label.get(edge.source.as_str()),
            id_to_label.get(edge.target.as_str()),
        ) {
            let src_var = var_name(src_label);
            let tgt_var = var_name(tgt_label);
            out.push_str(&format!(
                "      {} -> {} \"depends on\"\n",
                src_var, tgt_var
            ));
        }
    }

    out.push_str("    }\n  }\n}\n");
    out
}

/// Serializes a `ComputedLayout` to pretty-printed JSON.
pub fn to_json(layout: &ComputedLayout) -> String {
    serde_json::to_string_pretty(layout)
        .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::layout::{
        ComputedLayout, EdgeVisualType, LayoutEdge, LayoutNode, NodeMetadata, NodeType, Point,
    };
    use crate::visual::render::RenderMetadata;

    fn make_3_module_layout() -> ComputedLayout {
        ComputedLayout {
            nodes: vec![
                LayoutNode {
                    id: "src/api".into(),
                    label: "api".into(),
                    layer: 0,
                    position: Point { x: 0.0, y: 0.0 },
                    width: 160.0,
                    height: 48.0,
                    node_type: NodeType::Module,
                    metadata: NodeMetadata::default(),
                },
                LayoutNode {
                    id: "src/db".into(),
                    label: "db".into(),
                    layer: 1,
                    position: Point { x: 0.0, y: 120.0 },
                    width: 160.0,
                    height: 48.0,
                    node_type: NodeType::Module,
                    metadata: NodeMetadata::default(),
                },
                LayoutNode {
                    id: "src/util".into(),
                    label: "util".into(),
                    layer: 1,
                    position: Point { x: 220.0, y: 120.0 },
                    width: 160.0,
                    height: 48.0,
                    node_type: NodeType::Module,
                    metadata: NodeMetadata::default(),
                },
            ],
            edges: vec![
                LayoutEdge {
                    source: "src/api".into(),
                    target: "src/db".into(),
                    edge_type: EdgeVisualType::Import,
                    weight: 1.0,
                    is_cycle: false,
                    waypoints: vec![],
                },
                LayoutEdge {
                    source: "src/api".into(),
                    target: "src/util".into(),
                    edge_type: EdgeVisualType::Import,
                    weight: 1.0,
                    is_cycle: false,
                    waypoints: vec![],
                },
            ],
            width: 380.0,
            height: 168.0,
            layers: vec![
                vec!["src/api".into()],
                vec!["src/db".into(), "src/util".into()],
            ],
        }
    }

    fn make_test_meta() -> RenderMetadata {
        RenderMetadata {
            repo_name: "test-repo".into(),
            generated_at: "2026-04-12T00:00:00Z".into(),
            health_score: Some(0.85),
            node_count: 3,
            edge_count: 2,
            cxpak_version: "2.0.0".into(),
        }
    }

    #[test]
    fn test_to_mermaid_3_modules() {
        let layout = make_3_module_layout();
        let mermaid = to_mermaid(&layout);
        assert!(mermaid.starts_with("graph TD"));
        // 3 node definitions — IDs have / replaced with _
        assert!(mermaid.contains("src_api"));
        assert!(mermaid.contains("src_db"));
        assert!(mermaid.contains("src_util"));
    }

    #[test]
    fn test_mermaid_id_escaping() {
        assert_eq!(mermaid_id("src/index/mod.rs"), "src_index_mod_rs");
    }

    #[test]
    fn test_to_svg_valid() {
        let layout = make_3_module_layout();
        let meta = make_test_meta();
        let svg = to_svg(&layout, &meta);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    #[cfg(feature = "visual")]
    fn test_to_png_magic_bytes() {
        let layout = make_3_module_layout();
        let meta = make_test_meta();
        let bytes = to_png(&layout, &meta, 800, 600).unwrap();
        assert_eq!(&bytes[..4], &[137, 80, 78, 71]); // PNG magic
    }

    #[test]
    fn test_to_c4_modules() {
        let layout = make_3_module_layout();
        let meta = make_test_meta();
        let c4 = to_c4(&layout, &meta);
        assert!(c4.contains("container"));
    }

    #[test]
    fn test_to_json_roundtrip() {
        let layout = make_3_module_layout();
        let json = to_json(&layout);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["nodes"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_mermaid_id_truncation() {
        let long_id = "a".repeat(50);
        let result = mermaid_id(&long_id);
        assert!(result.len() <= 32);
    }

    // ── Additional export tests ───────────────────────────────────────────────

    fn make_cycle_layout() -> ComputedLayout {
        let nodes = vec![
            LayoutNode {
                id: "src/a".into(),
                label: "a".into(),
                layer: 0,
                position: Point { x: 0.0, y: 0.0 },
                width: 160.0,
                height: 48.0,
                node_type: NodeType::Module,
                metadata: NodeMetadata::default(),
            },
            LayoutNode {
                id: "src/b".into(),
                label: "b".into(),
                layer: 1,
                position: Point { x: 0.0, y: 120.0 },
                width: 160.0,
                height: 48.0,
                node_type: NodeType::Module,
                metadata: NodeMetadata::default(),
            },
        ];
        let edges = vec![LayoutEdge {
            source: "src/a".into(),
            target: "src/b".into(),
            edge_type: EdgeVisualType::Import,
            weight: 1.0,
            is_cycle: true,
            waypoints: vec![],
        }];
        ComputedLayout {
            nodes,
            edges,
            width: 200.0,
            height: 200.0,
            layers: vec![vec!["src/a".into()], vec!["src/b".into()]],
        }
    }

    fn make_waypoint_layout() -> ComputedLayout {
        let nodes = vec![
            LayoutNode {
                id: "src/a".into(),
                label: "a".into(),
                layer: 0,
                position: Point { x: 0.0, y: 0.0 },
                width: 160.0,
                height: 48.0,
                node_type: NodeType::Module,
                metadata: NodeMetadata::default(),
            },
            LayoutNode {
                id: "src/b".into(),
                label: "b".into(),
                layer: 2,
                position: Point { x: 0.0, y: 240.0 },
                width: 160.0,
                height: 48.0,
                node_type: NodeType::Module,
                metadata: NodeMetadata::default(),
            },
        ];
        let edges = vec![LayoutEdge {
            source: "src/a".into(),
            target: "src/b".into(),
            edge_type: EdgeVisualType::Import,
            weight: 1.0,
            is_cycle: false,
            waypoints: vec![Point { x: 80.0, y: 120.0 }],
        }];
        ComputedLayout {
            nodes,
            edges,
            width: 200.0,
            height: 288.0,
            layers: vec![vec!["src/a".into()], vec!["src/b".into()]],
        }
    }

    fn make_empty_layout() -> ComputedLayout {
        ComputedLayout {
            nodes: vec![],
            edges: vec![],
            width: 0.0,
            height: 0.0,
            layers: vec![],
        }
    }

    #[test]
    fn test_to_mermaid_with_cycle_edge_has_style_directive() {
        let layout = make_cycle_layout();
        let mermaid = to_mermaid(&layout);
        assert!(
            mermaid.contains("style"),
            "cycle edge should emit 'style' directive, got:\n{mermaid}"
        );
    }

    #[test]
    fn test_to_mermaid_empty_layout_starts_with_graph_td() {
        let layout = make_empty_layout();
        let mermaid = to_mermaid(&layout);
        assert!(mermaid.starts_with("graph TD"), "got: {mermaid}");
    }

    #[test]
    fn test_to_svg_cycle_edge_uses_distinct_stroke_color() {
        let layout = make_cycle_layout();
        let meta = make_test_meta();
        let svg = to_svg(&layout, &meta);
        // Cycle stroke is "#ff4444"; normal is "#5a7a9a"
        assert!(
            svg.contains("#ff4444"),
            "cycle edge should use red stroke '#ff4444', got:\n{svg}"
        );
    }

    #[test]
    fn test_to_svg_with_waypoints_renders_polyline() {
        let layout = make_waypoint_layout();
        let meta = make_test_meta();
        let svg = to_svg(&layout, &meta);
        assert!(
            svg.contains("<polyline"),
            "edge with waypoints should render as polyline, got:\n{svg}"
        );
    }

    #[test]
    fn test_to_c4_uses_container_keyword() {
        let layout = make_3_module_layout();
        let meta = make_test_meta();
        let c4 = to_c4(&layout, &meta);
        // Structurizr DSL uses "container" for each module
        assert!(
            c4.contains("container"),
            "C4 output must use 'container' keyword, got:\n{c4}"
        );
    }

    #[test]
    fn test_to_json_does_not_panic_on_normal_layout() {
        let layout = make_3_module_layout();
        let json = to_json(&layout);
        // Must start with '{' — not an error placeholder
        assert!(
            json.trim_start().starts_with('{'),
            "expected JSON object, got: {json}"
        );
    }

    #[test]
    fn test_to_json_roundtrip_node_count() {
        let layout = make_3_module_layout();
        let json = to_json(&layout);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["nodes"].as_array().unwrap().len(),
            3,
            "roundtrip must preserve 3 nodes"
        );
    }

    #[test]
    fn test_mermaid_id_multibyte_unicode_no_panic() {
        // Multi-byte Unicode characters should not panic.
        let id = "src/αβγ/δεζ.rs";
        let result = mermaid_id(id);
        assert!(
            !result.is_empty(),
            "mermaid_id should return non-empty string"
        );
        assert!(result.len() <= 32, "result should be at most 32 chars");
    }

    #[test]
    #[cfg(feature = "visual")]
    fn test_to_png_magic_bytes_and_minimum_size() {
        let layout = make_3_module_layout();
        let meta = make_test_meta();
        let bytes = to_png(&layout, &meta, 800, 600).unwrap();
        // PNG magic bytes: 0x89 50 4E 47
        assert_eq!(
            &bytes[..4],
            &[137, 80, 78, 71],
            "must start with PNG magic bytes"
        );
        assert!(
            bytes.len() > 1024,
            "PNG output must be > 1KB, got {} bytes",
            bytes.len()
        );
    }
}
