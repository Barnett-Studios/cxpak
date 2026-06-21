//! The `Domain` enum — a pure data type stored on `CodebaseIndex.domains`.
//!
//! The detection heuristics (`detect_domains`) and query-expansion synonym
//! maps (`expand_query`, `DOMAIN_SYNONYMS`) that operate over this enum stay in
//! `src/context_quality/expansion.rs`; only the data type lives here so that
//! `core_graph` remains a leaf foundation (cxpak 3.0.0 Phase 0 de-cycle) —
//! `core_graph` must not depend on the higher `context_quality` layer.

/// A detected codebase domain (file-pattern heuristic result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Domain {
    Web,
    Database,
    Auth,
    Infra,
    Testing,
    Api,
    Mobile,
    ML,
}
