use crate::conventions::{ConventionProfile, PatternStrength};

/// Render the full DNA section (~800-1000 tokens).
///
/// Includes Convention + Trend patterns ordered by percentage desc,
/// plus git health (top 5 churn, reverts).
pub fn render_dna_section(profile: &ConventionProfile) -> String {
    let mut sections: Vec<String> = Vec::new();

    sections.push("## Repository DNA\n".to_string());

    // Naming conventions
    let mut naming_lines = Vec::new();
    if let Some(ref obs) = profile.naming.function_style {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            naming_lines.push(format!(
                "- Functions: {} ({}/{}, {:.1}%)",
                obs.dominant, obs.count, obs.total, obs.percentage
            ));
        }
    }
    if let Some(ref obs) = profile.naming.type_style {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            naming_lines.push(format!(
                "- Types: {} ({}/{}, {:.1}%)",
                obs.dominant, obs.count, obs.total, obs.percentage
            ));
        }
    }
    if let Some(ref obs) = profile.naming.file_style {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            naming_lines.push(format!(
                "- Files: {} ({:.1}%)",
                obs.dominant, obs.percentage
            ));
        }
    }
    if let Some(ref obs) = profile.naming.constant_style {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            naming_lines.push(format!(
                "- Constants: {} ({:.1}%)",
                obs.dominant, obs.percentage
            ));
        }
    }
    if !naming_lines.is_empty() {
        let strength = profile
            .naming
            .function_style
            .as_ref()
            .map(|o| format!("{:?}", o.strength).to_lowercase())
            .unwrap_or_else(|| "not detected".into());
        sections.push(format!("### Naming ({strength})"));
        sections.extend(naming_lines);
        sections.push(String::new());
    }

    // Error handling
    let mut error_lines = Vec::new();
    if let Some(ref obs) = profile.errors.result_return {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            error_lines.push(format!(
                "- Return type: {} ({}/{}, {:.1}%)",
                obs.dominant, obs.count, obs.total, obs.percentage
            ));
        }
    }
    if let Some(ref obs) = profile.errors.unwrap_usage {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            error_lines.push(format!(
                "- {} ({}/{}, {:.1}%)",
                obs.dominant, obs.count, obs.total, obs.percentage
            ));
        }
    }
    if let Some(ref obs) = profile.errors.question_mark_propagation {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            error_lines.push(format!(
                "- Propagation: {} ({}/{}, {:.1}%)",
                obs.dominant, obs.count, obs.total, obs.percentage
            ));
        }
    }
    if !error_lines.is_empty() {
        let strength = profile
            .errors
            .result_return
            .as_ref()
            .map(|o| format!("{:?}", o.strength).to_lowercase())
            .unwrap_or_else(|| "not detected".into());
        sections.push(format!("### Error Handling ({strength})"));
        sections.extend(error_lines);
        sections.push(String::new());
    }

    // Imports
    if let Some(ref obs) = profile.imports.style {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            let strength = format!("{:?}", obs.strength).to_lowercase();
            sections.push(format!("### Imports ({strength})"));
            sections.push(format!(
                "- Style: {} ({:.1}%)",
                obs.dominant, obs.percentage
            ));
            sections.push(String::new());
        }
    }

    // Dependencies / Architecture
    if !profile.dependencies.strict_layers.is_empty()
        || !profile.dependencies.circular_deps.is_empty()
    {
        sections.push("### Architecture".to_string());
        for layer in &profile.dependencies.strict_layers {
            sections.push(format!(
                "- Layering: {} → {} ({} edges, 0 reverse)",
                layer.from, layer.to, layer.edge_count
            ));
        }
        if profile.dependencies.circular_deps.is_empty() {
            sections.push("- No circular deps between top-level modules".to_string());
        } else {
            for circ in &profile.dependencies.circular_deps {
                sections.push(format!("- Circular: {circ}"));
            }
        }
        sections.push(String::new());
    }

    // Visibility
    if let Some(ref obs) = profile.visibility.public_ratio {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            let strength = format!("{:?}", obs.strength).to_lowercase();
            sections.push(format!(
                "### Visibility ({strength}, {:.0}%)",
                obs.percentage
            ));
            sections.push(format!(
                "- Default: {} ({}/{})",
                obs.dominant, obs.count, obs.total
            ));
            if let Some(ref doc) = profile.visibility.doc_comment_coverage {
                sections.push(format!(
                    "- Doc comments on {:.0}% of public APIs",
                    doc.percentage
                ));
            }
            sections.push(String::new());
        }
    }

    // Functions
    if let Some(avg) = profile.functions.avg_length {
        sections.push("### Functions".to_string());
        sections.push(format!(
            "- Average length: {:.0} lines (median {:.0})",
            avg,
            profile.functions.median_length.unwrap_or(0.0)
        ));
        for (dir, stats) in &profile.functions.by_directory {
            if stats.count >= 3 {
                sections.push(format!(
                    "- {dir}: avg {:.0} lines ({} functions)",
                    stats.avg_length, stats.count
                ));
            }
        }
        sections.push(String::new());
    }

    // Testing
    if let Some(ref obs) = profile.testing.mock_usage {
        if matches!(
            obs.strength,
            PatternStrength::Convention | PatternStrength::Trend
        ) {
            let strength = format!("{:?}", obs.strength).to_lowercase();
            sections.push(format!("### Testing ({strength})"));
            sections.push(format!("- {}", obs.dominant));
            if let Some(ref naming) = profile.testing.test_naming {
                sections.push(format!(
                    "- Naming: {} ({:.0}%)",
                    naming.dominant, naming.percentage
                ));
            }
            if let Some(ref density) = profile.testing.density {
                sections.push(format!("- Density: {}", density.dominant));
            }
            sections.push(String::new());
        }
    }

    // Git health (top 5 churn + reverts)
    if !profile.git_health.churn_30d.is_empty() || !profile.git_health.reverts.is_empty() {
        sections.push("### Git Health (30d / 180d)".to_string());
        for entry in profile.git_health.churn_30d.iter().take(5) {
            let c180 = profile
                .git_health
                .churn_180d
                .iter()
                .find(|e| e.path == entry.path)
                .map(|e| e.modifications)
                .unwrap_or(0);
            let trend = profile
                .git_health
                .churn_trend
                .get(&entry.path)
                .map(|t| format!("{t:?}").to_lowercase())
                .unwrap_or_else(|| "unknown".into());
            sections.push(format!(
                "- {} ({} / {}) — {trend}",
                entry.path, entry.modifications, c180
            ));
        }
        if !profile.git_health.reverts.is_empty() {
            sections.push(format!("- Reverts: {}×", profile.git_health.reverts.len()));
            for revert in &profile.git_health.reverts {
                if let Some(ref orig) = revert.reverted_message {
                    sections.push(format!("  - Reverted: {orig}"));
                } else {
                    sections.push(format!("  - {}", revert.commit_message));
                }
            }
        }
        sections.push(String::new());
    }

    sections.join("\n")
}

/// Render a compact DNA section (~200-300 tokens).
///
/// Top 3 Convention-strength patterns only. Used when budget is 2000-5000 tokens.
pub fn render_compact_dna(profile: &ConventionProfile) -> String {
    let mut lines = vec!["## Repository DNA (compact)\n".to_string()];
    let mut count = 0;

    // Collect all Convention-strength observations
    let observations: Vec<(&str, &crate::conventions::PatternObservation)> = [
        profile
            .naming
            .function_style
            .as_ref()
            .map(|o| ("Function naming", o)),
        profile
            .naming
            .type_style
            .as_ref()
            .map(|o| ("Type naming", o)),
        profile
            .errors
            .result_return
            .as_ref()
            .map(|o| ("Error handling", o)),
        profile
            .errors
            .unwrap_usage
            .as_ref()
            .map(|o| ("Unwrap policy", o)),
        profile.imports.style.as_ref().map(|o| ("Import style", o)),
    ]
    .into_iter()
    .flatten()
    .filter(|(_, o)| matches!(o.strength, PatternStrength::Convention))
    .collect();

    for (label, obs) in observations {
        if count >= 3 {
            break;
        }
        lines.push(format!(
            "- {label}: {} ({:.0}%)",
            obs.dominant, obs.percentage
        ));
        count += 1;
    }

    if count == 0 {
        lines.push("- No strong conventions detected yet".to_string());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::counter::TokenCounter;
    use crate::conventions::PatternObservation;

    #[test]
    fn test_render_dna_empty_profile() {
        let profile = ConventionProfile::default();
        let dna = render_dna_section(&profile);
        assert!(dna.contains("Repository DNA"));
    }

    #[test]
    fn test_render_dna_includes_conventions() {
        let mut profile = ConventionProfile::default();
        profile.naming.function_style = PatternObservation::new("fn_naming", "snake_case", 95, 100);
        let dna = render_dna_section(&profile);
        assert!(dna.contains("snake_case"));
        assert!(dna.contains("95"));
    }

    #[test]
    fn test_render_dna_includes_trends() {
        let mut profile = ConventionProfile::default();
        profile.visibility.public_ratio = PatternObservation::new("visibility", "private", 75, 100);
        let dna = render_dna_section(&profile);
        assert!(dna.contains("private"));
    }

    #[test]
    fn test_render_dna_excludes_mixed() {
        let mut profile = ConventionProfile::default();
        profile.naming.function_style = PatternObservation::new("fn_naming", "snake_case", 55, 100);
        let dna = render_dna_section(&profile);
        // Mixed strength should not appear in naming section
        assert!(!dna.contains("### Naming"));
    }

    #[test]
    fn test_render_dna_under_1200_tokens() {
        let counter = TokenCounter::new();
        let mut profile = ConventionProfile::default();
        profile.naming.function_style = PatternObservation::new("fn_naming", "snake_case", 95, 100);
        profile.naming.type_style = PatternObservation::new("type_naming", "PascalCase", 98, 100);
        profile.errors.result_return =
            PatternObservation::new("result_return", "Result<T, E>", 90, 100);

        let dna = render_dna_section(&profile);
        let token_count = counter.count(&dna);
        assert!(
            token_count <= 1200,
            "DNA section should be under 1200 tokens, got {token_count}"
        );
    }

    #[test]
    fn test_render_compact_dna() {
        let mut profile = ConventionProfile::default();
        profile.naming.function_style = PatternObservation::new("fn_naming", "snake_case", 95, 100);
        profile.naming.type_style = PatternObservation::new("type_naming", "PascalCase", 98, 100);
        profile.errors.result_return =
            PatternObservation::new("result_return", "Result<T, E>", 92, 100);
        profile.imports.style = PatternObservation::new("import_style", "absolute", 96, 100);

        let compact = render_compact_dna(&profile);
        assert!(compact.contains("compact"));
        // At most 3 lines of conventions
        let convention_lines: Vec<&str> = compact.lines().filter(|l| l.starts_with("- ")).collect();
        assert!(convention_lines.len() <= 3);
    }

    #[test]
    fn test_render_compact_dna_empty() {
        let profile = ConventionProfile::default();
        let compact = render_compact_dna(&profile);
        assert!(compact.contains("No strong conventions"));
    }

    #[test]
    fn test_render_dna_dependencies_strict_layers() {
        use crate::conventions::deps::{DependencyConventions, DirectionPair};

        let profile = ConventionProfile {
            dependencies: DependencyConventions {
                strict_layers: vec![DirectionPair {
                    from: "src".to_string(),
                    to: "core".to_string(),
                    edge_count: 12,
                    reverse_count: 0,
                }],
                circular_deps: vec![],
                db_isolation: None,
                additional: vec![],
            },
            ..Default::default()
        };

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Architecture"));
        assert!(dna.contains("src"));
        assert!(dna.contains("core"));
        assert!(dna.contains("12"));
        assert!(dna.contains("No circular deps"));
    }

    #[test]
    fn test_render_dna_dependencies_with_circular() {
        use crate::conventions::deps::{DependencyConventions, DirectionPair};

        let profile = ConventionProfile {
            dependencies: DependencyConventions {
                strict_layers: vec![DirectionPair {
                    from: "a".to_string(),
                    to: "b".to_string(),
                    edge_count: 3,
                    reverse_count: 1,
                }],
                circular_deps: vec!["a → b → a".to_string()],
                db_isolation: None,
                additional: vec![],
            },
            ..Default::default()
        };

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Circular"));
        assert!(dna.contains("a → b → a"));
    }

    #[test]
    fn test_render_dna_visibility_with_doc_coverage() {
        let mut profile = ConventionProfile::default();
        profile.visibility.public_ratio = PatternObservation::new("visibility", "private", 80, 100);
        profile.visibility.doc_comment_coverage =
            PatternObservation::new("doc_coverage", "documented public APIs", 70, 100);

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Visibility"));
        assert!(dna.contains("Doc comments"));
    }

    #[test]
    fn test_render_dna_functions_section() {
        use crate::conventions::functions::{DirectoryFunctionStats, FunctionConventions};
        use std::collections::HashMap;

        let mut profile = ConventionProfile::default();
        let mut by_directory = HashMap::new();
        by_directory.insert(
            "src/api".to_string(),
            DirectoryFunctionStats {
                avg_length: 15.0,
                median_length: 12.0,
                count: 5,
            },
        );
        profile.functions = FunctionConventions {
            avg_length: Some(15.0),
            median_length: Some(12.0),
            by_directory,
            additional: vec![],
            file_contributions: HashMap::new(),
        };

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Functions"));
        assert!(dna.contains("15"));
        assert!(dna.contains("src/api"));
    }

    #[test]
    fn test_render_dna_testing_section_with_naming_and_density() {
        let mut profile = ConventionProfile::default();
        profile.testing.mock_usage = PatternObservation::new("mock_usage", "no mocks", 9, 10);
        profile.testing.test_naming =
            PatternObservation::new("test_naming", "test_{fn}_{scenario}", 8, 10);
        profile.testing.density = Some(crate::conventions::PatternObservation {
            name: "test_density".into(),
            dominant: "2.0 tests/public fn".into(),
            count: 20,
            total: 10,
            percentage: 200.0,
            strength: crate::conventions::PatternStrength::Trend,
            exceptions: vec![],
        });

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Testing"));
        assert!(dna.contains("no mocks"));
        assert!(dna.contains("test_{fn}_{scenario}"));
        assert!(dna.contains("2.0 tests/public fn"));
    }

    #[test]
    fn test_render_dna_git_health_with_reverts() {
        use crate::conventions::git_health::{ChurnEntry, GitHealthProfile, RevertEntry};

        let profile = ConventionProfile {
            git_health: GitHealthProfile {
                churn_30d: vec![ChurnEntry {
                    path: "src/hot_file.rs".to_string(),
                    modifications: 42,
                }],
                churn_180d: vec![ChurnEntry {
                    path: "src/hot_file.rs".to_string(),
                    modifications: 120,
                }],
                reverts: vec![RevertEntry {
                    commit_message: "Revert bad change".to_string(),
                    reverted_message: Some("Add broken feature".to_string()),
                }],
                bugfix_density: std::collections::HashMap::new(),
                churn_trend: std::collections::HashMap::new(),
                co_changes: vec![],
                last_computed: None,
            },
            ..Default::default()
        };

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Git Health"));
        assert!(dna.contains("src/hot_file.rs"));
        assert!(dna.contains("42"));
        assert!(dna.contains("Reverts"));
        assert!(dna.contains("Add broken feature"));
    }

    #[test]
    fn test_render_dna_git_health_revert_no_original_message() {
        use crate::conventions::git_health::{ChurnEntry, GitHealthProfile, RevertEntry};

        let profile = ConventionProfile {
            git_health: GitHealthProfile {
                churn_30d: vec![ChurnEntry {
                    path: "src/lib.rs".to_string(),
                    modifications: 10,
                }],
                churn_180d: vec![],
                reverts: vec![RevertEntry {
                    commit_message: "Revert: undo something".to_string(),
                    reverted_message: None, // no original message
                }],
                bugfix_density: std::collections::HashMap::new(),
                churn_trend: std::collections::HashMap::new(),
                co_changes: vec![],
                last_computed: None,
            },
            ..Default::default()
        };

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Revert: undo something"));
    }

    #[test]
    fn test_render_dna_type_style_and_file_style_and_constant_style() {
        let mut profile = ConventionProfile::default();
        profile.naming.type_style = PatternObservation::new("type_naming", "PascalCase", 95, 100);
        profile.naming.file_style = PatternObservation::new("file_naming", "snake_case", 90, 100);
        profile.naming.constant_style =
            PatternObservation::new("constant_naming", "SCREAMING_SNAKE_CASE", 92, 100);

        let dna = render_dna_section(&profile);
        assert!(dna.contains("PascalCase"));
        assert!(dna.contains("snake_case"));
        assert!(dna.contains("SCREAMING_SNAKE_CASE"));
    }

    #[test]
    fn test_render_dna_error_handling_all_fields() {
        let mut profile = ConventionProfile::default();
        profile.errors.result_return =
            PatternObservation::new("result_return", "Result<T, E>", 90, 100);
        profile.errors.unwrap_usage =
            PatternObservation::new("unwrap_usage", "no .unwrap() in src/", 88, 100);
        profile.errors.question_mark_propagation =
            PatternObservation::new("question_mark", "? operator", 85, 100);

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Error Handling"));
        assert!(dna.contains("Result<T, E>"));
        assert!(dna.contains("no .unwrap() in src/"));
        assert!(dna.contains("? operator"));
    }

    #[test]
    fn test_render_dna_imports_section() {
        let mut profile = ConventionProfile::default();
        profile.imports.style = PatternObservation::new("import_style", "absolute", 95, 100);

        let dna = render_dna_section(&profile);
        assert!(dna.contains("Imports"));
        assert!(dna.contains("absolute"));
    }

    #[test]
    fn test_render_compact_dna_caps_at_three() {
        // With 5 Convention-strength observations, only 3 are rendered.
        let mut profile = ConventionProfile::default();
        profile.naming.function_style = PatternObservation::new("fn_naming", "snake_case", 95, 100);
        profile.naming.type_style = PatternObservation::new("type_naming", "PascalCase", 98, 100);
        profile.errors.result_return =
            PatternObservation::new("result_return", "Result<T, E>", 92, 100);
        profile.errors.unwrap_usage =
            PatternObservation::new("unwrap_usage", "no .unwrap() in src/", 91, 100);
        profile.imports.style = PatternObservation::new("import_style", "absolute", 96, 100);

        let compact = render_compact_dna(&profile);
        let lines: Vec<&str> = compact.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 3);
    }

    // Bug 3 regression: the git-health churn format string previously contained the
    // path field twice ("- {}: {} ({} / {}) — {trend}"), causing the file path to
    // appear as a duplicate label before the modification count.  After the fix the
    // line format is "- {path} ({modifications} / {churn_180d}) — {trend}" — only one
    // occurrence of the path.
    #[test]
    fn test_render_dna_churn_line_no_duplicate_path_label() {
        use crate::conventions::git_health::{ChurnEntry, ChurnTrend, GitHealthProfile};
        let git_health = GitHealthProfile {
            churn_30d: vec![ChurnEntry {
                path: "src/main.rs".to_string(),
                modifications: 42,
            }],
            churn_180d: vec![ChurnEntry {
                path: "src/main.rs".to_string(),
                modifications: 15,
            }],
            churn_trend: {
                let mut m = std::collections::HashMap::new();
                m.insert("src/main.rs".to_string(), ChurnTrend::Hot);
                m
            },
            ..Default::default()
        };
        let profile = ConventionProfile {
            git_health,
            ..Default::default()
        };

        let dna = render_dna_section(&profile);

        // Find the churn line for src/main.rs.
        let churn_lines: Vec<&str> = dna.lines().filter(|l| l.contains("src/main.rs")).collect();
        assert!(
            !churn_lines.is_empty(),
            "churn line for src/main.rs must appear"
        );
        for line in &churn_lines {
            // The path must appear exactly once — no "src/main.rs: src/main.rs ..." duplicate.
            let occurrences = line.matches("src/main.rs").count();
            assert_eq!(
                occurrences, 1,
                "path must appear exactly once per churn line, got: {line}"
            );
        }
    }

    // Bug 7 regression: when `function_style` is None but other naming observations
    // exist (e.g. type_style), the DNA section previously used `"trend"` as a literal
    // fallback for the Naming section header strength label.  After the fix it must
    // emit `"not detected"` instead.
    #[test]
    fn test_render_dna_missing_function_style_shows_not_detected() {
        let mut profile = ConventionProfile::default();
        // Set type_style but NOT function_style — this forces the "no function_style"
        // branch in the Naming section header rendering.
        profile.naming.type_style = PatternObservation::new("type_naming", "PascalCase", 95, 100);

        let dna = render_dna_section(&profile);
        // The naming section header should appear (because type_style is set).
        assert!(
            dna.contains("Naming"),
            "Naming section header must be present when type_style is set"
        );
        // The header must say "not detected" for the absent function_style strength.
        assert!(
            dna.contains("not detected"),
            "absent function_style must render as 'not detected' in Naming header, got:\n{dna}"
        );
        // The bare word "trend" must NOT appear as a spurious fallback in the header.
        for line in dna.lines() {
            if line.contains("### Naming") {
                assert!(
                    !line.ends_with(" trend)") && !line.contains("(trend)"),
                    "bare 'trend' fallback must not appear in Naming header: {line}"
                );
            }
        }
    }
}
