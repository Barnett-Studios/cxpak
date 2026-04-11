# v2.0.0 "The Experience" Implementation Plan — Part 1 (Tasks 1-15)
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development

**Goal:** Visual intelligence dashboard (6 views), multi-format export, CLI + MCP tools.
**Architecture:** Layout pre-computed in Rust (simplified Sugiyama). Self-contained HTML with custom D3 bundle (~100KB) inlined. PNG via resvg. No Sigma.js.
**Tech Stack:** Rust, D3.js (custom bundle), petgraph, resvg, serde

---

## Prerequisites

All v1.2.0–v1.6.0 intelligence types must be available on `CodebaseIndex`:
- `health: HealthScore` (v1.2.0)
- `risks: Vec<RiskEntry>` (v1.2.0)
- `architecture: ArchitectureMap` (v1.2.0)
- `dead_code: Vec<DeadSymbol>` (v1.3.0)
- `drift: Option<DriftReport>` (v1.4.0)
- `security: SecuritySurface` (v1.4.0)
- `data_flows: Vec<DataFlowResult>` (v1.5.0)

If building v2.0.0 before those versions ship, stub the missing fields with `Default` impls.

---

## Task 1 — Feature Flags: `visual` and `plugins` in Cargo.toml

**Files:** `Cargo.toml`

**Steps:**

1. Write test: `cargo check --features visual` must succeed and `resvg` must be in the dep tree.
2. Add `resvg` as optional dependency.
3. Add `wasmtime` as optional dependency for `plugins`.
4. Add feature definitions and include both in `default`.
5. Verify: `cargo check --features visual`, `cargo check --no-default-features --features visual`.

**Code:**

```toml
[dependencies]
resvg = { version = "0.44", optional = true }
wasmtime = { version = "28", optional = true }

[features]
default = [
    # ... existing features ...
    "daemon",
    "embeddings",
    "visual",
    "plugins",
]
visual = ["dep:resvg"]
plugins = ["dep:wasmtime"]
```

**Note:** `resvg 0.44` adds ~2MB to binary. `wasmtime 28` is the LTS-aligned release as of early 2026 — verify latest stable before pinning. Both are gated so users can opt out: `cargo build --no-default-features --features daemon,embeddings`.

**Commands:**
```bash
cargo check --features visual
cargo check --features plugins
cargo check --no-default-features --features "lang-rust,daemon,embeddings"
```

---

## Task 2 — Visual Module Scaffold

**Files:**
- `src/visual/mod.rs`
- `src/visual/layout.rs`
- `src/visual/render.rs`
- `src/visual/export.rs`
- `src/visual/onboard.rs`
- `src/main.rs` (add `#[cfg(feature = "visual")] pub mod visual;`)

**Steps:**

1. Write test: `src/visual/mod.rs` re-exports compile with `--features visual`.
2. Create `src/visual/mod.rs` with public re-exports and core types.
3. Add `#[cfg(feature = "visual")] pub mod visual;` to `src/main.rs`.
4. Create stub files for `layout.rs`, `render.rs`, `export.rs`, `onboard.rs`.
5. Verify: `cargo build --features visual` with no warnings.

**Core types in `src/visual/mod.rs`:**

```rust
#[cfg(feature = "visual")]
pub mod layout;
#[cfg(feature = "visual")]
pub mod render;
#[cfg(feature = "visual")]
pub mod export;
#[cfg(feature = "visual")]
pub mod onboard;

/// Which visualization type to generate
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VisualType {
    Dashboard,
    Architecture,
    Risk,
    Flow,
    Timeline,
    Diff,
}

/// Output format for a visualization
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VisualFormat {
    Html,
    Mermaid,
    Svg,
    #[cfg(feature = "visual")]
    Png,
    C4,
    Json,
}

/// The computed result of a visualization — before serialization to a format
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VisualOutput {
    pub visual_type: VisualType,
    pub format: VisualFormat,
    /// File path written, or None if returned inline
    pub path: Option<String>,
    /// Inline content (HTML string, Mermaid source, SVG, JSON)
    pub content: Option<String>,
}
```

---

## Task 3 — Sugiyama Layout Engine: Types and Topological Sort

**Files:** `src/visual/layout.rs`

**Steps:**

1. Write test: `layer_assign` on a DAG with 5 nodes returns layers with no forward-edge violations.
2. Implement `LayoutGraph` wrapper over petgraph `DiGraph`.
3. Implement `layer_assign()` — longest-path layering via topological sort.
4. Write test: cycle-containing graph returns `Err(LayoutError::Cyclic)`.
5. Handle SCCs: condense cycles into single virtual nodes before layering.

**Types:**

```rust
use std::collections::HashMap;

/// A node in the layout graph — maps to a file, module, or symbol
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutNode {
    pub id: String,
    pub label: String,
    pub layer: usize,
    pub position: Point,
    pub width: f64,
    pub height: f64,
    pub node_type: NodeType,
    pub metadata: NodeMetadata,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum NodeType {
    Module,
    File,
    Symbol,
    /// Virtual node representing a condensed SCC
    Cluster { member_ids: Vec<String> },
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NodeMetadata {
    pub pagerank: f64,
    pub risk_score: f64,
    pub token_count: usize,
    pub health_score: Option<f64>,
    pub is_god_file: bool,
    pub has_dead_code: bool,
    pub is_circular: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutEdge {
    pub source: String,
    pub target: String,
    pub edge_type: EdgeVisualType,
    pub weight: f64,
    /// True when this edge participates in a cycle
    pub is_cycle: bool,
    /// Waypoints for edges that route through dummy nodes
    pub waypoints: Vec<Point>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EdgeVisualType {
    Import,
    Call,
    Schema,
    CrossLanguage,
    CoChange,
    DataFlow,
}

/// Fully computed layout — positions ready for D3 rendering
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ComputedLayout {
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
    pub width: f64,
    pub height: f64,
    pub layers: Vec<Vec<String>>,  // node ids per layer
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("graph is fully cyclic and cannot be layered")]
    Cyclic,
    #[error("node not found: {0}")]
    NodeNotFound(String),
    #[error("empty graph")]
    Empty,
}

/// Entry point — computes full Sugiyama layout
pub fn compute_layout(
    nodes: Vec<LayoutNode>,
    edges: Vec<LayoutEdge>,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError>;

#[derive(Debug, Clone)]
pub struct LayoutConfig {
    pub layer_sep: f64,      // vertical gap between layers (default: 120.0)
    pub node_sep: f64,       // horizontal gap between nodes in same layer (default: 60.0)
    pub node_width: f64,     // default node width (default: 160.0)
    pub node_height: f64,    // default node height (default: 48.0)
    pub max_nodes_per_layer: usize,  // default: 9 (7±2)
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            layer_sep: 120.0,
            node_sep: 60.0,
            node_width: 160.0,
            node_height: 48.0,
            max_nodes_per_layer: 9,
        }
    }
}
```

**Key function signature for layer assignment:**

```rust
/// Assigns each node a layer index via longest-path layering.
/// Condenses SCCs to virtual nodes first so the input DAG is always acyclic.
pub(crate) fn layer_assign(
    node_ids: &[String],
    edges: &[(String, String)],
) -> Result<HashMap<String, usize>, LayoutError>;
```

**Tests:**
```rust
#[test]
fn test_layer_assign_linear_chain() {
    // a -> b -> c => layers [0, 1, 2]
    let layers = layer_assign(
        &["a".into(), "b".into(), "c".into()],
        &[("a".into(), "b".into()), ("b".into(), "c".into())],
    ).unwrap();
    assert_eq!(layers["a"], 0);
    assert_eq!(layers["b"], 1);
    assert_eq!(layers["c"], 2);
}

#[test]
fn test_layer_assign_diamond() {
    // a -> b, a -> c, b -> d, c -> d
    // d must be in layer >= 2
    let layers = layer_assign(
        &["a".into(), "b".into(), "c".into(), "d".into()],
        &[
            ("a".into(), "b".into()), ("a".into(), "c".into()),
            ("b".into(), "d".into()), ("c".into(), "d".into()),
        ],
    ).unwrap();
    assert!(layers["d"] >= 2);
    assert!(layers["b"] > layers["a"]);
    assert!(layers["c"] > layers["a"]);
}

#[test]
fn test_layer_assign_handles_cycle_via_scc_condensation() {
    // a -> b -> a (cycle) — condensed to virtual node, does not error
    let result = layer_assign(
        &["a".into(), "b".into()],
        &[("a".into(), "b".into()), ("b".into(), "a".into())],
    );
    assert!(result.is_ok());
}
```

---

## Task 4 — Sugiyama Layout Engine: Crossing Minimization and Coordinate Assignment

**Files:** `src/visual/layout.rs` (continued)

**Steps:**

1. Write test: two parallel 3-node chains, `barycenter_sort` should not swap nodes that start in optimal order.
2. Implement `insert_dummy_nodes()` — adds virtual nodes on edges spanning >1 layer for Brandes-Kopf.
3. Implement `barycenter_sort()` — one-sided barycenter heuristic, 2-pass (top-down then bottom-up), 4 iterations.
4. Write test: `assign_coordinates` places all nodes with no horizontal overlaps within the same layer.
5. Implement `assign_coordinates()` — Brandes-Kopf simplified: align to median neighbor, then compact.
6. Write test: `compute_layout` on the cxpak dependency graph (loaded from fixtures) completes in under 2s.

**Key function signatures:**

```rust
/// Inserts virtual (dummy) nodes on edges spanning multiple layers.
/// Returns augmented node list, augmented edge list, and dummy node ids.
pub(crate) fn insert_dummy_nodes(
    nodes: &[LayoutNode],
    edges: &[LayoutEdge],
    layers: &HashMap<String, usize>,
) -> (Vec<LayoutNode>, Vec<LayoutEdge>, Vec<String>);

/// One-sided barycenter crossing minimization.
/// Mutates layer ordering in place; 4 passes alternating top-down/bottom-up.
pub(crate) fn barycenter_sort(
    layer_order: &mut Vec<Vec<String>>,
    adjacency: &HashMap<String, Vec<String>>,
    reverse_adjacency: &HashMap<String, Vec<String>>,
);

/// Brandes-Kopf simplified coordinate assignment.
/// Returns x,y for each node id. y is determined by layer × layer_sep.
pub(crate) fn assign_coordinates(
    layer_order: &[Vec<String>],
    config: &LayoutConfig,
) -> HashMap<String, Point>;
```

**Tests:**
```rust
#[test]
fn test_assign_coordinates_no_horizontal_overlaps() {
    let layer_order = vec![
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        vec!["d".to_string(), "e".to_string()],
    ];
    let config = LayoutConfig::default();
    let coords = assign_coordinates(&layer_order, &config);
    // nodes in same layer must not overlap
    let x_a = coords["a"].x;
    let x_b = coords["b"].x;
    let x_c = coords["c"].x;
    assert!(x_b > x_a + config.node_width);
    assert!(x_c > x_b + config.node_width);
}

#[test]
fn test_compute_layout_produces_valid_positions() {
    let (nodes, edges) = make_test_graph_5_nodes();
    let layout = compute_layout(nodes, edges, &LayoutConfig::default()).unwrap();
    // every node has been assigned a position
    assert_eq!(layout.nodes.len(), 5);
    for node in &layout.nodes {
        assert!(node.position.x >= 0.0);
        assert!(node.position.y >= 0.0);
    }
    // overall dimensions are positive
    assert!(layout.width > 0.0);
    assert!(layout.height > 0.0);
}
```

---

## Task 5 — Layout Builders: Module Graph, File Graph, Symbol Graph

**Files:** `src/visual/layout.rs` (continued)

**Steps:**

1. Write test: `build_module_layout` on a 3-module index produces exactly 3 non-cluster nodes (one per module).
2. Implement `build_module_layout()` — walks `ArchitectureMap`, creates one `LayoutNode` per module, edges from cross-module imports. Node width scaled by `aggregate_pagerank`. Color metadata from health sub-score.
3. Write test: `build_file_layout` for a single module returns nodes with `risk_score` populated from `RiskEntry`.
4. Implement `build_file_layout(module_prefix)` — files within a module. Size by token count, color by risk score. God files set `is_god_file: true`. Dead code files set `has_dead_code: true`.
5. Write test: `build_symbol_layout` for a file with 5 symbols returns 5 nodes, each with `pagerank` set.
6. Implement `build_symbol_layout(file_path)` — symbols within a file. Call graph edges. Convention violations tagged in metadata.

**Key function signatures:**

```rust
use crate::index::CodebaseIndex;

/// Level 1: one node per module
pub fn build_module_layout(
    index: &CodebaseIndex,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError>;

/// Level 2: files within a specific module prefix
pub fn build_file_layout(
    index: &CodebaseIndex,
    module_prefix: &str,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError>;

/// Level 3: symbols within a file, with call-graph edges
pub fn build_symbol_layout(
    index: &CodebaseIndex,
    file_path: &str,
    config: &LayoutConfig,
) -> Result<ComputedLayout, LayoutError>;
```

**Cognitive load constraint enforcement:**

```rust
/// If a layer would exceed max_nodes_per_layer, group the tail into a Cluster node.
/// The cluster node carries the member_ids for expansion on click.
fn enforce_cognitive_limit(
    nodes: Vec<LayoutNode>,
    layer_order: &mut Vec<Vec<String>>,
    max_per_layer: usize,
) -> Vec<LayoutNode>;
```

---

## Task 6 — HTML Template System and D3 Asset

**Files:**
- `src/visual/render.rs`
- `assets/d3-bundle.min.js` (pre-built, committed to repo)
- `assets/cxpak-visual.css` (minimal stylesheet)

**Steps:**

1. Download/build the custom D3 bundle (d3-hierarchy, d3-zoom, d3-transition, d3-scale, d3-selection, d3-shape, d3-color, d3-interpolate). Target: ~100KB minified. Commit to `assets/`. This is a one-time asset build, not part of the Rust build.
2. Write test: `render_html(layout, VisualType::Dashboard)` returns a string containing `<!DOCTYPE html>`, `<script>`, and the JSON-serialized layout data.
3. Implement `render_html()` — inlines d3-bundle via `include_str!`, serializes layout as JSON, emits self-contained HTML.
4. Write test: the emitted HTML is valid (contains `</html>`, has no unclosed `<script>` tags).
5. Write test: layout JSON is valid — `serde_json::from_str` on the embedded data succeeds.
6. Write test: total HTML size for a 50-node layout is under 500KB.

**Key function signatures in `src/visual/render.rs`:**

```rust
static D3_BUNDLE: &str = include_str!("../../assets/d3-bundle.min.js");
static VISUAL_CSS: &str = include_str!("../../assets/cxpak-visual.css");

/// Renders a self-contained HTML file. All JS/CSS inlined.
/// The layout data is JSON-serialized into a <script id="cxpak-data"> tag.
pub fn render_html(
    layout: &ComputedLayout,
    visual_type: VisualType,
    metadata: &RenderMetadata,
) -> String;

/// Metadata injected into the HTML for display (repo name, health score, etc.)
#[derive(Debug, serde::Serialize)]
pub struct RenderMetadata {
    pub repo_name: String,
    pub generated_at: String,
    pub health_score: Option<f64>,
    pub node_count: usize,
    pub edge_count: usize,
    pub cxpak_version: String,
}
```

**HTML skeleton (what `render_html` produces — not the full template, just the structure):**

```rust
// The function emits this structure:
// <!DOCTYPE html>
// <html lang="en">
// <head><meta charset="utf-8"><title>cxpak — {visual_type}</title>
//   <style>{VISUAL_CSS}</style>
// </head>
// <body>
//   <div id="cxpak-app"></div>
//   <script id="cxpak-data" type="application/json">{layout_json}</script>
//   <script id="cxpak-meta" type="application/json">{meta_json}</script>
//   <script>{D3_BUNDLE}</script>
//   <script>{view_controller_js}</script>  <!-- inline, per visual_type -->
// </body>
// </html>
```

The `view_controller_js` is a small inline script (~20-50 lines) that reads `cxpak-data` and `cxpak-meta` then calls the appropriate D3 render function. It is embedded as a Rust string literal per `VisualType`, not from a separate file.

**Tests:**
```rust
#[test]
fn test_render_html_is_self_contained() {
    let layout = make_test_layout_5_nodes();
    let meta = RenderMetadata { /* ... */ };
    let html = render_html(&layout, VisualType::Dashboard, &meta);
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("</html>"));
    assert!(html.contains("cxpak-data"));
    assert!(!html.contains("cdn.jsdelivr.net"));
    assert!(!html.contains("unpkg.com"));
}

#[test]
fn test_render_html_layout_json_is_valid() {
    let layout = make_test_layout_5_nodes();
    let meta = RenderMetadata { /* ... */ };
    let html = render_html(&layout, VisualType::Architecture, &meta);
    // Extract JSON from the data script tag
    let start = html.find(r#"<script id="cxpak-data""#).unwrap();
    let json_start = html[start..].find('>').unwrap() + start + 1;
    let json_end = html[json_start..].find("</script>").unwrap() + json_start;
    let json_str = &html[json_start..json_end];
    let _parsed: serde_json::Value = serde_json::from_str(json_str)
        .expect("layout JSON must be valid");
}
```

---

## Task 7 — Dashboard View

**Files:** `src/visual/render.rs` (continued)

**Steps:**

1. Write test: `render_dashboard` with a `DashboardData` input returns HTML containing the strings `"health-quadrant"`, `"risks-quadrant"`, `"architecture-quadrant"`, `"alerts-quadrant"`.
2. Define `DashboardData` struct aggregating all four quadrant inputs from `CodebaseIndex`.
3. Implement `build_dashboard_data()` — extracts health score, top-5 risks, module graph preview data, and alerts from `CodebaseIndex`.
4. Implement `render_dashboard()` — calls `render_html` with dashboard-specific view controller JS.
5. Write test: `build_dashboard_data` with a stub index containing 0 risks produces a `DashboardData` with `top_risks.is_empty()`.
6. Write test: alerts list includes `"circular_dependency"` when `architecture.circular_deps` is non-empty.

**Types:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct DashboardData {
    pub health: HealthQuadrant,
    pub risks: RisksQuadrant,
    pub architecture_preview: ArchitecturePreviewQuadrant,
    pub alerts: AlertsQuadrant,
}

#[derive(Debug, serde::Serialize)]
pub struct HealthQuadrant {
    pub composite: f64,
    pub dimensions: Vec<(String, f64)>,   // ("conventions", 8.2), ...
    /// Sparkline: (timestamp_label, composite_score) pairs from snapshots
    pub trend: Vec<(String, f64)>,
}

#[derive(Debug, serde::Serialize)]
pub struct RisksQuadrant {
    pub top_risks: Vec<RiskDisplayEntry>,  // top 5 only
}

#[derive(Debug, serde::Serialize)]
pub struct RiskDisplayEntry {
    pub path: String,
    pub risk_score: f64,
    pub churn_30d: u32,
    pub blast_radius: usize,
    pub has_tests: bool,
    /// "high" | "medium" | "low" — derived from risk_score thresholds
    pub severity: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ArchitecturePreviewQuadrant {
    /// Simplified layout: just module nodes + edge counts (no file-level detail)
    pub layout: ComputedLayout,
    pub module_count: usize,
    pub circular_dep_count: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct AlertsQuadrant {
    pub alerts: Vec<Alert>,
}

#[derive(Debug, serde::Serialize)]
pub struct Alert {
    pub kind: AlertKind,
    pub message: String,
    pub severity: AlertSeverity,
    /// Link target: which view to navigate to on click
    pub link_view: VisualType,
}

#[derive(Debug, serde::Serialize)]
pub enum AlertKind {
    CircularDependency,
    DeadSymbols,
    UnprotectedEndpoints,
    CouplingTrend,
    HighRiskFile,
}

#[derive(Debug, serde::Serialize)]
pub enum AlertSeverity { High, Medium, Low }

pub fn build_dashboard_data(index: &CodebaseIndex) -> DashboardData;

pub fn render_dashboard(index: &CodebaseIndex, metadata: &RenderMetadata) -> String;
```

---

## Task 8 — Architecture Explorer View (3-Level Semantic Zoom)

**Files:** `src/visual/render.rs` (continued)

**Steps:**

1. Write test: `build_architecture_explorer_data` on a 3-module index returns a struct with `level1.nodes.len() == 3`.
2. Implement `build_architecture_explorer_data()` — precomputes all 3 layout levels and embeds them in a single JSON payload.
3. Write test: level2 data for module `"src/index"` contains only files with path prefix `"src/index"`.
4. Write test: level3 data for a file with 8 symbols caps at 9 nodes (7±2 enforcement with 1 "others" cluster).
5. Implement `render_architecture_explorer()` — calls `render_html` with architecture view controller.
6. Write test: the breadcrumb data structure is present in the JSON payload (`"breadcrumbs"` key).

**Types:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct ArchitectureExplorerData {
    /// Level 1: module graph layout
    pub level1: ComputedLayout,
    /// Level 2: per-module file layouts, keyed by module prefix
    pub level2: std::collections::HashMap<String, ComputedLayout>,
    /// Level 3: per-file symbol layouts, keyed by file path
    /// Only populated for files with >3 symbols (lazy: populated on first click in browser)
    /// For static export, populated for top-20 files by PageRank
    pub level3: std::collections::HashMap<String, ComputedLayout>,
    pub initial_level: u8,
    pub breadcrumbs: Vec<BreadcrumbEntry>,
}

#[derive(Debug, serde::Serialize)]
pub struct BreadcrumbEntry {
    pub label: String,
    pub level: u8,
    pub target_id: String,
}

pub fn build_architecture_explorer_data(
    index: &CodebaseIndex,
    config: &LayoutConfig,
) -> Result<ArchitectureExplorerData, LayoutError>;

pub fn render_architecture_explorer(
    index: &CodebaseIndex,
    metadata: &RenderMetadata,
) -> Result<String, LayoutError>;
```

**Level 3 lazy population strategy:** The HTML payload embeds level1 and level2 always. Level 3 layouts are embedded only for the top-20 files by PageRank. The view controller JS shows a loading spinner for files not pre-computed, then POSTs to `/v1/visual` (if served) or displays "open in interactive mode" if file:// context.

---

## Task 9 — Risk Heatmap View (Treemap)

**Files:** `src/visual/render.rs` (continued)

**Steps:**

1. Write test: `build_risk_heatmap_data` returns a treemap where all `RiskCell.area_value` are positive.
2. Implement `build_risk_heatmap_data()` — maps `RiskEntry` list into nested treemap cells. Outer group = module prefix, inner = file. Area = blast radius. Color = risk_score.
3. Write test: file with `risk_score > 0.8` gets `severity: "high"` in treemap cell.
4. Write test: file with no blast radius (blast_radius == 0) gets `area_value = 1` (minimum floor so it's visible).
5. Implement `render_risk_heatmap()` — D3 treemap layout is pre-computed server-side for static export; for HTML the treemap layout is delegated to D3 squarify in the browser (much simpler).
6. Write test: the blast-radius explode data is present as a nested structure on each cell.

**Types:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct RiskHeatmapData {
    pub root: TreemapNode,
    pub total_risk_files: usize,
    pub max_risk: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct TreemapNode {
    pub id: String,
    pub label: String,
    /// For leaf nodes: blast_radius. For group nodes: sum of children.
    pub area_value: f64,
    pub risk_score: f64,
    pub severity: String,   // "high" | "medium" | "low"
    pub children: Vec<TreemapNode>,
    /// Present on leaf nodes: the files in the blast radius
    pub blast_radius_files: Vec<String>,
    pub tooltip: RiskTooltip,
}

#[derive(Debug, serde::Serialize)]
pub struct RiskTooltip {
    pub path: String,
    pub churn_30d: u32,
    pub blast_radius: usize,
    pub test_count: usize,
    pub coupling: f64,
}

pub fn build_risk_heatmap_data(index: &CodebaseIndex) -> RiskHeatmapData;
pub fn render_risk_heatmap(index: &CodebaseIndex, metadata: &RenderMetadata) -> String;
```

---

## Task 10 — Flow Diagram View

**Files:** `src/visual/render.rs` (continued)

**Steps:**

1. Write test: `build_flow_diagram_data` on a 4-node flow path with one cross-language boundary produces `dividers.len() == 1`.
2. Implement `build_flow_diagram_data()` — converts `DataFlowResult` into a left-to-right layout. Groups passthrough chains (>3 consecutive Passthrough nodes) into collapsed segments.
3. Write test: a flow path with 15 nodes where nodes 4-11 are Passthrough gets collapsed to ≤10 visible nodes.
4. Write test: a `Sink` node gets `color_class: "sink"` in the rendered output.
5. Implement `render_flow_diagram()` — Sugiyama with forced left-to-right layering (each node in the path gets its own layer, dummy nodes for cross-layer edges).
6. Write test: cross-language boundary between Python and TypeScript produces a divider with `languages: ["python", "typescript"]`.

**Types:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct FlowDiagramData {
    pub layout: ComputedLayout,
    pub dividers: Vec<CrossLangDivider>,
    pub security_checkpoints: Vec<SecurityCheckpoint>,
    pub missing_security: Vec<MissingSecurityEdge>,
    pub symbol: String,
    pub confidence: String,   // "Exact" | "Approximate" | "Speculative"
    pub truncated: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct CrossLangDivider {
    /// x-coordinate where the vertical divider line is drawn
    pub x_position: f64,
    pub left_language: String,
    pub right_language: String,
}

#[derive(Debug, serde::Serialize)]
pub struct SecurityCheckpoint {
    pub node_id: String,
    pub checkpoint_type: String,   // "auth", "validation", "sanitize"
}

#[derive(Debug, serde::Serialize)]
pub struct MissingSecurityEdge {
    pub from_node_id: String,
    pub to_node_id: String,
    pub warning: String,
}

pub fn build_flow_diagram_data(
    flow: &crate::intelligence::data_flow::DataFlowResult,
    index: &CodebaseIndex,
    config: &LayoutConfig,
) -> Result<FlowDiagramData, LayoutError>;

pub fn render_flow_diagram(
    flow: &crate::intelligence::data_flow::DataFlowResult,
    index: &CodebaseIndex,
    metadata: &RenderMetadata,
) -> Result<String, LayoutError>;
```

---

## Task 11 — Time Machine View

**Files:**
- `src/visual/render.rs` (view builder)
- `src/visual/timeline.rs` (snapshot computation)

**Steps:**

1. Write test: `TimelineSnapshot` serializes/deserializes round-trip losslessly.
2. Implement `TimelineSnapshot` — file-list + edge-count pairs + health metrics (no full parse state). Target: ~5KB per snapshot.
3. Write test: `compute_timeline_snapshots` on a repo with 50 commits produces ≤10 snapshots (sampled every 5th commit or weekly, whichever produces fewer).
4. Implement `compute_timeline_snapshots()` — walks git log, samples commits, extracts architecture diffs from git diff deltas (no re-parse). Caches to `.cxpak/timeline/`.
5. Write test: `build_time_machine_data` with 8 snapshots returns `steps.len() == 8`.
6. Implement `build_time_machine_data()` — wraps snapshots into time machine data with key event detection.
7. Write test: a commit that introduces the first cycle is tagged as `KeyEventKind::CycleIntroduced`.
8. Implement `render_time_machine()`.

**Types in `src/visual/timeline.rs`:**

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimelineSnapshot {
    pub commit_sha: String,
    pub commit_date: String,        // ISO 8601
    pub commit_message: String,
    pub files: Vec<SnapshotFile>,
    pub edge_count: usize,
    pub module_count: usize,
    pub health_composite: Option<f64>,
    pub circular_dep_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotFile {
    pub path: String,
    pub imports: Vec<String>,   // file paths, not symbols — from git diff heuristic
}

pub fn compute_timeline_snapshots(
    repo_path: &std::path::Path,
    max_snapshots: usize,   // default 100
) -> Result<Vec<TimelineSnapshot>, crate::git::GitError>;

pub fn load_cached_snapshots(
    repo_path: &std::path::Path,
) -> Option<Vec<TimelineSnapshot>>;

pub fn save_snapshots(
    repo_path: &std::path::Path,
    snapshots: &[TimelineSnapshot],
) -> Result<(), std::io::Error>;
```

**Types in `src/visual/render.rs`:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct TimeMachineData {
    pub steps: Vec<TimeMachineStep>,
    pub current_index: usize,
    pub health_sparkline: Vec<(String, f64)>,   // (date, score)
    pub key_events: Vec<KeyEvent>,
}

#[derive(Debug, serde::Serialize)]
pub struct TimeMachineStep {
    pub snapshot: TimelineSnapshot,
    /// Diff vs previous snapshot
    pub added_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub added_edges: usize,
    pub removed_edges: usize,
    pub layout: ComputedLayout,   // pre-computed for this snapshot
}

#[derive(Debug, serde::Serialize)]
pub struct KeyEvent {
    pub step_index: usize,
    pub commit_sha: String,
    pub kind: KeyEventKind,
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
pub enum KeyEventKind {
    CycleIntroduced,
    CycleResolved,
    LargeChurn,          // >20% of files changed in one commit
    HealthDropped,       // composite dropped >1.0 point
    NewModule,
    ModuleRemoved,
}

pub fn build_time_machine_data(
    snapshots: Vec<TimelineSnapshot>,
    config: &LayoutConfig,
) -> Result<TimeMachineData, LayoutError>;

pub fn render_time_machine(
    snapshots: Vec<TimelineSnapshot>,
    metadata: &RenderMetadata,
    config: &LayoutConfig,
) -> Result<String, LayoutError>;
```

---

## Task 12 — Diff View

**Files:** `src/visual/render.rs` (continued)

**Steps:**

1. Write test: `build_diff_view_data` with `changed_files: ["src/a.rs"]` produces `before` and `after` layouts each with the same total node count.
2. Implement `build_diff_view_data()` — takes current index and a list of pending changed files. Runs `compute_blast_radius` for the change set, then builds two layout variants: `before` (current) and `after` (with blast radius overlay).
3. Write test: files in blast radius on the `after` layout have `metadata.risk_score` set to the change-risk value from `compute_risk()`.
4. Write test: new circular deps introduced by the change appear in `new_cycles`.
5. Implement `render_diff_view()`.
6. Write test: `impact_score` for a change touching 0 files is 0.0; for a change touching all files it is 1.0.

**Types:**

```rust
#[derive(Debug, serde::Serialize)]
pub struct DiffViewData {
    pub before: ComputedLayout,
    pub after: ComputedLayout,
    pub changed_files: Vec<String>,
    pub blast_radius_files: Vec<String>,
    pub new_risks: Vec<RiskDisplayEntry>,
    pub new_cycles: Vec<Vec<String>>,
    pub convention_violations: Vec<ConventionViolationEntry>,
    /// 0.0–1.0 normalized change impact
    pub impact_score: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct ConventionViolationEntry {
    pub file: String,
    pub violation: String,
}

pub fn build_diff_view_data(
    index: &CodebaseIndex,
    changed_files: &[String],
    config: &LayoutConfig,
) -> Result<DiffViewData, LayoutError>;

pub fn render_diff_view(
    index: &CodebaseIndex,
    changed_files: &[String],
    metadata: &RenderMetadata,
    config: &LayoutConfig,
) -> Result<String, LayoutError>;
```

---

## Task 13 — Multi-Format Export

**Files:** `src/visual/export.rs`

**Steps:**

1. Write test: `to_mermaid` on a 3-module layout produces a string starting with `"graph TD"` and containing exactly 3 node definitions.
2. Implement `to_mermaid()` — module graph as Mermaid `graph TD`. File-level as `graph LR`. Cycles marked with `style` red.
3. Write test: `to_svg` returns a string containing `<svg` and `</svg>`.
4. Implement `to_svg()` — renders `ComputedLayout` as SVG using pre-computed coordinates. No JS, no interactivity. Pure SVG with `<rect>`, `<text>`, `<line>` elements.
5. Write test (feature-gated): `to_png` with `--features visual` returns `Ok(bytes)` where `bytes` starts with the PNG magic bytes `[137, 80, 78, 71]`.
6. Implement `to_png()` under `#[cfg(feature = "visual")]` — calls `to_svg()` then rasterizes via `resvg`.
7. Write test: `to_c4` on a 3-module layout produces a string containing `"System("` for each module.
8. Implement `to_c4()` — C4 DSL for Structurizr. Module-level only (C4 Container diagram).
9. Write test: `to_json` round-trips through `serde_json`: `from_str(&to_json(layout)).unwrap()` equals original.
10. Implement `to_json()` — serializes `ComputedLayout` directly.

**Key function signatures:**

```rust
/// Mermaid diagram (module or file level, depending on layout)
pub fn to_mermaid(layout: &ComputedLayout) -> String;

/// Static SVG (no JS, no interactivity — for embedding in docs)
pub fn to_svg(layout: &ComputedLayout, metadata: &RenderMetadata) -> String;

/// PNG rasterization via resvg
#[cfg(feature = "visual")]
pub fn to_png(
    layout: &ComputedLayout,
    metadata: &RenderMetadata,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ExportError>;

/// C4 DSL for Structurizr import
pub fn to_c4(layout: &ComputedLayout, metadata: &RenderMetadata) -> String;

/// JSON serialization of ComputedLayout
pub fn to_json(layout: &ComputedLayout) -> String;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("SVG rendering failed: {0}")]
    SvgRender(String),
    #[error("PNG rasterization failed: {0}")]
    PngRaster(String),
    #[error("serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}
```

**Mermaid node ID escaping:** Node IDs in Mermaid cannot contain `/`, `.`, or `-`. Replace with `_` and truncate to 32 chars. Test: `mermaid_id("src/index/mod.rs")` returns `"src_index_mod_rs"`.

**SVG edge routing:** Use straight lines between pre-computed waypoints for inter-layer edges, and cubic bezier curves for edges within the same layer. The waypoints are already computed by `insert_dummy_nodes()`.

**PNG dimensions:** Default 1920×1080 for dashboard, 2560×1440 for architecture/risk. Configurable via `to_png(layout, meta, width, height)`.

---

## Task 14 — CLI Commands: `cxpak visual` and `cxpak onboard`

**Files:**
- `src/cli/mod.rs`
- `src/commands/visual.rs`
- `src/commands/onboard.rs`

**Steps:**

1. Write test: `Cli::try_parse_from(["cxpak", "visual", "--type", "dashboard"])` succeeds and produces `Commands::Visual { visual_type: VisualType::Dashboard, format: VisualFormat::Html, .. }`.
2. Add `Visual` and `Onboard` variants to `Commands` enum, behind `#[cfg(feature = "visual")]`.
3. Write test: `--type flow --format png` parses as `VisualType::Flow`, `VisualFormat::Png`.
4. Write test: `--type timeline --format mermaid` parses correctly.
5. Implement `src/commands/visual.rs` — builds index, dispatches to the appropriate render function, writes output file or stdout.
6. Write test: `cxpak visual --type dashboard --format json .` produces valid JSON output containing `"health_quadrant"` key.
7. Implement `src/commands/onboard.rs` — builds index, calls `build_onboarding_map`, writes result.
8. Write test: `cxpak onboard .` returns output containing the string `"phases"`.
9. Add dispatch to `src/main.rs`.

**CLI additions to `src/cli/mod.rs`:**

```rust
#[cfg(feature = "visual")]
Visual {
    /// dashboard | architecture | risk | flow | timeline | diff
    #[arg(long, default_value = "dashboard")]
    visual_type: VisualTypeArg,
    /// html | mermaid | svg | png | c4 | json
    #[arg(long, default_value = "html")]
    format: VisualFormatArg,
    #[arg(long)]
    out: Option<PathBuf>,
    /// For flow type: the symbol to trace
    #[arg(long)]
    symbol: Option<String>,
    /// For diff type: comma-separated changed file paths
    #[arg(long)]
    files: Option<String>,
    #[arg(long)]
    focus: Option<String>,
    #[arg(default_value = ".")]
    path: PathBuf,
},

#[cfg(feature = "visual")]
Onboard {
    #[arg(long)]
    focus: Option<String>,
    #[arg(long, default_value = "markdown")]
    format: OutputFormat,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(default_value = ".")]
    path: PathBuf,
},
```

`VisualTypeArg` and `VisualFormatArg` are `clap::ValueEnum` wrappers that convert to `VisualType` and `VisualFormat`. Separate from the serde types to keep CLI parsing decoupled from serialization.

**Default output file naming:**
- `cxpak visual --type dashboard` → writes `cxpak-dashboard.html` in cwd (or `--out` path)
- `cxpak visual --type architecture --format mermaid` → writes `cxpak-architecture.mmd`
- `cxpak visual --type risk --format png` → writes `cxpak-risk.png`
- `cxpak onboard` → writes to stdout by default (markdown)

---

## Task 15 — MCP Tools: `cxpak_visual` and `cxpak_onboard`

**Files:**
- `src/commands/serve.rs` (add new tool handlers)
- `src/daemon/mod.rs` (register tools)

**Steps:**

1. Write test: MCP tool schema for `cxpak_visual` contains parameters `type`, `format`, `focus`, `symbol`, `files`.
2. Implement `handle_cxpak_visual()` — parses parameters, dispatches to appropriate render function, returns tool result with `content` as HTML string (for `format: html`) or file path (for `format: png`).
3. Write test: `cxpak_visual` with `type: "dashboard", format: "json"` returns valid JSON in the tool result content.
4. Write test: `cxpak_visual` with `type: "flow"` but no `symbol` parameter returns an MCP error `"symbol required for flow view"`.
5. Write test: `cxpak_visual` with `type: "diff"` but no `files` parameter returns an MCP error `"files required for diff view"`.
6. Implement `handle_cxpak_onboard()` — builds onboarding map, returns as JSON or markdown.
7. Write test: `cxpak_onboard` returns a result containing a `"phases"` key.
8. Register both tools in the tool list returned by `list_tools()`.

**Tool schemas (MCP JSON Schema format):**

```rust
// cxpak_visual tool description
pub const CXPAK_VISUAL_DESCRIPTION: &str =
    "Generate a visual intelligence dashboard for the codebase. \
     Returns self-contained HTML (default), Mermaid, SVG, PNG, C4 DSL, or JSON. \
     Use type=dashboard for the overview, architecture for dependency exploration, \
     risk for the heatmap, flow for data flow traces, timeline for git history, \
     diff for change impact visualization.";

// cxpak_onboard tool description
pub const CXPAK_ONBOARD_DESCRIPTION: &str =
    "Generate a guided onboarding reading order for a new engineer. \
     Returns phases of files to read, ordered by PageRank and dependency topology. \
     Each file includes symbols to focus on and estimated reading time.";
```

**MCP handler signatures:**

```rust
#[cfg(feature = "visual")]
pub async fn handle_cxpak_visual(
    params: serde_json::Value,
    index: &Arc<RwLock<CodebaseIndex>>,
) -> Result<ToolResult, McpError>;

#[cfg(feature = "visual")]
pub async fn handle_cxpak_onboard(
    params: serde_json::Value,
    index: &Arc<RwLock<CodebaseIndex>>,
) -> Result<ToolResult, McpError>;
```

**Parameter validation in `handle_cxpak_visual`:**

```rust
let visual_type = params.get("type")
    .and_then(|v| v.as_str())
    .unwrap_or("dashboard");

// Validate symbol required for flow
if visual_type == "flow" && params.get("symbol").is_none() {
    return Err(McpError::InvalidParams("symbol required for flow view".into()));
}

// Validate files required for diff
if visual_type == "diff" && params.get("files").is_none() {
    return Err(McpError::InvalidParams("files required for diff view".into()));
}
```

**Large HTML responses:** When `format == "html"` and the rendered HTML exceeds 1MB, write to `.cxpak/visual/` and return the file path instead of inline content. Threshold configurable in `.cxpak.json` (`"mcp_inline_limit_bytes": 1048576`).

---

## Onboarding Map (shared by CLI and MCP)

**IMPORTANT:** The canonical onboarding implementation lives in `src/intelligence/onboarding.rs` (Part 2, Tasks 16-20). This file (`src/visual/onboard.rs`) is a THIN RENDERING LAYER that imports from `src/intelligence/onboarding.rs` and provides `render_onboarding_markdown()` and `render_onboarding_json()`. Do NOT duplicate the computation logic here.

**Files:** `src/visual/onboard.rs` (rendering only — imports types from `src/intelligence/onboarding.rs`)

**Steps:**

1. Import `OnboardingMap`, `OnboardingPhase`, `OnboardingFile` from `crate::intelligence::onboarding`.
2. Implement `render_onboarding_markdown(map: &OnboardingMap) -> String` — formats phases as markdown sections.
3. Implement `render_onboarding_json(map: &OnboardingMap) -> String` — `serde_json::to_string_pretty`.
4. Write test: markdown output contains phase names and file paths.
5. Write test: JSON output round-trips through serde.

**Types:**

```rust
// Defined in design spec — reproduced here for completeness
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OnboardingMap {
    pub total_files: usize,
    pub estimated_reading_time: String,
    pub phases: Vec<OnboardingPhase>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OnboardingPhase {
    pub name: String,
    pub module: String,
    pub rationale: String,
    pub files: Vec<OnboardingFile>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OnboardingFile {
    pub path: String,
    pub pagerank: f64,
    pub symbols_to_focus_on: Vec<String>,
    pub estimated_tokens: usize,
}

pub fn build_onboarding_map(
    index: &CodebaseIndex,
    focus: Option<&str>,
) -> OnboardingMap;

pub fn render_onboarding_markdown(map: &OnboardingMap) -> String;

pub fn render_onboarding_json(map: &OnboardingMap) -> String;
```

**Reading time formula:** `total_tokens / 200` tokens per minute, rounded to nearest 5 minutes. For < 5 minutes: return `"under 5 minutes"`.

---

## File Summary

```
src/visual/
  mod.rs          — VisualType, VisualFormat, VisualOutput; feature-gated sub-mods
  layout.rs       — LayoutNode, LayoutEdge, ComputedLayout; Sugiyama pipeline
  render.rs       — render_html, render_dashboard, render_architecture_explorer,
                    render_risk_heatmap, render_flow_diagram, render_time_machine,
                    render_diff_view; DashboardData et al.
  export.rs       — to_mermaid, to_svg, to_png (visual feature), to_c4, to_json
  timeline.rs     — TimelineSnapshot, compute_timeline_snapshots, cache I/O
  onboard.rs      — OnboardingMap, build_onboarding_map, render_*

assets/
  d3-bundle.min.js  — pre-built custom D3 bundle (~100KB)
  cxpak-visual.css  — minimal stylesheet

src/cli/mod.rs    — Visual and Onboard command variants added
src/commands/
  visual.rs       — CLI handler for cxpak visual
  onboard.rs      — CLI handler for cxpak onboard
src/commands/serve.rs — handle_cxpak_visual, handle_cxpak_onboard
src/main.rs       — dispatch for Visual and Onboard
Cargo.toml        — visual = ["dep:resvg"], plugins = ["dep:wasmtime"]
```

---

## Test Count Target

| Module | Minimum Tests |
|---|---|
| `layout.rs` | 18 (layer assign, crossing, coordinates, builders per level) |
| `render.rs` | 20 (one per view + HTML validity + JSON validity + cognitive limits) |
| `export.rs` | 12 (per format × 2 assertions each) |
| `timeline.rs` | 8 (snapshot compute, cache roundtrip, key event detection) |
| `onboard.rs` | 8 (ordering, dependency order, reading time, symbol selection) |
| `cli/mod.rs` additions | 8 (parse Visual/Onboard with all flag combinations) |
| `daemon/mcp.rs` additions | 6 (visual tool, onboard tool, error cases) |
| **Total** | **≥80 new tests** |

---

## Commands

```bash
# Build with visual feature
cargo build --features visual

# Run all visual tests
cargo test --features visual visual::

# Run layout tests only
cargo test --features visual visual::layout::

# Run export tests (requires resvg for PNG test)
cargo test --features visual visual::export::

# Check formatting and lints before commit
cargo fmt --check
cargo clippy --all-targets --features visual -- -D warnings
```
