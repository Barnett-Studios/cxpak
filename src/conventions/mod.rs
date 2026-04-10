pub mod deps;
pub mod errors;
pub mod export;
pub mod functions;
pub mod git_health;
pub mod imports;
pub mod naming;
pub mod render;
pub mod testing;
pub mod verify;
pub mod visibility;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConventionProfile {
    pub naming: naming::NamingConventions,
    pub imports: imports::ImportConventions,
    pub errors: errors::ErrorConventions,
    pub dependencies: deps::DependencyConventions,
    pub testing: testing::TestingConventions,
    pub visibility: visibility::VisibilityConventions,
    pub functions: functions::FunctionConventions,
    pub git_health: git_health::GitHealthProfile,
}

/// Build the full convention profile from an already-constructed index.
///
/// Called AFTER `CodebaseIndex::build()` — not inside it. The `repo_path`
/// is needed for git_health extraction via git2.
pub fn build_convention_profile(
    index: &crate::index::CodebaseIndex,
    repo_path: &Path,
) -> ConventionProfile {
    ConventionProfile {
        naming: naming::extract_naming(index),
        imports: imports::extract_imports(index),
        errors: errors::extract_errors(index),
        dependencies: deps::extract_deps(index),
        testing: testing::extract_testing(index),
        visibility: visibility::extract_visibility(index),
        functions: functions::extract_functions(index),
        git_health: git_health::extract_git_health(repo_path),
    }
}

/// Incrementally update conventions when files change.
///
/// Called after `apply_incremental_update` in the watch loop.
/// Subtracts old file contributions, adds new ones, recomputes percentages.
/// Git health is NOT updated here (uses 60s TTL cache).
pub fn update_conventions_incremental(
    profile: &mut ConventionProfile,
    modified_paths: &[String],
    removed_paths: &[String],
    index: &crate::index::CodebaseIndex,
) {
    // Remove contributions from deleted files
    for path in removed_paths {
        naming::remove_file_contribution(&mut profile.naming, path);
        imports::remove_file_contribution(&mut profile.imports, path);
        errors::remove_file_contribution(&mut profile.errors, path);
        visibility::remove_file_contribution(&mut profile.visibility, path);
        functions::remove_file_contribution(&mut profile.functions, path);
    }

    // Update contributions from modified files (subtract old, add new)
    for path in modified_paths {
        if let Some(file) = index.files.iter().find(|f| f.relative_path == *path) {
            naming::update_file_contribution(&mut profile.naming, file);
            imports::update_file_contribution(&mut profile.imports, file);
            errors::update_file_contribution(&mut profile.errors, file);
            visibility::update_file_contribution(&mut profile.visibility, file);
            functions::update_file_contribution(&mut profile.functions, file);
        }
    }

    // Recompute dependency conventions from full graph (cheap)
    profile.dependencies = deps::extract_deps(index);

    // Recompute testing from full test_map (cheap)
    profile.testing = testing::extract_testing(index);

    // Git health: NOT updated here — uses TTL cache
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_observation_convention() {
        let obs = PatternObservation::new("fn_naming", "snake_case", 95, 100).unwrap();
        assert_eq!(obs.percentage, 95.0);
        assert!(matches!(obs.strength, PatternStrength::Convention));
        assert_eq!(obs.name, "fn_naming");
        assert_eq!(obs.dominant, "snake_case");
        assert_eq!(obs.count, 95);
        assert_eq!(obs.total, 100);
    }

    #[test]
    fn test_pattern_observation_trend() {
        let obs = PatternObservation::new("fn_naming", "snake_case", 75, 100).unwrap();
        assert_eq!(obs.percentage, 75.0);
        assert!(matches!(obs.strength, PatternStrength::Trend));
    }

    #[test]
    fn test_pattern_observation_mixed() {
        let obs = PatternObservation::new("fn_naming", "snake_case", 55, 100).unwrap();
        assert!((obs.percentage - 55.0).abs() < 0.01);
        assert!(matches!(obs.strength, PatternStrength::Mixed));
    }

    #[test]
    fn test_pattern_observation_below_50_returns_none() {
        assert!(PatternObservation::new("fn_naming", "snake_case", 40, 100).is_none());
    }

    #[test]
    fn test_pattern_observation_zero_total_returns_none() {
        assert!(PatternObservation::new("fn_naming", "snake_case", 0, 0).is_none());
    }

    #[test]
    fn test_pattern_observation_boundary_90() {
        let obs = PatternObservation::new("x", "y", 90, 100).unwrap();
        assert!(matches!(obs.strength, PatternStrength::Convention));
    }

    #[test]
    fn test_pattern_observation_boundary_70() {
        let obs = PatternObservation::new("x", "y", 70, 100).unwrap();
        assert!(matches!(obs.strength, PatternStrength::Trend));
    }

    #[test]
    fn test_pattern_observation_boundary_50() {
        let obs = PatternObservation::new("x", "y", 50, 100).unwrap();
        assert!(matches!(obs.strength, PatternStrength::Mixed));
    }

    #[test]
    fn test_pattern_observation_boundary_49_none() {
        assert!(PatternObservation::new("x", "y", 49, 100).is_none());
    }

    #[test]
    fn test_pattern_observation_with_exceptions() {
        let obs = PatternObservation::new("fn_naming", "snake_case", 95, 100)
            .unwrap()
            .with_exceptions(vec!["ffi_binding".to_string()]);
        assert_eq!(obs.exceptions.len(), 1);
        assert_eq!(obs.exceptions[0], "ffi_binding");
    }

    #[test]
    fn test_convention_profile_default() {
        let profile = ConventionProfile::default();
        assert!(profile.naming.function_style.is_none());
        assert!(profile.naming.type_style.is_none());
    }

    #[test]
    fn test_build_convention_profile_runs_without_git_repo() {
        use crate::budget::counter::TokenCounter;
        use crate::parser::language::SymbolKind;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();
        let fp = dir.path().join("lib.rs");
        std::fs::write(&fp, "x").unwrap();

        let files = vec![ScannedFile {
            relative_path: "src/lib.rs".into(),
            absolute_path: fp,
            language: Some("rust".into()),
            size_bytes: 1,
        }];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/lib.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "my_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn my_fn() -> Result<(), E>".into(),
                    body: "{ Ok(()) }".into(),
                    start_line: 1,
                    end_line: 3,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = crate::index::CodebaseIndex::build(files, parse_results, &counter);
        // dir.path() is not a git repo — git_health will gracefully return default
        let profile = build_convention_profile(&index, dir.path());

        // naming should detect snake_case from "my_fn"
        assert!(profile.naming.function_style.is_some());
        let fn_style = profile.naming.function_style.unwrap();
        assert_eq!(fn_style.dominant, "snake_case");

        // errors should detect Result return type
        assert!(profile.errors.result_return.is_some());

        // git_health defaults when not a git repo
        assert!(profile.git_health.churn_30d.is_empty());
    }

    #[test]
    fn test_update_conventions_incremental_remove_then_add() {
        use crate::budget::counter::TokenCounter;
        use crate::parser::language::SymbolKind;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        // Create two source files
        let fp_a = dir.path().join("a.rs");
        let fp_b = dir.path().join("b.rs");
        std::fs::write(&fp_a, "x").unwrap();
        std::fs::write(&fp_b, "x").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/a.rs".into(),
                absolute_path: fp_a,
                language: Some("rust".into()),
                size_bytes: 1,
            },
            ScannedFile {
                relative_path: "src/b.rs".into(),
                absolute_path: fp_b,
                language: Some("rust".into()),
                size_bytes: 1,
            },
        ];

        let mut parse_results = HashMap::new();
        parse_results.insert(
            "src/a.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "fn_in_a".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn fn_in_a()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 2,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/b.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "fn_in_b".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn fn_in_b()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 2,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = crate::index::CodebaseIndex::build(files, parse_results, &counter);
        // Build initial profile without a git repo (graceful default for git_health)
        let mut profile = build_convention_profile(&index, dir.path());

        // Both files should be tracked in naming contributions
        assert!(profile.naming.file_contributions.contains_key("src/a.rs"));
        assert!(profile.naming.file_contributions.contains_key("src/b.rs"));

        // Remove "src/a.rs", update "src/b.rs"
        update_conventions_incremental(
            &mut profile,
            &["src/b.rs".to_string()],
            &["src/a.rs".to_string()],
            &index,
        );

        // "src/a.rs" must be gone from naming contributions
        assert!(!profile.naming.file_contributions.contains_key("src/a.rs"));
        // "src/b.rs" still present (update is a noop but the key is preserved)
        assert!(profile.naming.file_contributions.contains_key("src/b.rs"));
    }
}
