use crate::budget::counter::TokenCounter;
use crate::conventions::{ConventionProfile, PatternStrength};
use serde_json::Value;

// ── Token-budget constants ────────────────────────────────────────────────────

/// Default MCP/LSP/HTTP output-token cap for the conventions surface.
///
/// Justified in ADR-0183: the full `ConventionProfile` on a large repo serialises
/// to ~230 k tokens.  5 000 tokens is ≈ 46× smaller, enough to carry all
/// Convention/Trend-strength observations plus a representative top-N of git-health
/// data within one screenful of context — while staying well below a typical MCP
/// client's per-message limit (~8 k tokens).  It is intentionally different from
/// the 50 k default used by briefing/overview ops: those ops stream file content,
/// which warrants a larger budget; the conventions op streams structured metadata,
/// which is far denser but requires far less volume to be actionable.
pub const MAX_MCP_CONVENTIONS_TOKENS: usize = 5_000;

/// Headroom subtracted from `token_budget` for every "fits now?" check that
/// *precedes* `_omitted` marker injection.
///
/// The marker costs roughly 120–250 tokens (JSON object with `applied_budget`,
/// `original_tokens`, `steps_applied`, and `note`).  200 tokens is a
/// conservative upper bound across all step counts, ensuring the *returned*
/// output (value + marker) is always ≤ `token_budget`.
const MARKER_RESERVE: usize = 200;

/// Smallest `token_budget` for which the ≤-budget guarantee holds end-to-end.
///
/// Below this floor `render_budgeted_conventions` still returns a minimal
/// skeleton (`{}` + `_omitted` marker ≈ 200–280 tokens), but that skeleton
/// itself may marginally exceed the budget.  Callers that pass budgets below
/// this value should treat the output as best-effort.
pub const MIN_BUDGET_FLOOR: usize = 300;

/// Fixed category order for deterministic terminal degradation (Steps 4 and 6–8).
///
/// Using a static slice avoids `HashMap` / `HashSet` iteration-order leaking
/// into per-step log messages and keeps `render_budgeted_conventions` output
/// byte-identical across repeated calls for the same input.
const CATEGORIES: &[&str] = &[
    "naming",
    "imports",
    "errors",
    "dependencies",
    "testing",
    "visibility",
    "functions",
];

// ── Budgeted render core ──────────────────────────────────────────────────────

/// Navigate into a nested JSON object along `keys`, returning a mutable reference
/// to the final value if every intermediate key exists and is an Object.
fn get_nested_mut<'a>(value: &'a mut Value, keys: &[&str]) -> Option<&'a mut Value> {
    if keys.is_empty() {
        return Some(value);
    }
    value
        .as_object_mut()
        .and_then(|o| o.get_mut(keys[0]))
        .and_then(|v| get_nested_mut(v, &keys[1..]))
}

/// Count tokens in a `Value` serialised as pretty JSON.
fn token_count(counter: &TokenCounter, value: &Value) -> usize {
    counter.count(&serde_json::to_string_pretty(value).unwrap_or_default())
}

/// Drop the array at `path` inside `value`, recording the action in `steps`.
/// No-ops silently if the path does not exist or is already empty.
fn drop_array_at(value: &mut Value, path: &[&str], steps: &mut Vec<String>) {
    if let Some(v) = get_nested_mut(value, path) {
        if let Some(arr) = v.as_array() {
            let n = arr.len();
            if n > 0 {
                *v = Value::Array(vec![]);
                steps.push(format!("dropped {} ({n} entries)", path.join(".")));
            }
        }
    }
}

/// Truncate the array at `path` to `max` entries, recording the action in
/// `steps`.  No-ops if the array is already ≤ max.
fn truncate_array_at(value: &mut Value, path: &[&str], max: usize, steps: &mut Vec<String>) {
    if let Some(v) = get_nested_mut(value, path) {
        if let Some(arr) = v.as_array_mut() {
            if arr.len() > max {
                let dropped = arr.len() - max;
                arr.truncate(max);
                steps.push(format!(
                    "truncated {} to {max} entries ({dropped} dropped)",
                    path.join(".")
                ));
            }
        }
    }
}

/// Clear the object at `path` (replace with `{}`), recording the action in
/// `steps`.  No-ops if the path does not exist or is already empty.
fn clear_object_at(value: &mut Value, path: &[&str], steps: &mut Vec<String>) {
    if let Some(v) = get_nested_mut(value, path) {
        if let Some(obj) = v.as_object() {
            let n = obj.len();
            if n > 0 {
                *v = Value::Object(serde_json::Map::new());
                steps.push(format!("dropped {} ({n} entries)", path.join(".")));
            }
        }
    }
}

/// Inject the `_omitted` metadata marker into the top-level JSON object.
/// If `value` is not an Object, the marker is silently skipped.
fn inject_omitted_marker(
    value: &mut Value,
    original_tokens: usize,
    applied_budget: usize,
    steps: Vec<String>,
) {
    if let Some(obj) = value.as_object_mut() {
        let note = format!(
            "Response trimmed from ~{original_tokens} to fit within the \
             {applied_budget}-token budget. Pass a larger `tokens` value to \
             retrieve more detail."
        );
        obj.insert(
            "_omitted".to_string(),
            serde_json::json!({
                "applied_budget": applied_budget,
                "original_tokens": original_tokens,
                "steps_applied": steps,
                "note": note,
            }),
        );
    }
}

/// Render a budget-aware conventions response.
///
/// Applies progressive, **deterministic** degradation to `value` (already
/// category / strength / focus filtered) until the serialised token count fits
/// within `token_budget`.
///
/// Returns the (possibly pruned) JSON value.  When content was dropped an
/// `"_omitted"` key is injected at the top level describing what was removed.
/// If `value` already fits under the budget it is returned **unchanged and
/// without** an `"_omitted"` key.
///
/// # Guarantee
///
/// For `token_budget >= MIN_BUDGET_FLOOR` (currently 300 tokens) the *returned*
/// output — **including the `_omitted` marker** — is guaranteed to be
/// ≤ `token_budget` tokens.  Below this floor the function still returns a
/// minimal skeleton (see Minimal-skeleton backstop below), but the skeleton
/// itself may marginally exceed the budget because the marker alone costs
/// ≈ 120–250 tokens.
///
/// # Degradation order (most-impactful first, each step checks the budget)
///
/// Every "fits?" check uses `token_budget − MARKER_RESERVE` so that the
/// injected `_omitted` marker does not push the final output over budget.
///
/// ## Main stages (Steps 1–5)
///
/// 1. Drop `git_health.co_changes` (O(N²) file-pairs on active repos)
/// 2. Truncate `git_health.churn_30d` / `churn_180d` to 20 entries
/// 3. Clear `git_health.bugfix_density` and `git_health.churn_trend`
/// 4. Drop `additional` observation arrays from all categories (fixed order)
/// 5. Clear `testing.coverage_by_dir`
///
/// ## Terminal stages (Steps 6–10; reached only on very tight budgets)
///
/// 6. Clear `functions.by_directory` (O(dirs) — second-largest bulk source)
/// 7. Drop `dependencies.strict_layers` (O(layer-pairs))
/// 8. Drop `dependencies.circular_deps` (O(detected cycles))
/// 9. Truncate `git_health.churn_30d` / `churn_180d` further to 5; drop `reverts`
/// 10. Clear all remaining churn arrays
///
/// ## Minimal-skeleton backstop
///
/// If all targeted steps are still insufficient, replace the entire value with
/// `{}` and inject the `_omitted` marker.  Estimated output: ≤ 280 tokens.
/// The guarantee holds for `token_budget >= MIN_BUDGET_FLOOR` (300 tokens).
pub fn render_budgeted_conventions(mut value: Value, token_budget: usize) -> Value {
    let counter = TokenCounter::new();

    // Fast path — already fits, no marker needed.
    if token_count(&counter, &value) <= token_budget {
        return value;
    }

    let original_tokens = token_count(&counter, &value);
    let mut steps: Vec<String> = Vec::new();

    // Every "fits?" check reserves MARKER_RESERVE tokens for the `_omitted`
    // marker that will be injected immediately after, so that the returned
    // output (value + marker) stays ≤ token_budget.
    let budget_with_headroom = token_budget.saturating_sub(MARKER_RESERVE);

    // ── Main stages ──────────────────────────────────────────────────────────

    // Step 1: drop git_health.co_changes (largest contributor for active repos).
    drop_array_at(&mut value, &["git_health", "co_changes"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 2: truncate churn arrays to 20 entries (already ordered by
    // modifications desc in the Vec, so truncation is deterministic).
    truncate_array_at(&mut value, &["git_health", "churn_30d"], 20, &mut steps);
    truncate_array_at(&mut value, &["git_health", "churn_180d"], 20, &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 3: clear bugfix_density and churn_trend (HashMap → non-deterministic
    // iteration order; clearing is cheaper than sorting-then-truncating here).
    clear_object_at(&mut value, &["git_health", "bugfix_density"], &mut steps);
    clear_object_at(&mut value, &["git_health", "churn_trend"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 4: drop `additional` arrays from every convention category.
    // CATEGORIES is a fixed-order static slice — no HashMap iteration.
    for category in CATEGORIES {
        drop_array_at(&mut value, &[category, "additional"], &mut steps);
    }
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 5: clear testing.coverage_by_dir.
    clear_object_at(&mut value, &["testing", "coverage_by_dir"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // ── Terminal stages (very tight budgets) ─────────────────────────────────

    // Step 6: clear functions.by_directory — O(directories); the dominant bulk
    // source on large repos outside of git_health.
    clear_object_at(&mut value, &["functions", "by_directory"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 7: drop dependencies.strict_layers — O(layer-pairs).
    drop_array_at(&mut value, &["dependencies", "strict_layers"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 8: drop dependencies.circular_deps — O(detected cycles).
    drop_array_at(&mut value, &["dependencies", "circular_deps"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 9: further truncate churn to 5 entries and drop reverts.
    truncate_array_at(&mut value, &["git_health", "churn_30d"], 5, &mut steps);
    truncate_array_at(&mut value, &["git_health", "churn_180d"], 5, &mut steps);
    drop_array_at(&mut value, &["git_health", "reverts"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // Step 10: clear all remaining churn arrays entirely.
    drop_array_at(&mut value, &["git_health", "churn_30d"], &mut steps);
    drop_array_at(&mut value, &["git_health", "churn_180d"], &mut steps);
    if token_count(&counter, &value) <= budget_with_headroom {
        inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
        return value;
    }

    // ── Minimal-skeleton backstop ─────────────────────────────────────────────
    // All targeted steps exhausted and value is still over budget.  Replace the
    // entire value with an empty object and inject the marker.  Estimated output
    // ≤ 280 tokens — the ≤-budget guarantee holds for token_budget ≥ MIN_BUDGET_FLOOR.
    steps.push(
        "minimal_skeleton: all category bodies cleared (budget below actionable threshold)"
            .to_string(),
    );
    value = Value::Object(serde_json::Map::new());
    inject_omitted_marker(&mut value, original_tokens, token_budget, steps);
    value
}

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

    // ── render_budgeted_conventions unit tests ────────────────────────────────

    /// Build a serde_json::Value with many co_changes entries so the serialised
    /// size clearly exceeds `MAX_MCP_CONVENTIONS_TOKENS`.
    fn make_large_conventions_value() -> Value {
        use crate::conventions::git_health::{ChurnEntry, GitHealthProfile};
        use crate::core_graph::intel::CoChangeEdge;
        use std::collections::HashMap;

        let profile = crate::conventions::ConventionProfile {
            git_health: GitHealthProfile {
                co_changes: (0..300u32)
                    .map(|i| CoChangeEdge {
                        file_a: format!(
                            "src/subsystem/module_{i}/component_{i}/implementation_{i}.rs"
                        ),
                        file_b: format!("tests/subsystem/module_{}/tests_{i}.rs", (i + 1) % 300),
                        count: (i % 20) + 1,
                        recency_weight: 0.5 + (i % 10) as f64 * 0.05,
                    })
                    .collect(),
                churn_30d: (0..100u32)
                    .map(|i| ChurnEntry {
                        path: format!("src/heavy_module_{i}/file_{i}.rs"),
                        modifications: (100 - i) as usize,
                        last_commit_epoch: None,
                    })
                    .collect(),
                churn_180d: (0..100u32)
                    .map(|i| ChurnEntry {
                        path: format!("src/heavy_module_{i}/file_{i}.rs"),
                        modifications: (200 - i) as usize,
                        last_commit_epoch: None,
                    })
                    .collect(),
                bugfix_density: (0..100u32)
                    .map(|i| {
                        (
                            format!("src/heavy_module_{i}/file_{i}.rs"),
                            0.05 * (i as f64 % 5.0) + 0.01,
                        )
                    })
                    .collect::<HashMap<_, _>>(),
                churn_trend: HashMap::new(),
                reverts: vec![],
                last_computed: None,
            },
            ..Default::default()
        };
        serde_json::to_value(&profile).unwrap()
    }

    #[test]
    fn test_render_budgeted_conventions_small_profile_no_marker() {
        // A default (empty) profile serialises to a handful of tokens — must
        // return unchanged with NO _omitted marker.
        let profile = crate::conventions::ConventionProfile::default();
        let value = serde_json::to_value(&profile).unwrap();
        let result = render_budgeted_conventions(value, MAX_MCP_CONVENTIONS_TOKENS);
        assert!(
            result.get("_omitted").is_none(),
            "empty profile must fit under budget — no _omitted marker expected"
        );
    }

    #[test]
    fn test_render_budgeted_conventions_large_profile_under_cap() {
        let value = make_large_conventions_value();
        let counter = TokenCounter::new();

        // Verify the input actually exceeds the default cap (test has teeth).
        let raw_tokens = counter.count(&serde_json::to_string_pretty(&value).unwrap());
        assert!(
            raw_tokens > MAX_MCP_CONVENTIONS_TOKENS,
            "test fixture must exceed {MAX_MCP_CONVENTIONS_TOKENS} tokens \
             before budgeting, got {raw_tokens}"
        );

        let result = render_budgeted_conventions(value, MAX_MCP_CONVENTIONS_TOKENS);
        let output = serde_json::to_string_pretty(&result).unwrap();
        let output_tokens = counter.count(&output);

        assert!(
            output_tokens <= MAX_MCP_CONVENTIONS_TOKENS,
            "budgeted output must be ≤ {MAX_MCP_CONVENTIONS_TOKENS} tokens, got {output_tokens}"
        );
    }

    #[test]
    fn test_render_budgeted_conventions_omission_marker_present() {
        let value = make_large_conventions_value();
        let result = render_budgeted_conventions(value, MAX_MCP_CONVENTIONS_TOKENS);
        assert!(
            result.get("_omitted").is_some(),
            "_omitted must be present when content was dropped"
        );
        let omitted = &result["_omitted"];
        assert!(omitted["applied_budget"].is_number());
        assert!(omitted["steps_applied"].is_array());
        assert!(
            !omitted["steps_applied"].as_array().unwrap().is_empty(),
            "steps_applied must list at least one degradation step"
        );
    }

    #[test]
    fn test_render_budgeted_conventions_larger_budget_more_content() {
        let value = make_large_conventions_value();
        let small = serde_json::to_string_pretty(&render_budgeted_conventions(
            value.clone(),
            MAX_MCP_CONVENTIONS_TOKENS,
        ))
        .unwrap();
        let large =
            serde_json::to_string_pretty(&render_budgeted_conventions(value, 200_000)).unwrap();
        assert!(
            large.len() > small.len(),
            "larger budget must yield more content \
             (small={} chars, large={} chars)",
            small.len(),
            large.len()
        );
    }

    #[test]
    fn test_render_budgeted_conventions_deterministic() {
        let value = make_large_conventions_value();
        let a = serde_json::to_string_pretty(&render_budgeted_conventions(
            value.clone(),
            MAX_MCP_CONVENTIONS_TOKENS,
        ))
        .unwrap();
        let b = serde_json::to_string_pretty(&render_budgeted_conventions(
            value,
            MAX_MCP_CONVENTIONS_TOKENS,
        ))
        .unwrap();
        assert_eq!(
            a, b,
            "render_budgeted_conventions must be byte-identical for the same input"
        );
    }

    #[test]
    fn test_render_budgeted_conventions_narrow_result_no_marker() {
        // A narrow category=git_health with only a few entries fits under budget.
        // Assert no spurious _omitted marker appears.
        let small_profile = crate::conventions::ConventionProfile::default();
        let value = serde_json::to_value(&small_profile.git_health).unwrap();
        let result = render_budgeted_conventions(value, MAX_MCP_CONVENTIONS_TOKENS);
        assert!(
            result.get("_omitted").is_none(),
            "narrow result under budget must not carry an _omitted marker"
        );
    }

    // ── New ≤-budget guarantee tests (Steps 6-10 + marker headroom) ──────────

    /// Build a serde_json::Value whose bulk is in `functions.by_directory` and
    /// `dependencies.strict_layers` — NOT in `git_health`.  The original 5
    /// degradation steps do not touch these fields, so pre-fix output exceeded
    /// `MAX_MCP_CONVENTIONS_TOKENS` unconditionally.
    fn make_large_non_git_health_value() -> Value {
        use crate::conventions::deps::{DependencyConventions, DirectionPair};
        use crate::conventions::functions::{DirectoryFunctionStats, FunctionConventions};
        use std::collections::HashMap;

        let profile = crate::conventions::ConventionProfile {
            functions: FunctionConventions {
                avg_length: Some(15.0),
                median_length: Some(12.0),
                by_directory: (0..500u32)
                    .map(|i| {
                        (
                            format!("src/module_{i}/submodule_{i}/component_{i}"),
                            DirectoryFunctionStats {
                                avg_length: 15.0,
                                median_length: 12.0,
                                count: (i % 20 + 1) as usize,
                            },
                        )
                    })
                    .collect(),
                additional: vec![],
                file_contributions: HashMap::new(),
            },
            dependencies: DependencyConventions {
                strict_layers: (0..200u32)
                    .map(|i| DirectionPair {
                        from: format!("src/domain_{i}/module_{i}"),
                        to: format!("src/core/subsystem_{i}"),
                        edge_count: (i % 10 + 1) as usize,
                        reverse_count: (i % 3) as usize,
                    })
                    .collect(),
                circular_deps: (0..50u32)
                    .map(|i| {
                        format!(
                            "src/module_{i} → src/module_{} → src/module_{i}",
                            (i + 1) % 50
                        )
                    })
                    .collect(),
                db_isolation: None,
                additional: vec![],
            },
            ..Default::default()
        };
        serde_json::to_value(&profile).unwrap()
    }

    #[test]
    fn test_render_budgeted_bulk_not_in_git_health_under_default_cap() {
        // Regression: profile bulk is in functions.by_directory and
        // dependencies.strict_layers (not git_health).  Steps 1–5 do not touch
        // these fields; Steps 6–8 (terminal stage) must handle them.
        // Pre-fix: output far exceeded MAX_MCP_CONVENTIONS_TOKENS.
        let value = make_large_non_git_health_value();
        let counter = TokenCounter::new();

        let raw_tokens = counter.count(&serde_json::to_string_pretty(&value).unwrap());
        assert!(
            raw_tokens > MAX_MCP_CONVENTIONS_TOKENS,
            "fixture must exceed {MAX_MCP_CONVENTIONS_TOKENS} tokens before budgeting, \
             got {raw_tokens}"
        );

        let result = render_budgeted_conventions(value, MAX_MCP_CONVENTIONS_TOKENS);
        let output_tokens = counter.count(&serde_json::to_string_pretty(&result).unwrap());
        assert!(
            output_tokens <= MAX_MCP_CONVENTIONS_TOKENS,
            "output (marker included) must be ≤ {MAX_MCP_CONVENTIONS_TOKENS}, \
             got {output_tokens}"
        );
        assert!(
            result.get("_omitted").is_some(),
            "_omitted marker must be present when content was dropped"
        );
    }

    #[test]
    fn test_render_budgeted_small_budget_override_honored() {
        // Regression: a small token_budget override must be strictly honored.
        // 500 tokens is above MIN_BUDGET_FLOOR (300) so the ≤-budget guarantee
        // must hold end-to-end (marker included).
        // Pre-fix: no terminal cap — output far exceeded 500 tokens.
        let value = make_large_non_git_health_value();
        let counter = TokenCounter::new();
        let budget: usize = 500;

        let raw_tokens = counter.count(&serde_json::to_string_pretty(&value).unwrap());
        assert!(
            raw_tokens > budget,
            "fixture must exceed {budget} tokens before budgeting, got {raw_tokens}"
        );

        let result = render_budgeted_conventions(value, budget);
        let output_tokens = counter.count(&serde_json::to_string_pretty(&result).unwrap());
        assert!(
            output_tokens <= budget,
            "output (marker included) must be ≤ {budget}, got {output_tokens}"
        );
        assert!(
            result.get("_omitted").is_some(),
            "_omitted marker must be present when trimming occurred"
        );
    }

    #[test]
    fn test_render_budgeted_marker_headroom_included_in_budget() {
        // The _omitted marker's own tokens must be counted against the budget.
        //
        // Construction: we compute what the value looks like after step 1
        // (drop co_changes) using the same parameters as make_large_conventions_value(),
        // then pick budget = after_step1_tokens + 50.  This places us squarely
        // in the "value fits the old budget check but value+marker doesn't" zone.
        //
        // Pre-fix (old code): step 1 check passes (value ≤ budget), marker
        //   is injected → output = value + marker_tokens > budget.  FAILS.
        // Post-fix: headroom check (value ≤ budget − 200) fails, degradation
        //   continues to step 2+ which brings value well below budget_with_headroom,
        //   then marker is injected → output ≤ budget.  PASSES.
        use crate::conventions::git_health::{ChurnEntry, GitHealthProfile};
        use std::collections::HashMap;

        // Compute the approximate size of the value-after-step-1 (co_changes dropped).
        // We serialise the same profile without co_changes to get the exact count.
        let profile_no_co_changes = crate::conventions::ConventionProfile {
            git_health: GitHealthProfile {
                co_changes: vec![],
                churn_30d: (0..100u32)
                    .map(|i| ChurnEntry {
                        path: format!("src/heavy_module_{i}/file_{i}.rs"),
                        modifications: (100 - i) as usize,
                        last_commit_epoch: None,
                    })
                    .collect(),
                churn_180d: (0..100u32)
                    .map(|i| ChurnEntry {
                        path: format!("src/heavy_module_{i}/file_{i}.rs"),
                        modifications: (200 - i) as usize,
                        last_commit_epoch: None,
                    })
                    .collect(),
                bugfix_density: (0..100u32)
                    .map(|i| {
                        (
                            format!("src/heavy_module_{i}/file_{i}.rs"),
                            0.05 * (i as f64 % 5.0) + 0.01,
                        )
                    })
                    .collect::<HashMap<_, _>>(),
                churn_trend: HashMap::new(),
                reverts: vec![],
                last_computed: None,
            },
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let tokens_after_step1 = counter.count(
            &serde_json::to_string_pretty(&serde_json::to_value(&profile_no_co_changes).unwrap())
                .unwrap(),
        );

        // Pick a budget just above after_step1 — in the headroom danger zone.
        // Pre-fix: old code would do step1 check (tokens ≤ budget → true),
        //   inject marker → tokens + marker_size > budget. BUG.
        // Post-fix: headroom check (tokens ≤ budget − 200 → false since
        //   budget = tokens + 50 means budget − 200 = tokens − 150 < tokens),
        //   continues; step 2 truncates churn → much smaller → fits. ✓
        let budget = tokens_after_step1 + 50;

        let large_value = make_large_conventions_value();
        let raw_tokens = counter.count(&serde_json::to_string_pretty(&large_value).unwrap());
        assert!(
            raw_tokens > budget,
            "fixture must exceed budget={budget}, got {raw_tokens}"
        );
        assert!(
            tokens_after_step1 < budget,
            "after-step-1 ({tokens_after_step1}) must be < budget ({budget}) \
             for headroom to matter"
        );

        let result = render_budgeted_conventions(large_value, budget);
        let output = serde_json::to_string_pretty(&result).unwrap();
        let output_tokens = counter.count(&output);

        assert!(
            result.get("_omitted").is_some(),
            "_omitted marker must be present (trimming occurred)"
        );
        assert!(
            output_tokens <= budget,
            "_omitted marker must be counted against budget: \
             output={output_tokens} tokens, budget={budget}"
        );
    }

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
                    last_commit_epoch: None,
                }],
                churn_180d: vec![ChurnEntry {
                    path: "src/hot_file.rs".to_string(),
                    modifications: 120,
                    last_commit_epoch: None,
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
                    last_commit_epoch: None,
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
                last_commit_epoch: None,
            }],
            churn_180d: vec![ChurnEntry {
                path: "src/main.rs".to_string(),
                modifications: 15,
                last_commit_epoch: None,
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
