//! `core_graph` — the acyclic foundation of cxpak's data model (cxpak 3.0.0
//! Phase 0, ADR-0007 module boundaries).
//!
//! This module owns the shared data structures that `index`, `schema`,
//! `intelligence`, and `conventions` all build on: the dependency-graph
//! primitives, the `CodebaseIndex` data model (+ its pure queries), and every
//! payload type stored on it (schema, intelligence, conventions). Those four
//! modules depend on `core_graph` one-directionally — the analysis/orchestration
//! *logic* lives in them; only the data model and its pure queries live here.
//!
//! `core_graph` depends only on `parser` (for `Symbol`/`Import`/`ParseResult`
//! inside `IndexedFile`), `context_quality` (for the `Domain` set field),
//! `embeddings` (feature-gated index field), `std`, and `serde`. It must NEVER
//! import from `index`/`schema`/`intelligence`/`conventions`, or the cycle this
//! boundary was created to break would simply move here.

pub mod conventions;
pub mod graph;
pub mod index;
pub mod intel;
pub mod schema;

pub use index::{CodebaseIndex, IndexedFile, LanguageStats};
