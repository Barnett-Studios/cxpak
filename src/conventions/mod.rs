pub mod deps;
pub mod diff;
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

use std::path::Path;

// The convention data model (`ConventionProfile`, `PatternStrength`,
// `PatternObservation`, `FileContribution`, and every per-aspect `*Conventions`
// struct) lives in `core_graph` (cxpak 3.0.0 Phase 0 de-cycle, ADR-0007). The
// `extract_*` / `update_*` / `remove_*` analysis logic stays in this module and
// its sub-modules. Re-exported here at the historical `crate::conventions::{...}`
// paths so every existing reference keeps resolving unchanged.
pub use crate::core_graph::conventions::{
    ConventionProfile, FileContribution, PatternObservation, PatternStrength,
};

/// Build the full convention profile from an already-constructed index.
///
/// Called AFTER `CodebaseIndex::build()` — not inside it. The `repo_path`
/// is needed for git_health extraction via git2.
pub fn build_convention_profile(
    index: &crate::core_graph::CodebaseIndex,
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
    index: &crate::core_graph::CodebaseIndex,
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

    // Rebuild the five observation-based profiles so stale aggregates are
    // replaced. These are O(total symbols) — fast even for large codebases.
    profile.naming = naming::extract_naming(index);
    profile.imports = imports::extract_imports(index);
    profile.errors = errors::extract_errors(index);
    profile.visibility = visibility::extract_visibility(index);
    profile.functions = functions::extract_functions(index);

    // Git health: NOT updated here — uses TTL cache
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

        let index = crate::core_graph::CodebaseIndex::build(files, parse_results, &counter);
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
    fn test_incremental_update_naming_matches_full_rebuild() {
        use crate::budget::counter::TokenCounter;
        use crate::parser::language::SymbolKind;
        use crate::parser::language::{ParseResult, Symbol, Visibility};
        use crate::scanner::ScannedFile;
        use std::collections::HashMap;

        let counter = TokenCounter::new();
        let dir = tempfile::TempDir::new().unwrap();

        let fp_a = dir.path().join("a.rs");
        let fp_b = dir.path().join("b.rs");
        std::fs::write(&fp_a, "fn snake_one() {}").unwrap();
        std::fs::write(&fp_b, "fn snake_two() {}").unwrap();

        let files = vec![
            ScannedFile {
                relative_path: "src/a.rs".into(),
                absolute_path: fp_a.clone(),
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
                    name: "snake_one".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn snake_one()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );
        parse_results.insert(
            "src/b.rs".into(),
            ParseResult {
                symbols: vec![Symbol {
                    name: "snake_two".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: "pub fn snake_two()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                }],
                imports: vec![],
                exports: vec![],
            },
        );

        let index = crate::core_graph::CodebaseIndex::build(files, parse_results, &counter);
        let full_profile = build_convention_profile(&index, dir.path());

        let mut incremental_profile = build_convention_profile(&index, dir.path());
        // Trigger an incremental update without changing anything meaningful.
        update_conventions_incremental(
            &mut incremental_profile,
            &["src/b.rs".to_string()],
            &[],
            &index,
        );

        // Both profiles should agree on function_style dominant.
        assert_eq!(
            full_profile
                .naming
                .function_style
                .as_ref()
                .map(|o| o.dominant.as_str()),
            incremental_profile
                .naming
                .function_style
                .as_ref()
                .map(|o| o.dominant.as_str()),
            "incremental update must produce same function_style as full rebuild"
        );
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

        let index = crate::core_graph::CodebaseIndex::build(files, parse_results, &counter);
        // Build initial profile without a git repo (graceful default for git_health)
        let mut profile = build_convention_profile(&index, dir.path());

        // Both files should be tracked in naming contributions
        assert!(profile.naming.file_contributions.contains_key("src/a.rs"));
        assert!(profile.naming.file_contributions.contains_key("src/b.rs"));

        // Build a post-removal index that no longer contains src/a.rs.
        let fp_b2 = dir.path().join("b.rs");
        let files_after: Vec<crate::scanner::ScannedFile> = vec![ScannedFile {
            relative_path: "src/b.rs".into(),
            absolute_path: fp_b2,
            language: Some("rust".into()),
            size_bytes: 1,
        }];
        let mut pr_after = HashMap::new();
        pr_after.insert(
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
        let index_after = crate::core_graph::CodebaseIndex::build(files_after, pr_after, &counter);

        // Remove "src/a.rs", update "src/b.rs" — pass the post-removal index.
        update_conventions_incremental(
            &mut profile,
            &["src/b.rs".to_string()],
            &["src/a.rs".to_string()],
            &index_after,
        );

        // After rebuild from index_after, "src/a.rs" is gone because it is not
        // in the new index.
        assert!(!profile.naming.file_contributions.contains_key("src/a.rs"));
        // "src/b.rs" still present.
        assert!(profile.naming.file_contributions.contains_key("src/b.rs"));
    }
}
