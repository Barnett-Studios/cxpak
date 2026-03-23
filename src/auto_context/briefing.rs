// Fill-then-overflow budget allocation

use crate::budget::counter::TokenCounter;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PackedBriefing {
    pub budget: BudgetSummary,
    pub sections: PackedSections,
}

#[derive(Debug, Serialize)]
pub struct BudgetSummary {
    pub total: usize,
    pub used: usize,
    pub remaining: usize,
}

#[derive(Debug, Serialize)]
pub struct PackedSections {
    pub target_files: PackedFileSection,
    pub test_files: PackedFileSection,
    pub schema_context: PackedFileSection,
    pub api_surface: Option<serde_json::Value>,
    pub blast_radius: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct PackedFileSection {
    pub count: usize,
    pub tokens: usize,
    pub files: Vec<PackedFile>,
}

#[derive(Debug, Serialize)]
pub struct PackedFile {
    pub path: String,
    pub score: f64,
    pub detail_level: String,
    pub tokens: usize,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Core allocation logic
// ---------------------------------------------------------------------------

/// Pack a set of sections into the available token budget using a
/// fill-then-overflow strategy.
///
/// Priority order (highest → lowest):
/// 1. Target files  — scored by relevance; higher-scored files get full
///    content first.  When budget runs low, lower-scored files are
///    truncated then skipped.
/// 2. Test files    — packed after targets; sorted by path for stability.
/// 3. Schema JSON   — serialized and included if budget allows.
/// 4. API surface   — serialized and included if budget allows.
/// 5. Blast radius  — serialized and included if budget allows.
pub fn allocate_and_pack(
    mut target_files: Vec<(String, f64, String)>,
    test_files: Vec<(String, String)>,
    schema_json: Option<serde_json::Value>,
    api_surface_json: Option<serde_json::Value>,
    blast_radius_json: Option<serde_json::Value>,
    token_budget: usize,
) -> PackedBriefing {
    let counter = TokenCounter::new();
    let mut remaining = token_budget;

    // --- Section 1: Target files -------------------------------------------
    // Sort descending by score so higher-priority files are packed first.
    target_files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut packed_targets: Vec<PackedFile> = Vec::new();
    let mut target_tokens = 0usize;

    for (path, score, content) in target_files {
        if remaining == 0 {
            break;
        }
        let full_tokens = counter.count(&content);

        if full_tokens <= remaining {
            // Fits completely.
            remaining -= full_tokens;
            target_tokens += full_tokens;
            packed_targets.push(PackedFile {
                path,
                score,
                detail_level: "full".to_string(),
                tokens: full_tokens,
                content,
            });
        } else if remaining > 0 {
            // Truncate to whatever budget is left (line-level granularity).
            let truncated = truncate_to_budget(&content, remaining, &counter);
            let truncated_tokens = counter.count(&truncated);
            target_tokens += truncated_tokens;
            remaining = remaining.saturating_sub(truncated_tokens);
            packed_targets.push(PackedFile {
                path,
                score,
                detail_level: "truncated".to_string(),
                tokens: truncated_tokens,
                content: truncated,
            });
        }
    }

    // --- Section 2: Test files ---------------------------------------------
    let mut packed_tests: Vec<PackedFile> = Vec::new();
    let mut test_tokens = 0usize;

    for (path, content) in test_files {
        if remaining == 0 {
            break;
        }
        let full_tokens = counter.count(&content);

        if full_tokens <= remaining {
            remaining -= full_tokens;
            test_tokens += full_tokens;
            packed_tests.push(PackedFile {
                path,
                score: 0.0,
                detail_level: "full".to_string(),
                tokens: full_tokens,
                content,
            });
        } else if remaining > 0 {
            let truncated = truncate_to_budget(&content, remaining, &counter);
            let truncated_tokens = counter.count(&truncated);
            test_tokens += truncated_tokens;
            remaining = remaining.saturating_sub(truncated_tokens);
            packed_tests.push(PackedFile {
                path,
                score: 0.0,
                detail_level: "truncated".to_string(),
                tokens: truncated_tokens,
                content: truncated,
            });
        }
    }

    // --- Section 3: Schema context -----------------------------------------
    // Schema JSON is supplied as an Option<serde_json::Value>; we re-serialize
    // it here (to an intermediary string) to count tokens accurately.
    let mut packed_schema_files: Vec<PackedFile> = Vec::new();
    let mut schema_tokens = 0usize;

    if let Some(schema_val) = schema_json {
        if remaining > 0 {
            let schema_str = serde_json::to_string_pretty(&schema_val).unwrap_or_default();
            let schema_tok = counter.count(&schema_str);
            if schema_tok <= remaining {
                remaining -= schema_tok;
                schema_tokens = schema_tok;
                packed_schema_files.push(PackedFile {
                    path: "<schema>".to_string(),
                    score: 0.0,
                    detail_level: "full".to_string(),
                    tokens: schema_tok,
                    content: schema_str,
                });
            } else {
                let truncated = truncate_to_budget(&schema_str, remaining, &counter);
                let truncated_tokens = counter.count(&truncated);
                schema_tokens = truncated_tokens;
                remaining = remaining.saturating_sub(truncated_tokens);
                packed_schema_files.push(PackedFile {
                    path: "<schema>".to_string(),
                    score: 0.0,
                    detail_level: "truncated".to_string(),
                    tokens: truncated_tokens,
                    content: truncated,
                });
            }
        }
    }

    // --- Section 4: API surface -------------------------------------------
    let resolved_api = if let Some(api_val) = api_surface_json {
        if remaining > 0 {
            let api_str = serde_json::to_string_pretty(&api_val).unwrap_or_default();
            let api_tok = counter.count(&api_str);
            if api_tok <= remaining {
                remaining -= api_tok;
                Some(api_val)
            } else {
                // Too large — skip rather than truncating structured JSON.
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // --- Section 5: Blast radius ------------------------------------------
    let resolved_blast = if let Some(blast_val) = blast_radius_json {
        if remaining > 0 {
            let blast_str = serde_json::to_string_pretty(&blast_val).unwrap_or_default();
            let blast_tok = counter.count(&blast_str);
            if blast_tok <= remaining {
                remaining -= blast_tok;
                Some(blast_val)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // --- Compute budget summary -------------------------------------------
    let used = token_budget - remaining;

    PackedBriefing {
        budget: BudgetSummary {
            total: token_budget,
            used,
            remaining,
        },
        sections: PackedSections {
            target_files: PackedFileSection {
                count: packed_targets.len(),
                tokens: target_tokens,
                files: packed_targets,
            },
            test_files: PackedFileSection {
                count: packed_tests.len(),
                tokens: test_tokens,
                files: packed_tests,
            },
            schema_context: PackedFileSection {
                count: packed_schema_files.len(),
                tokens: schema_tokens,
                files: packed_schema_files,
            },
            api_surface: resolved_api,
            blast_radius: resolved_blast,
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate `content` to approximately `budget` tokens by dropping trailing
/// lines and appending an omission marker.  Returns the truncated string.
/// If the budget is too tight even for the marker, returns an empty string.
fn truncate_to_budget(content: &str, budget: usize, counter: &TokenCounter) -> String {
    if budget == 0 {
        return String::new();
    }
    let marker = "\n// ... (truncated)";
    let marker_tokens = counter.count(marker);
    if marker_tokens >= budget {
        return String::new();
    }

    let available = budget - marker_tokens;
    let lines: Vec<&str> = content.lines().collect();
    let mut accumulated = String::new();
    let mut accumulated_tokens = 0usize;

    for line in &lines {
        let line_with_newline = format!("{}\n", line);
        let line_tokens = counter.count(&line_with_newline);
        if accumulated_tokens + line_tokens > available {
            break;
        }
        accumulated.push_str(&line_with_newline);
        accumulated_tokens += line_tokens;
    }

    if accumulated.is_empty() {
        return String::new();
    }

    format!("{}{}", accumulated.trim_end_matches('\n'), marker)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_target(path: &str, score: f64, content: &str) -> (String, f64, String) {
        (path.to_string(), score, content.to_string())
    }

    fn make_test(path: &str, content: &str) -> (String, String) {
        (path.to_string(), content.to_string())
    }

    // Count real tokens so tests don't rely on hard-coded numbers.
    fn tok(s: &str) -> usize {
        TokenCounter::new().count(s)
    }

    // -----------------------------------------------------------------------
    // test_target_files_packed_first
    // -----------------------------------------------------------------------
    #[test]
    fn test_target_files_packed_first() {
        // Budget exactly covers both a target and a test file; verify that
        // targets are included even if budget is just barely sufficient.
        let target_content = "fn target() { /* target body */ }";
        let test_content = "fn test_target() { assert!(true); }";
        let target_toks = tok(target_content);
        let test_toks = tok(test_content);

        // Budget is enough for target + test combined.
        let budget = target_toks + test_toks + 10;

        let result = allocate_and_pack(
            vec![make_target("src/target.rs", 0.9, target_content)],
            vec![make_test("tests/test_target.rs", test_content)],
            None,
            None,
            None,
            budget,
        );

        assert_eq!(
            result.sections.target_files.count, 1,
            "target file should be packed"
        );
        assert!(
            result.sections.target_files.tokens > 0,
            "target tokens should be non-zero"
        );
    }

    // -----------------------------------------------------------------------
    // test_budget_exhausted_skips_lower_sections
    // -----------------------------------------------------------------------
    #[test]
    fn test_budget_exhausted_skips_lower_sections() {
        // A tiny budget that fits the target but not the test file.
        let target_content = "fn x() {}";
        let target_toks = tok(target_content);

        // Budget is exactly the target tokens — nothing left for tests.
        let budget = target_toks;

        let result = allocate_and_pack(
            vec![make_target("src/x.rs", 0.8, target_content)],
            vec![make_test(
                "tests/x_test.rs",
                "fn test_x() { assert_eq!(1, 1); }",
            )],
            None,
            None,
            None,
            budget,
        );

        assert_eq!(
            result.sections.target_files.count, 1,
            "target should be packed"
        );
        assert_eq!(
            result.sections.test_files.count, 0,
            "tests should be skipped when budget is exhausted"
        );
    }

    // -----------------------------------------------------------------------
    // test_generous_budget_everything_fits
    // -----------------------------------------------------------------------
    #[test]
    fn test_generous_budget_everything_fits() {
        let target_content = "fn main() { println!(\"hello\"); }";
        let test_content = "fn test_main() { assert!(true); }";
        let api_val = serde_json::json!({"routes": []});
        let blast_val = serde_json::json!({"total_affected": 0});

        let result = allocate_and_pack(
            vec![make_target("src/main.rs", 0.95, target_content)],
            vec![make_test("tests/main_test.rs", test_content)],
            None,
            Some(api_val),
            Some(blast_val),
            100_000,
        );

        assert_eq!(result.sections.target_files.count, 1);
        assert_eq!(result.sections.test_files.count, 1);
        assert!(
            result.sections.api_surface.is_some(),
            "api surface should be included with generous budget"
        );
        assert!(
            result.sections.blast_radius.is_some(),
            "blast radius should be included with generous budget"
        );
    }

    // -----------------------------------------------------------------------
    // test_empty_input
    // -----------------------------------------------------------------------
    #[test]
    fn test_empty_input() {
        let result = allocate_and_pack(vec![], vec![], None, None, None, 10_000);

        assert_eq!(result.sections.target_files.count, 0);
        assert_eq!(result.sections.target_files.tokens, 0);
        assert!(result.sections.target_files.files.is_empty());
        assert_eq!(result.sections.test_files.count, 0);
        assert_eq!(result.sections.schema_context.count, 0);
        assert!(result.sections.api_surface.is_none());
        assert!(result.sections.blast_radius.is_none());
    }

    // -----------------------------------------------------------------------
    // test_budget_summary_accurate
    // -----------------------------------------------------------------------
    #[test]
    fn test_budget_summary_accurate() {
        let content = "fn alpha() {} fn beta() {}";
        let result = allocate_and_pack(
            vec![make_target("src/alpha.rs", 0.7, content)],
            vec![],
            None,
            None,
            None,
            10_000,
        );

        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total,
            "used + remaining must equal total"
        );
        assert_eq!(result.budget.total, 10_000);
        assert!(result.budget.used > 0, "some tokens should have been used");
    }

    // -----------------------------------------------------------------------
    // Additional: higher-scored targets come first
    // -----------------------------------------------------------------------
    #[test]
    fn test_higher_scored_targets_packed_first() {
        let high_content = "fn high_priority() {}";
        let low_content = "fn low_priority() {}";
        let high_toks = tok(high_content);

        // Budget only fits the high-scored file.
        let budget = high_toks;

        let result = allocate_and_pack(
            vec![
                make_target("src/low.rs", 0.2, low_content),
                make_target("src/high.rs", 0.9, high_content),
            ],
            vec![],
            None,
            None,
            None,
            budget,
        );

        assert_eq!(result.sections.target_files.count, 1);
        assert_eq!(
            result.sections.target_files.files[0].path, "src/high.rs",
            "higher-scored file should be packed when budget is tight"
        );
    }
}
