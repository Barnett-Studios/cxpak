//! Intelligence payload data types stored on `CodebaseIndex`.
//!
//! These are pure data structures (plus the call-graph's pure query methods).
//! The analysis functions that build them (`detect_dead_code`, `compute_health`,
//! `build_test_map`, `build_call_graph`, `mine_co_changes_*`,
//! `detect_cross_lang_edges`) stay in `src/intelligence/` — they consume a
//! `CodebaseIndex` and therefore live above the `core_graph` boundary.

use crate::core_graph::graph::BridgeType;
use crate::parser::language::SymbolKind;
use serde::{Deserialize, Serialize};

// ─── dead code ────────────────────────────────────────────────────────────────

/// A symbol classified as dead (zero callers, not an entry point).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadSymbol {
    pub file: String,
    pub symbol: String,
    pub kind: SymbolKind,
    /// Sorting key: higher = more concerning dead symbol.
    /// Formula: pagerank * (1.0 + test_file_count) * export_weight
    /// where export_weight = 2.0 for pub exports, 1.0 otherwise.
    pub liveness_score: f64,
    pub reason: String,
}

// ─── health ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct HealthScore {
    pub composite: f64,
    pub conventions: f64,
    pub test_coverage: f64,
    pub churn_stability: f64,
    pub coupling: f64,
    pub cycles: f64,
    pub dead_code: Option<f64>,
}

// ─── test map ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFileRef {
    pub path: String,
    pub confidence: TestConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestConfidence {
    NameMatch,
    ImportMatch,
    Both,
}

// ─── call graph ───────────────────────────────────────────────────────────────

/// Confidence level for a resolved call edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallConfidence {
    /// Tree-sitter extracted call expression, import-resolved to a specific file.
    Exact,
    /// Regex-matched against known symbol names in Tier 2 or unresolvable Tier 1.
    Approximate,
}

/// A resolved cross-file function call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_file: String,
    pub callee_symbol: String,
    pub confidence: CallConfidence,
    /// Present when this edge was resolved ambiguously. For example, when
    /// multiple files export the same symbol the Approximate picker selects
    /// the first exporter lexicographically — deterministic but arbitrary.
    /// Consumers that require exact provenance should treat this edge as
    /// low-confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
}

/// A call that could not be resolved to a specific file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedCall {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_name: String,
}

/// The full call graph for a codebase.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<UnresolvedCall>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns all callers of a given symbol in a given file.
    pub fn callers_of(&self, file: &str, symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.callee_file == file && e.callee_symbol == symbol)
            .collect()
    }

    /// Returns all callees from a given symbol in a given file.
    pub fn callees_from(&self, file: &str, symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.caller_file == file && e.caller_symbol == symbol)
            .collect()
    }

    /// Returns true if a symbol has at least one caller — ANY confidence.
    pub fn has_callers(&self, file: &str, symbol: &str) -> bool {
        self.edges
            .iter()
            .any(|e| e.callee_file == file && e.callee_symbol == symbol)
    }

    /// Returns true only if the symbol has at least one EXACT caller — an
    /// edge whose resolution is confirmed (either intra-file or imported
    /// from this file via the dependency graph).
    ///
    /// `Approximate` edges are emitted when a call's name matches a public
    /// symbol elsewhere but the caller does not explicitly import from the
    /// definer. These are common-name ambiguity artifacts: a call to
    /// `run()` in module A binds approximately to whoever exports `run`
    /// even if A never imports that module. Dead-code detection must not
    /// treat these as real callers, or every function named `run` / `new`
    /// / `build` gets falsely marked alive.
    pub fn has_exact_callers(&self, file: &str, symbol: &str) -> bool {
        self.edges.iter().any(|e| {
            e.callee_file == file
                && e.callee_symbol == symbol
                && e.confidence == CallConfidence::Exact
        })
    }
}

// ─── co-change ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoChangeEdge {
    pub file_a: String,
    pub file_b: String,
    pub count: u32,
    pub recency_weight: f64,
}

// ─── cross-language ───────────────────────────────────────────────────────────

/// A detected cross-language boundary between two files.
#[derive(Debug, Clone, Serialize)]
pub struct CrossLangEdge {
    pub source_file: String,
    pub source_symbol: String,
    pub source_language: String,
    pub target_file: String,
    pub target_symbol: String,
    pub target_language: String,
    pub bridge_type: BridgeType,
}
