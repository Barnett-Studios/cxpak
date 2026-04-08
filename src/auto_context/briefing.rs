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
    /// Cross-language boundary edges (v1.5.0). Populated when the index has
    /// detected cross-language bridges within the focus scope. Up to 500
    /// tokens worth of edges are included; the rest are dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_language_edges: Option<serde_json::Value>,
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
    pub content: Option<String>,
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
    target_files: Vec<(String, f64, String)>,
    test_files: Vec<(String, String)>,
    schema_json: Option<serde_json::Value>,
    api_surface_json: Option<serde_json::Value>,
    blast_radius_json: Option<serde_json::Value>,
    token_budget: usize,
    briefing_mode: bool,
) -> PackedBriefing {
    allocate_and_pack_with_cross_lang(
        target_files,
        test_files,
        schema_json,
        api_surface_json,
        blast_radius_json,
        None,
        token_budget,
        briefing_mode,
    )
}

/// Same as [`allocate_and_pack`] but with an extra cross-language edges
/// section. Kept as a separate function so existing callers stay binary
/// compatible.
#[allow(clippy::too_many_arguments)]
pub fn allocate_and_pack_with_cross_lang(
    mut target_files: Vec<(String, f64, String)>,
    test_files: Vec<(String, String)>,
    schema_json: Option<serde_json::Value>,
    api_surface_json: Option<serde_json::Value>,
    blast_radius_json: Option<serde_json::Value>,
    cross_lang_json: Option<serde_json::Value>,
    token_budget: usize,
    briefing_mode: bool,
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
                content: if briefing_mode { None } else { Some(content) },
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
                content: if briefing_mode { None } else { Some(truncated) },
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
                content: if briefing_mode { None } else { Some(content) },
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
                content: if briefing_mode { None } else { Some(truncated) },
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
                    content: if briefing_mode {
                        None
                    } else {
                        Some(schema_str)
                    },
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
                    content: if briefing_mode { None } else { Some(truncated) },
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

    // --- Section 6: Cross-language edges (v1.5.0) --------------------------
    // Dedicated section; capped at min(remaining, 500) tokens so it never
    // starves the primary budget allocations above. Structured JSON is kept
    // intact (no truncation) because LLMs rely on the structure for routing.
    let resolved_cross_lang = if let Some(cross_val) = cross_lang_json {
        let cap = remaining.min(500);
        if cap > 0 {
            let cross_str = serde_json::to_string_pretty(&cross_val).unwrap_or_default();
            let cross_tok = counter.count(&cross_str);
            if cross_tok <= cap {
                remaining = remaining.saturating_sub(cross_tok);
                Some(cross_val)
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
            cross_language_edges: resolved_cross_lang,
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
            false,
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
            false,
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
            false,
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
        let result = allocate_and_pack(vec![], vec![], None, None, None, 10_000, false);

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
            false,
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
            false,
        );

        assert_eq!(result.sections.target_files.count, 1);
        assert_eq!(
            result.sections.target_files.files[0].path, "src/high.rs",
            "higher-scored file should be packed when budget is tight"
        );
        assert!(
            result.sections.target_files.files[0].content.is_some(),
            "content must be Some in full mode"
        );
    }

    // -----------------------------------------------------------------------
    // test_truncate_to_budget_helper
    // -----------------------------------------------------------------------
    #[test]
    fn test_truncate_to_budget_zero() {
        let counter = TokenCounter::new();
        let result = truncate_to_budget("some content\nmore lines\n", 0, &counter);
        assert_eq!(result, "", "zero budget should produce empty string");
    }

    #[test]
    fn test_truncate_to_budget_very_small() {
        let counter = TokenCounter::new();
        // Budget so small even the marker doesn't fit.
        let result = truncate_to_budget("some content\nmore lines\n", 1, &counter);
        assert_eq!(
            result, "",
            "budget smaller than marker should produce empty string"
        );
    }

    #[test]
    fn test_truncate_to_budget_partial() {
        let counter = TokenCounter::new();
        // Use many lines so that even 3/4 of the budget triggers truncation but
        // still leaves room for at least one line plus the marker.
        let content = (1..=20)
            .map(|i| format!("line number {} has enough text to cost several tokens", i))
            .collect::<Vec<_>>()
            .join("\n");
        let full_tokens = tok(&content);
        // 3/4 budget: enough for the marker + some lines, but not all.
        let budget = (full_tokens * 3) / 4;
        let result = truncate_to_budget(&content, budget, &counter);
        assert!(
            result.contains("// ... (truncated)"),
            "truncated output should contain omission marker"
        );
        assert!(
            result.len() < content.len(),
            "truncated output should be shorter than original"
        );
    }

    // -----------------------------------------------------------------------
    // test_target_file_truncated_when_budget_tight
    // -----------------------------------------------------------------------
    #[test]
    fn test_target_file_truncated_when_budget_tight() {
        // Use many lines so there's enough room for the marker + some lines.
        let content = (1..=20)
            .map(|i| format!("fn line_{}() {{ /* body {} */ }}", i, i))
            .collect::<Vec<_>>()
            .join("\n");
        let full_tokens = tok(&content);
        // 3/4 budget: enough for some lines + marker, but not all.
        let budget = (full_tokens * 3) / 4;

        let result = allocate_and_pack(
            vec![make_target("src/big.rs", 0.9, &content)],
            vec![],
            None,
            None,
            None,
            budget,
            false,
        );

        assert_eq!(result.sections.target_files.count, 1);
        assert_eq!(
            result.sections.target_files.files[0].detail_level, "truncated",
            "file should be truncated when budget is tight"
        );
        let packed_content = result.sections.target_files.files[0]
            .content
            .as_ref()
            .unwrap();
        assert!(
            packed_content.contains("// ... (truncated)"),
            "truncated content should include omission marker"
        );
    }

    // -----------------------------------------------------------------------
    // test_test_files_truncated_when_budget_tight
    // -----------------------------------------------------------------------
    #[test]
    fn test_test_files_truncated_when_budget_tight() {
        let test_content =
            "fn test_a() { assert!(true); }\nfn test_b() { assert!(true); }\nfn test_c() {}";
        let test_tokens = tok(test_content);
        // Budget only covers a fraction of the test file (no targets to consume budget first).
        let budget = test_tokens / 2;

        let result = allocate_and_pack(
            vec![],
            vec![make_test("tests/big.rs", test_content)],
            None,
            None,
            None,
            budget,
            false,
        );

        assert_eq!(result.sections.test_files.count, 1);
        assert_eq!(
            result.sections.test_files.files[0].detail_level, "truncated",
            "test file should be truncated"
        );
    }

    // -----------------------------------------------------------------------
    // test_schema_section_packed
    // -----------------------------------------------------------------------
    #[test]
    fn test_schema_section_packed_full() {
        let schema_val = serde_json::json!({
            "tables": ["users", "orders"],
            "views": []
        });

        let result =
            allocate_and_pack(vec![], vec![], Some(schema_val), None, None, 100_000, false);

        assert_eq!(result.sections.schema_context.count, 1);
        assert_eq!(
            result.sections.schema_context.files[0].detail_level, "full",
            "schema should be fully packed with generous budget"
        );
        assert_eq!(result.sections.schema_context.files[0].path, "<schema>");
        assert!(
            result.sections.schema_context.files[0].content.is_some(),
            "content should be present in non-briefing mode"
        );
    }

    #[test]
    fn test_schema_section_truncated_when_tight() {
        let schema_val = serde_json::json!({
            "tables": ["users", "orders", "products", "categories", "reviews", "inventory"],
            "columns": {
                "users": ["id", "name", "email", "created_at"],
                "orders": ["id", "user_id", "total", "status"]
            }
        });
        let schema_str = serde_json::to_string_pretty(&schema_val).unwrap();
        let schema_tokens = tok(&schema_str);

        // Budget smaller than the full schema but > 0.
        let budget = schema_tokens / 2;

        let result = allocate_and_pack(vec![], vec![], Some(schema_val), None, None, budget, false);

        assert_eq!(result.sections.schema_context.count, 1);
        assert_eq!(
            result.sections.schema_context.files[0].detail_level, "truncated",
            "schema should be truncated when budget is tight"
        );
    }

    // -----------------------------------------------------------------------
    // test_api_surface_skipped_when_too_large
    // -----------------------------------------------------------------------
    #[test]
    fn test_api_surface_skipped_when_too_large() {
        let api_val = serde_json::json!({
            "endpoints": [
                {"method": "GET", "path": "/users"},
                {"method": "POST", "path": "/users"},
                {"method": "GET", "path": "/orders"},
                {"method": "DELETE", "path": "/orders/:id"}
            ]
        });
        // Tiny budget: API surface JSON won't fit.
        let result = allocate_and_pack(vec![], vec![], None, Some(api_val), None, 2, false);

        assert!(
            result.sections.api_surface.is_none(),
            "api surface should be skipped when it doesn't fit the budget"
        );
    }

    #[test]
    fn test_api_surface_included_when_fits() {
        let api_val = serde_json::json!({"routes": ["/health"]});
        let result = allocate_and_pack(
            vec![],
            vec![],
            None,
            Some(api_val.clone()),
            None,
            100_000,
            false,
        );

        assert!(
            result.sections.api_surface.is_some(),
            "api surface should be included when budget is generous"
        );
        assert_eq!(result.sections.api_surface.unwrap(), api_val);
    }

    // -----------------------------------------------------------------------
    // test_blast_radius_skipped_when_too_large
    // -----------------------------------------------------------------------
    #[test]
    fn test_blast_radius_skipped_when_too_large() {
        let blast_val = serde_json::json!({
            "direct_dependents": ["a.rs", "b.rs", "c.rs"],
            "transitive_dependents": ["d.rs", "e.rs"]
        });

        let result = allocate_and_pack(vec![], vec![], None, None, Some(blast_val), 2, false);

        assert!(
            result.sections.blast_radius.is_none(),
            "blast radius should be skipped when budget is too small"
        );
    }

    #[test]
    fn test_blast_radius_included_when_fits() {
        let blast_val = serde_json::json!({"affected": 0});
        let result = allocate_and_pack(
            vec![],
            vec![],
            None,
            None,
            Some(blast_val.clone()),
            100_000,
            false,
        );

        assert!(
            result.sections.blast_radius.is_some(),
            "blast radius should be included when budget is generous"
        );
        assert_eq!(result.sections.blast_radius.unwrap(), blast_val);
    }

    // -----------------------------------------------------------------------
    // test_briefing_mode_suppresses_content
    // -----------------------------------------------------------------------
    #[test]
    fn test_briefing_mode_suppresses_content() {
        let target_content = "fn target() { /* body */ }";
        let test_content = "fn test_target() { assert!(true); }";
        let schema_val = serde_json::json!({"tables": ["users"]});

        let result = allocate_and_pack(
            vec![make_target("src/main.rs", 0.9, target_content)],
            vec![make_test("tests/main.rs", test_content)],
            Some(schema_val),
            None,
            None,
            100_000,
            true, // briefing_mode
        );

        assert_eq!(result.sections.target_files.count, 1);
        assert!(
            result.sections.target_files.files[0].content.is_none(),
            "briefing mode should suppress target file content"
        );
        assert_eq!(result.sections.test_files.count, 1);
        assert!(
            result.sections.test_files.files[0].content.is_none(),
            "briefing mode should suppress test file content"
        );
        assert_eq!(result.sections.schema_context.count, 1);
        assert!(
            result.sections.schema_context.files[0].content.is_none(),
            "briefing mode should suppress schema content"
        );
    }

    // -----------------------------------------------------------------------
    // test_multiple_targets_fill_then_overflow
    // -----------------------------------------------------------------------
    #[test]
    fn test_multiple_targets_fill_then_overflow() {
        let content_a = "fn alpha() { /* alpha body */ }";
        let content_b = "fn beta() { /* beta body here too */ }";
        let content_c = "fn gamma() { /* gamma body content */ }";
        let toks_a = tok(content_a);
        let toks_b = tok(content_b);

        // Budget fits exactly two files (a and b) but not c.
        let budget = toks_a + toks_b;

        let result = allocate_and_pack(
            vec![
                make_target("src/a.rs", 0.9, content_a),
                make_target("src/b.rs", 0.8, content_b),
                make_target("src/c.rs", 0.7, content_c),
            ],
            vec![],
            None,
            None,
            None,
            budget,
            false,
        );

        // Two files fit fully; third is skipped or truncated.
        assert_eq!(
            result.sections.target_files.count, 2,
            "only two files should fit in the budget"
        );
        assert_eq!(result.sections.target_files.files[0].path, "src/a.rs");
        assert_eq!(result.sections.target_files.files[1].path, "src/b.rs");
        assert_eq!(result.sections.target_files.files[0].detail_level, "full");
        assert_eq!(result.sections.target_files.files[1].detail_level, "full");
    }

    // -----------------------------------------------------------------------
    // test_all_sections_compete_for_budget
    // -----------------------------------------------------------------------
    #[test]
    fn test_all_sections_compete_for_budget() {
        let target_content = "fn main() { println!(\"hello world\"); }";
        let test_content = "fn test_main() { assert_eq!(1, 1); }";
        let schema_val = serde_json::json!({"tables": ["users"]});
        let api_val = serde_json::json!({"routes": ["/api"]});
        let blast_val = serde_json::json!({"total": 1});

        let target_toks = tok(target_content);
        let test_toks = tok(test_content);
        let schema_toks = tok(&serde_json::to_string_pretty(&schema_val).unwrap());

        // Budget covers target + test + schema but not API or blast.
        let budget = target_toks + test_toks + schema_toks;

        let result = allocate_and_pack(
            vec![make_target("src/main.rs", 0.9, target_content)],
            vec![make_test("tests/main.rs", test_content)],
            Some(schema_val),
            Some(api_val),
            Some(blast_val),
            budget,
            false,
        );

        assert_eq!(result.sections.target_files.count, 1);
        assert_eq!(result.sections.test_files.count, 1);
        assert_eq!(result.sections.schema_context.count, 1);
        // API and blast are lower priority; they should be skipped/None.
        assert!(
            result.sections.api_surface.is_none(),
            "api surface should be skipped when budget is exhausted by higher-priority sections"
        );
        assert!(
            result.sections.blast_radius.is_none(),
            "blast radius should be skipped when budget is exhausted"
        );
        assert_eq!(
            result.budget.used + result.budget.remaining,
            result.budget.total,
            "budget accounting must be consistent"
        );
    }

    // -----------------------------------------------------------------------
    // test_zero_budget
    // -----------------------------------------------------------------------
    #[test]
    fn test_zero_budget() {
        let result = allocate_and_pack(
            vec![make_target("src/x.rs", 0.9, "fn x() { /* content */ }")],
            vec![make_test("tests/x.rs", "fn test_x() {}")],
            Some(serde_json::json!({"tables": []})),
            Some(serde_json::json!({"routes": []})),
            Some(serde_json::json!({"total": 0})),
            0,
            false,
        );

        assert_eq!(result.sections.target_files.count, 0);
        assert_eq!(result.sections.test_files.count, 0);
        assert_eq!(result.sections.schema_context.count, 0);
        assert!(result.sections.api_surface.is_none());
        assert!(result.sections.blast_radius.is_none());
        assert_eq!(result.budget.total, 0);
        assert_eq!(result.budget.used, 0);
        assert_eq!(result.budget.remaining, 0);
    }
}
