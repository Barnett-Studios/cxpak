//! Multi-format export for visualizations.
//!
//! The export module serializes computed visualizations into various output formats:
//! HTML (interactive), Mermaid (diagram syntax), SVG (vector graphics),
//! PNG (raster images), C4 (model notation), and JSON (programmatic access).
//!
//! Implementation includes:
//! - Format-specific serialization and optimization
//! - Embedded asset management (CSS, JavaScript, fonts)
//! - Streaming for large visualizations
//! - Metadata and lineage preservation
