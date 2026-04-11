//! Sugiyama layout engine for architecture visualizations.
//!
//! The layout module implements a multi-level graph layout algorithm (Sugiyama method)
//! for drawing dependency graphs, architecture diagrams, and data flow visualizations.
//!
//! Implementation includes:
//! - Layer assignment via topological sort (Task 3)
//! - Crossing minimization using heuristic ordering (Task 4)
//! - Coordinate assignment for node positioning (Task 4)
//! - Layout builders for module, file, and symbol graphs (Task 5)
