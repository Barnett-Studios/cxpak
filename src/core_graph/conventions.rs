//! Convention-profile data model: the `ConventionProfile`, its per-aspect
//! sub-structs, and the shared pattern primitives.
//!
//! Pure data types (plus `PatternObservation`'s pure constructors). The
//! `extract_*` / `update_*` / `remove_*` analysis functions that build these
//! from a `CodebaseIndex` stay in `src/conventions/` — they consume the index
//! and live above the `core_graph` boundary.

use crate::core_graph::intel::CoChangeEdge;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── shared pattern primitives ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatternStrength {
    Convention, // ≥90%
    Trend,      // 70-89%
    Mixed,      // 50-69%
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternObservation {
    pub name: String,
    pub dominant: String,
    pub count: usize,
    pub total: usize,
    pub percentage: f64,
    pub strength: PatternStrength,
    pub exceptions: Vec<String>,
}

impl PatternObservation {
    pub fn new(name: &str, dominant: &str, count: usize, total: usize) -> Option<Self> {
        if total == 0 {
            return None;
        }
        let percentage = (count as f64 / total as f64) * 100.0;
        if percentage < 50.0 {
            return None;
        }
        let strength = if percentage >= 90.0 {
            PatternStrength::Convention
        } else if percentage >= 70.0 {
            PatternStrength::Trend
        } else {
            PatternStrength::Mixed
        };
        Some(Self {
            name: name.to_string(),
            dominant: dominant.to_string(),
            count,
            total,
            percentage,
            strength,
            exceptions: Vec::new(),
        })
    }

    pub fn with_exceptions(mut self, exceptions: Vec<String>) -> Self {
        self.exceptions = exceptions;
        self
    }
}

/// Per-file contribution tracking for incremental updates.
///
/// Each category stores a map of file path → contribution counts.
/// When a file changes: subtract old, add new, recompute percentages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileContribution {
    /// Counts keyed by pattern name (e.g., "snake_case" → 5, "camel_case" → 1)
    pub counts: HashMap<String, usize>,
}

// ─── top-level profile ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConventionProfile {
    pub naming: NamingConventions,
    pub imports: ImportConventions,
    pub errors: ErrorConventions,
    pub dependencies: DependencyConventions,
    pub testing: TestingConventions,
    pub visibility: VisibilityConventions,
    pub functions: FunctionConventions,
    pub git_health: GitHealthProfile,
}

// ─── naming ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamingStyle {
    SnakeCase,
    CamelCase,
    PascalCase,
    ScreamingSnake,
    KebabCase,
    Other,
}

impl std::fmt::Display for NamingStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NamingStyle::SnakeCase => write!(f, "snake_case"),
            NamingStyle::CamelCase => write!(f, "camelCase"),
            NamingStyle::PascalCase => write!(f, "PascalCase"),
            NamingStyle::ScreamingSnake => write!(f, "SCREAMING_SNAKE_CASE"),
            NamingStyle::KebabCase => write!(f, "kebab-case"),
            NamingStyle::Other => write!(f, "other"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamingConventions {
    pub function_style: Option<PatternObservation>,
    pub type_style: Option<PatternObservation>,
    pub file_style: Option<PatternObservation>,
    pub constant_style: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

// ─── imports ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportConventions {
    pub style: Option<PatternObservation>,
    pub grouping: Option<PatternObservation>,
    pub re_exports: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

// ─── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorConventions {
    pub result_return: Option<PatternObservation>,
    pub unwrap_usage: Option<PatternObservation>,
    pub expect_usage: Option<PatternObservation>,
    pub question_mark_propagation: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

// ─── dependencies ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyConventions {
    pub strict_layers: Vec<DirectionPair>,
    pub circular_deps: Vec<String>,
    pub db_isolation: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectionPair {
    pub from: String,
    pub to: String,
    pub edge_count: usize,
    pub reverse_count: usize,
}

// ─── testing ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestingConventions {
    pub coverage_by_dir: HashMap<String, f64>,
    pub mock_usage: Option<PatternObservation>,
    pub test_naming: Option<PatternObservation>,
    pub density: Option<PatternObservation>,
    /// Whether the codebase uses inline tests (e.g. `#[cfg(test)]` in Rust,
    /// `def test_` in Python, `describe(`/`it(` in JS/TS, `func Test` in Go).
    pub has_inline_tests: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
}

// ─── visibility ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisibilityConventions {
    pub public_ratio: Option<PatternObservation>,
    pub doc_comment_coverage: Option<PatternObservation>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

// ─── functions ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionConventions {
    pub avg_length: Option<f64>,
    pub median_length: Option<f64>,
    pub by_directory: HashMap<String, DirectoryFunctionStats>,
    pub additional: Vec<PatternObservation>,
    #[serde(skip)]
    pub file_contributions: HashMap<String, FileContribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryFunctionStats {
    pub avg_length: f64,
    pub median_length: f64,
    pub count: usize,
}

// ─── git health ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHealthProfile {
    pub churn_30d: Vec<ChurnEntry>,
    pub churn_180d: Vec<ChurnEntry>,
    pub bugfix_density: HashMap<String, f64>,
    pub reverts: Vec<RevertEntry>,
    pub churn_trend: HashMap<String, ChurnTrend>,
    pub co_changes: Vec<CoChangeEdge>,
    #[serde(skip)]
    pub last_computed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnEntry {
    pub path: String,
    pub modifications: usize,
    /// UNIX epoch seconds of this file's most recent commit inside the window.
    /// `#[serde(default)]` keeps deserialization of v2.1.0-and-earlier
    /// conventions.json (which lacks this field) working — missing -> None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_epoch: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertEntry {
    pub commit_message: String,
    pub reverted_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChurnTrend {
    Hot,        // high 30d, lower 180d (growing)
    Stabilized, // low 30d, high 180d (cooling down)
    Chronic,    // high both windows
    Cold,       // low both windows
}
