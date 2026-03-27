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
            .unwrap_or_else(|| "trend".into());
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
            .unwrap_or_else(|| "trend".into());
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
                "- {}: {} ({} / {}) — {trend}",
                trend, entry.path, entry.modifications, c180
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
}
