//! Visual output and architecture rendering.
//!
//! The visual module provides interactive dashboards, architecture diagrams,
//! risk heatmaps, data flow visualizations, timelines, and diff comparisons
//! across multiple output formats (HTML, Mermaid, SVG, PNG, C4, JSON).

#[cfg(feature = "visual")]
pub mod export;
#[cfg(feature = "visual")]
pub mod layout;
#[cfg(feature = "visual")]
pub mod onboard;
#[cfg(feature = "visual")]
pub mod render;

/// Which visualization type to generate
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VisualType {
    /// Interactive dashboard with summary metrics and navigation
    Dashboard,
    /// Architecture diagram showing modules and dependencies
    Architecture,
    /// Risk heatmap highlighting risky files and patterns
    Risk,
    /// Data flow diagram showing value propagation through the system
    Flow,
    /// Timeline showing changes and evolution over time
    Timeline,
    /// Diff visualization comparing two snapshots
    Diff,
}

/// Output format for a visualization
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VisualFormat {
    /// Interactive HTML with embedded JavaScript
    Html,
    /// Mermaid diagram syntax (can be rendered by various tools)
    Mermaid,
    /// Scalable Vector Graphics
    Svg,
    /// Raster image (PNG)
    #[cfg(feature = "visual")]
    Png,
    /// C4 model notation
    C4,
    /// JSON representation for programmatic use
    Json,
}

/// The computed result of a visualization — before serialization to a format
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct VisualOutput {
    /// The type of visualization generated
    pub visual_type: VisualType,
    /// The output format (HTML, Mermaid, etc.)
    pub format: VisualFormat,
    /// File path written, or None if returned inline
    pub path: Option<String>,
    /// Inline content (HTML string, Mermaid source, SVG, JSON)
    pub content: Option<String>,
}
