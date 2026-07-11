//! Multi-format export for visualizations.
//!
//! The export module serializes computed visualizations into various output formats:
//! Mermaid (diagram syntax), SVG (vector graphics), PNG (raster images),
//! C4 (model notation), and JSON (programmatic access).

use super::layout::ComputedLayout;
use super::render::RenderMetadata;
use crate::core_graph::graph::{DependencyGraph, EdgeConfidence};
use std::collections::BTreeSet;

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
    let char_count = escaped.chars().count();
    if char_count > 32 {
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
        // Sanitize label: replace characters that break Mermaid syntax.
        let label = node
            .label
            .replace('"', "'")
            .replace('[', "(")
            .replace(']', ")")
            .replace('|', "/");
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
///
/// C0 control bytes that are illegal in XML 1.0 (everything below U+0020 except
/// tab/LF/CR) have no valid representation — not even a numeric char ref — so
/// they are dropped before escaping. Node labels and repo names come from
/// git-tracked paths/symbols, which on Unix may legally contain such bytes; a
/// stray one would otherwise emit XML no conformant parser can load.
/// ponytail: strips C0 only — U+FFFE/U+FFFF and other Unicode noncharacters
/// (astronomically unlikely in a path) are left to the caller.
fn xml_escape(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\t' || c == '\n' || c == '\r' || c >= '\u{20}')
        .collect::<String>()
        .replace('&', "&amp;")
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

    const MAX_DIM: u32 = 16384;
    if width > MAX_DIM || height > MAX_DIM {
        return Err(ExportError::PngRaster(format!(
            "canvas dimensions {width}x{height} exceed maximum {MAX_DIM}x{MAX_DIM}"
        )));
    }

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

/// Escape a string for use in a Structurizr C4 DSL string literal.
///
/// Removes characters that would break the DSL parser: `"` becomes a space,
/// `{` and `}` are stripped.
fn escape_c4(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' | '{' | '}' => ' ',
            other => other,
        })
        .collect()
}

/// Renders a `ComputedLayout` as a C4 model in Structurizr DSL format.
pub fn to_c4(layout: &ComputedLayout, metadata: &RenderMetadata) -> String {
    let repo = escape_c4(&metadata.repo_name);
    let mut out = format!(
        "workspace {{\n  model {{\n    softwareSystem \"{}\" {{\n",
        repo
    );

    // Build id → index map for relationship lookup.
    let id_to_index: std::collections::HashMap<&str, usize> = layout
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // Emit containers as named bindings: c0, c1, …
    for (i, node) in layout.nodes.iter().enumerate() {
        out.push_str(&format!(
            "      c{i} = container \"{}\" {{}}\n",
            escape_c4(&node.label)
        ));
    }

    // Emit relationships using the numeric variable names.
    for edge in &layout.edges {
        if let (Some(&src), Some(&tgt)) = (
            id_to_index.get(edge.source.as_str()),
            id_to_index.get(edge.target.as_str()),
        ) {
            out.push_str(&format!("      c{src} -> c{tgt} \"depends on\"\n"));
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

// ─── Graph-serialization exports (Cypher + GraphML) ────────────────────────────
//
// Unlike the visual exporters above (which consume the laid-out `ComputedLayout`
// of module nodes + `EdgeVisualType` edges), these serialize the *dependency
// graph itself* — `DependencyGraph`'s typed edges carry the honest
// `EdgeType` + `EdgeConfidence` (Phase A, ADR-0097 descriptive-honesty) that the
// positional layout drops. They reuse the existing `index.graph` rather than
// re-deriving it; the dispatch sites pass it in.

/// String form of an [`EdgeConfidence`] for export metadata.
///
/// Derived from [`EdgeConfidence::is_inferred`] rather than `Debug` so the
/// emitted token is a deliberate, stable part of the export contract.
fn confidence_str(confidence: EdgeConfidence) -> &'static str {
    if confidence.is_inferred() {
        "Inferred"
    } else {
        "Extracted"
    }
}

/// Collect every node id in the graph (edge sources, edge targets, and reverse
/// roots) as a canonically sorted, de-duplicated list.
///
/// `DependencyGraph` stores adjacency as `BTreeMap`/`BTreeSet`, so insertion
/// order here is already deterministic; the explicit `BTreeSet` guarantees the
/// node list is sorted and unique regardless of which side an isolated node
/// appears on.
fn collect_graph_nodes(graph: &DependencyGraph) -> Vec<&str> {
    let mut nodes: BTreeSet<&str> = BTreeSet::new();
    for (source, targets) in &graph.edges {
        nodes.insert(source.as_str());
        for edge in targets {
            nodes.insert(edge.target.as_str());
        }
    }
    // Reverse roots catch nodes that only ever appear as a target (already
    // covered above) and any source-less sink; harmless to re-insert.
    for target in graph.reverse_edges.keys() {
        nodes.insert(target.as_str());
    }
    nodes.into_iter().collect()
}

/// Map a node id to its Cypher/GraphML node kind.
///
/// Synthetic column nodes (Task A2) are keyed `col:{table}.{column}`; the `col:`
/// prefix can never collide with a real file path, so any id carrying it is a
/// `Column`. Everything else is an indexed source `File`.
fn node_kind(id: &str) -> &'static str {
    if id.starts_with("col:") {
        "Column"
    } else {
        "File"
    }
}

/// Escape a string for a single-quoted Cypher string literal.
///
/// Node ids/paths are attacker-influenced (a repo can contain a file whose name
/// holds a quote, backslash, or newline), so this is a correctness + injection
/// boundary: the output must always be a syntactically valid, non-breakable
/// literal. Backslash is escaped first (handled char-by-char, so order is not a
/// hazard), then the quote and the control characters that would otherwise split
/// the statement across lines.
fn escape_cypher(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// Serialize a [`DependencyGraph`] as a Neo4j-importable Cypher script.
///
/// Shape (canonical, deterministic):
/// - A header comment naming the repo and the node/relationship counts.
/// - One `MERGE` per node, labeled by kind (`:File` / `:Column`) with an `id`
///   property. `MERGE` (not `CREATE`) so re-running the script against an
///   existing graph is idempotent — node identity is the `id` property.
/// - One `MERGE` per relationship: endpoints matched by `id`, a single
///   `DEPENDS_ON` relationship type carrying `type` (the [`EdgeType::label`]),
///   `confidence` (`Extracted`/`Inferred`), and an `inferred` boolean. A fixed
///   relationship type avoids having to escape edge labels (e.g.
///   `cross_language:HttpCall`) into Cypher relationship-type identifiers; the
///   honest type label lives in the `type` property instead.
///
/// Determinism: nodes come from [`collect_graph_nodes`] (sorted); relationships
/// iterate the `BTreeMap`/`BTreeSet` adjacency in sorted order.
pub fn to_cypher(graph: &DependencyGraph, repo_name: &str) -> String {
    let nodes = collect_graph_nodes(graph);
    let rel_count = graph.edge_count();

    let mut out = format!(
        "// cxpak dependency graph export — repo: {}\n// nodes: {}, relationships: {}\n",
        escape_cypher(repo_name),
        nodes.len(),
        rel_count,
    );

    for id in &nodes {
        out.push_str(&format!(
            "MERGE (:{} {{id: '{}'}});\n",
            node_kind(id),
            escape_cypher(id),
        ));
    }

    for (source, targets) in &graph.edges {
        for edge in targets {
            out.push_str(&format!(
                "MATCH (a {{id: '{src}'}}), (b {{id: '{dst}'}}) MERGE (a)-[:DEPENDS_ON {{type: '{ty}', confidence: '{conf}', inferred: {inf}}}]->(b);\n",
                src = escape_cypher(source),
                dst = escape_cypher(&edge.target),
                ty = escape_cypher(&edge.edge_type.label()),
                conf = confidence_str(edge.confidence),
                inf = edge.confidence.is_inferred(),
            ));
        }
    }

    out
}

/// Serialize a [`DependencyGraph`] as well-formed GraphML (plain XML).
///
/// Shape: `<graphml>` → `<key>` declarations for the node `kind` and the edge
/// `type` / `confidence` / `inferred` attributes → `<graph edgedefault="directed">`
/// → sorted `<node>`s then sorted `<edge>`s, each carrying its `<data>` children.
/// Every id/value is XML-escaped via the module's [`xml_escape`] helper (shared
/// with the SVG exporter — no new dependency; GraphML is plain XML).
///
/// Determinism: nodes from [`collect_graph_nodes`] (sorted); edges iterate the
/// sorted adjacency and are assigned sequential `e{n}` ids in that order.
pub fn to_graphml(graph: &DependencyGraph, repo_name: &str) -> String {
    let nodes = collect_graph_nodes(graph);

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">\n");
    out.push_str(&format!("  <!-- repo: {} -->\n", xml_escape(repo_name)));
    out.push_str("  <key id=\"d_kind\" for=\"node\" attr.name=\"kind\" attr.type=\"string\"/>\n");
    out.push_str("  <key id=\"d_type\" for=\"edge\" attr.name=\"type\" attr.type=\"string\"/>\n");
    out.push_str(
        "  <key id=\"d_confidence\" for=\"edge\" attr.name=\"confidence\" attr.type=\"string\"/>\n",
    );
    out.push_str(
        "  <key id=\"d_inferred\" for=\"edge\" attr.name=\"inferred\" attr.type=\"boolean\"/>\n",
    );
    out.push_str("  <graph id=\"G\" edgedefault=\"directed\">\n");

    for id in &nodes {
        out.push_str(&format!(
            "    <node id=\"{id}\"><data key=\"d_kind\">{kind}</data></node>\n",
            id = xml_escape(id),
            kind = node_kind(id),
        ));
    }

    let mut edge_index = 0usize;
    for (source, targets) in &graph.edges {
        for edge in targets {
            out.push_str(&format!(
                "    <edge id=\"e{idx}\" source=\"{src}\" target=\"{dst}\"><data key=\"d_type\">{ty}</data><data key=\"d_confidence\">{conf}</data><data key=\"d_inferred\">{inf}</data></edge>\n",
                idx = edge_index,
                src = xml_escape(source),
                dst = xml_escape(&edge.target),
                ty = xml_escape(&edge.edge_type.label()),
                conf = confidence_str(edge.confidence),
                inf = edge.confidence.is_inferred(),
            ));
            edge_index += 1;
        }
    }

    out.push_str("  </graph>\n");
    out.push_str("</graphml>\n");
    out
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
                    aria_label: String::new(),
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
                    aria_label: String::new(),
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
                    aria_label: String::new(),
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

    // ── Mermaid label bracket/pipe escape regression (46ced99) ──────────────
    //
    // Bug: node labels containing `[`, `]`, or `|` were emitted verbatim into
    // the Mermaid output.  Both `[`/`]` are Mermaid node-shape syntax and `|`
    // is flow-chart syntax, causing parse errors in Mermaid renderers.
    //
    // The test would FAIL against the pre-fix code because to_mermaid() would
    // write the literal `[bracket]` in the label, which Mermaid parsers treat
    // as a nested node definition rather than label content.

    #[test]
    fn test_to_mermaid_escapes_brackets_in_label() {
        let layout = ComputedLayout {
            nodes: vec![LayoutNode {
                id: "src/api".into(),
                // Label with square brackets and pipe that would break Mermaid.
                label: "[Handler|Route]".into(),
                layer: 0,
                position: Point { x: 0.0, y: 0.0 },
                width: 160.0,
                height: 48.0,
                node_type: NodeType::Module,
                metadata: NodeMetadata::default(),
                aria_label: String::new(),
            }],
            edges: vec![],
            width: 200.0,
            height: 100.0,
            layers: vec![vec!["src/api".into()]],
        };
        let mermaid = to_mermaid(&layout);

        // Raw `[`, `]`, and `|` must not appear inside the label string.
        // The fix maps `[` → `(`, `]` → `)`, `|` → `/`.
        assert!(
            !mermaid.contains("[Handler|Route]"),
            "raw brackets and pipe in label must be escaped; \
             if present, the 46ced99 fix has been reverted.\nGot:\n{mermaid}"
        );
        // The sanitised form must appear.
        assert!(
            mermaid.contains("(Handler/Route)"),
            "label must be sanitised to '(Handler/Route)', got:\n{mermaid}"
        );
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
    fn test_c4_containers_are_bound() {
        let layout = make_3_module_layout();
        let meta = make_test_meta();
        let c4 = to_c4(&layout, &meta);
        // Every container must be bound to a cN variable.
        assert!(c4.contains("c0 = container"), "c0 binding missing:\n{c4}");
        assert!(c4.contains("c1 = container"), "c1 binding missing:\n{c4}");
        assert!(c4.contains("c2 = container"), "c2 binding missing:\n{c4}");
        // Relationships must use numeric variable names.
        assert!(
            c4.contains("c0 -> c1") || c4.contains("c0 -> c2"),
            "c0 relationship missing:\n{c4}"
        );
    }

    #[test]
    fn test_c4_adversarial_repo_name() {
        // Repo name containing `"`, `{`, `}` must produce valid DSL (no stray chars).
        let layout = make_3_module_layout();
        let mut meta = make_test_meta();
        meta.repo_name = r#"foo"bar{baz}"#.to_string();
        let c4 = to_c4(&layout, &meta);

        // The softwareSystem line must not contain unescaped `"` after the
        // opening one, or unbalanced `{`/`}`.
        let sys_line = c4
            .lines()
            .find(|l| l.contains("softwareSystem"))
            .expect("softwareSystem line must be present");
        // After stripping the DSL keyword and structural chars, no `"` or `{` or `}` from the name.
        // The name portion is between the first and last `"` on that line.
        let between_quotes: Vec<&str> = sys_line.splitn(3, '"').collect();
        assert_eq!(
            between_quotes.len(),
            3,
            "softwareSystem line must have exactly one quoted string: {sys_line}"
        );
        let name_content = between_quotes[1];
        assert!(
            !name_content.contains('"'),
            "repo name must not contain unescaped quotes: {name_content}"
        );
        assert!(
            !name_content.contains('{') && !name_content.contains('}'),
            "repo name must not contain braces: {name_content}"
        );
    }

    #[test]
    fn test_escape_c4_chars() {
        assert_eq!(escape_c4(r#"foo"bar"#), "foo bar");
        assert_eq!(escape_c4("a{b}c"), "a b c");
        assert_eq!(escape_c4("normal"), "normal");
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
                aria_label: String::new(),
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
                aria_label: String::new(),
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
                aria_label: String::new(),
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
                aria_label: String::new(),
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

    // ── Graph-serialization exports: Cypher + GraphML ──────────────────────────

    use crate::core_graph::graph::{DependencyGraph, EdgeConfidence, EdgeType};

    /// A small fixture graph mixing an Extracted file→file import, an Inferred
    /// embedded-SQL edge, and a synthetic `col:` column node — enough to exercise
    /// node labeling, edge type/confidence metadata, and canonical ordering.
    fn make_fixture_graph() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        // Extracted import: api → db.
        g.add_edge("src/api.rs", "src/db.rs", EdgeType::Import);
        // Inferred embedded-SQL edge into a synthetic column node.
        g.add_edge("src/db.rs", "col:users.id", EdgeType::EmbeddedSql);
        // Structurally-extracted column→table anchor.
        g.add_edge_with_confidence(
            "col:users.id",
            "src/schema.sql",
            EdgeType::ColumnReference,
            EdgeConfidence::Extracted,
        );
        g
    }

    #[test]
    fn test_to_cypher_emits_nodes_and_relationships() {
        let g = make_fixture_graph();
        let cypher = to_cypher(&g, "demo-repo");

        // Header comment names the repo.
        assert!(
            cypher.contains("// cxpak dependency graph export — repo: demo-repo"),
            "header missing:\n{cypher}"
        );
        // Nodes are MERGE'd, labeled by kind.
        assert!(
            cypher.contains("MERGE (:File {id: 'src/api.rs'});"),
            "file node missing:\n{cypher}"
        );
        assert!(
            cypher.contains("MERGE (:Column {id: 'col:users.id'});"),
            "column node missing:\n{cypher}"
        );
        // Relationships carry type + confidence + inferred flag.
        assert!(
            cypher.contains("MATCH (a {id: 'src/api.rs'}), (b {id: 'src/db.rs'}) MERGE (a)-[:DEPENDS_ON {type: 'import', confidence: 'Extracted', inferred: false}]->(b);"),
            "extracted import relationship missing:\n{cypher}"
        );
        assert!(
            cypher.contains("type: 'embedded_sql', confidence: 'Inferred', inferred: true"),
            "inferred embedded-sql relationship metadata missing:\n{cypher}"
        );
    }

    #[test]
    fn test_to_cypher_escapes_quote_and_backslash_in_path() {
        // A path holding a single quote, a backslash, and a newline must not break
        // the literal — this is the injection/correctness boundary.
        let mut g = DependencyGraph::new();
        g.add_edge("src/weird'\\\n.rs", "src/db.rs", EdgeType::Import);
        let cypher = to_cypher(&g, "r");

        // The escaped node literal must appear with \' , \\ , and \n.
        assert!(
            cypher.contains("MERGE (:File {id: 'src/weird\\'\\\\\\n.rs'});"),
            "adversarial path not correctly escaped:\n{cypher}"
        );
        // No raw newline may appear inside any statement: every line that opens a
        // MERGE/MATCH statement must also terminate it with `;` on the same line.
        for line in cypher.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("MERGE") || trimmed.starts_with("MATCH") {
                assert!(
                    line.trim_end().ends_with(';'),
                    "statement split across lines (raw control char leaked): {line:?}"
                );
            }
        }
    }

    #[test]
    fn test_to_cypher_is_byte_deterministic() {
        let g = make_fixture_graph();
        let a = to_cypher(&g, "demo-repo");
        let b = to_cypher(&g, "demo-repo");
        assert_eq!(a, b, "cypher export must be byte-identical across runs");
    }

    #[test]
    fn test_to_cypher_every_relationship_endpoint_is_a_declared_node() {
        // Structural validity proxy (no Neo4j): every id referenced in a MATCH
        // relationship must have been MERGE'd as a node.
        let g = make_fixture_graph();
        let cypher = to_cypher(&g, "r");

        let declared: BTreeSet<String> = cypher
            .lines()
            .filter_map(|l| {
                let l = l.trim_start();
                // `MERGE (:Kind {id: '...'});`
                l.strip_prefix("MERGE (:")?;
                let start = l.find("{id: '")? + "{id: '".len();
                let rest = &l[start..];
                let end = rest.find("'}")?;
                Some(rest[..end].to_string())
            })
            .collect();

        let mut rel_endpoints = 0usize;
        for l in cypher.lines() {
            if let Some(rest) = l.trim_start().strip_prefix("MATCH (a {id: '") {
                let src_end = rest.find("'}").expect("malformed MATCH src");
                let src = &rest[..src_end];
                let after = &rest[src_end..];
                let dst_start = after.find("(b {id: '").expect("missing dst") + "(b {id: '".len();
                let dst_rest = &after[dst_start..];
                let dst_end = dst_rest.find("'}").expect("malformed MATCH dst");
                let dst = &dst_rest[..dst_end];
                assert!(declared.contains(src), "undeclared src node: {src}");
                assert!(declared.contains(dst), "undeclared dst node: {dst}");
                rel_endpoints += 1;
            }
        }
        assert_eq!(
            rel_endpoints,
            g.edge_count(),
            "every edge must produce exactly one relationship statement"
        );
    }

    #[test]
    fn test_to_graphml_is_well_formed_with_keys_and_data() {
        let g = make_fixture_graph();
        let xml = to_graphml(&g, "demo-repo");

        assert!(
            xml.starts_with("<?xml version=\"1.0\""),
            "xml prolog missing"
        );
        assert!(xml.contains("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">"));
        // Key declarations for node kind + edge type/confidence/inferred.
        assert!(xml.contains("<key id=\"d_kind\" for=\"node\""));
        assert!(xml.contains("<key id=\"d_type\" for=\"edge\""));
        assert!(xml.contains("<key id=\"d_confidence\" for=\"edge\""));
        assert!(xml.contains("<key id=\"d_inferred\" for=\"edge\""));
        // Nodes carry kind data.
        assert!(xml.contains("<node id=\"col:users.id\"><data key=\"d_kind\">Column</data></node>"));
        assert!(xml.contains("<node id=\"src/api.rs\"><data key=\"d_kind\">File</data></node>"));
        // Edge carries type + confidence + inferred.
        assert!(
            xml.contains("<data key=\"d_type\">import</data><data key=\"d_confidence\">Extracted</data><data key=\"d_inferred\">false</data>"),
            "edge data missing:\n{xml}"
        );
        assert!(xml.trim_end().ends_with("</graphml>"));
    }

    #[test]
    fn test_to_graphml_structural_well_formedness() {
        // No XML-parser dependency: assert structural well-formedness by counting
        // balanced open/close tags for the elements we emit.
        let g = make_fixture_graph();
        let xml = to_graphml(&g, "demo-repo");

        let opens = xml.matches("<node ").count();
        let closes = xml.matches("</node>").count();
        assert_eq!(opens, closes, "unbalanced <node> tags");
        let eopens = xml.matches("<edge ").count();
        let ecloses = xml.matches("</edge>").count();
        assert_eq!(eopens, ecloses, "unbalanced <edge> tags");
        assert_eq!(eopens, g.edge_count(), "one <edge> per graph edge");
        // Exactly one <graph> and one <graphml> wrapper.
        assert_eq!(xml.matches("<graph ").count(), 1);
        assert_eq!(xml.matches("</graph>").count(), 1);
        assert_eq!(xml.matches("</graphml>").count(), 1);
    }

    #[test]
    fn test_to_graphml_xml_escapes_adversarial_node_id() {
        let mut g = DependencyGraph::new();
        g.add_edge("src/a<b>&\"c.rs", "src/db.rs", EdgeType::Import);
        let xml = to_graphml(&g, "r");
        // Raw angle brackets / ampersand / quote must be escaped inside the id.
        assert!(
            xml.contains("<node id=\"src/a&lt;b&gt;&amp;&quot;c.rs\">"),
            "adversarial node id not XML-escaped:\n{xml}"
        );
        assert!(
            !xml.contains("a<b>&\"c.rs"),
            "raw unescaped chars leaked into XML:\n{xml}"
        );
    }

    #[test]
    fn test_to_graphml_strips_xml_illegal_control_chars() {
        // A git-tracked path can legally contain C0 control bytes on Unix.
        // They are illegal in XML 1.0 and must not reach the output; tab/LF/CR
        // are legal and must survive.
        let mut g = DependencyGraph::new();
        g.add_edge(
            "src/a\u{01}b\u{0B}c\u{1F}\td.rs",
            "src/db.rs",
            EdgeType::Import,
        );
        let xml = to_graphml(&g, "r");
        for illegal in ['\u{01}', '\u{0B}', '\u{1F}'] {
            assert!(
                !xml.contains(illegal),
                "XML-illegal control char {:#04x} leaked into graphml output",
                illegal as u32
            );
        }
        // The three C0 controls are removed; the legal tab survives.
        assert!(
            xml.contains("<node id=\"src/abc\td.rs\">"),
            "control chars not stripped as expected:\n{xml}"
        );
    }

    #[test]
    fn test_to_graphml_is_byte_deterministic() {
        let g = make_fixture_graph();
        let a = to_graphml(&g, "demo-repo");
        let b = to_graphml(&g, "demo-repo");
        assert_eq!(a, b, "graphml export must be byte-identical across runs");
    }

    #[test]
    fn test_graph_exports_handle_empty_graph() {
        let g = DependencyGraph::new();
        let cypher = to_cypher(&g, "empty");
        assert!(cypher.contains("nodes: 0, relationships: 0"));
        let xml = to_graphml(&g, "empty");
        assert!(xml.contains("<graph id=\"G\""));
        assert!(xml.trim_end().ends_with("</graphml>"));
    }
}
