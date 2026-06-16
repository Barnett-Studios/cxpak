//! Token-efficiency reporting for auto_context (ADR-0168).
//!
//! Decision-support, not a dashboard: the report answers "is this context good
//! enough, and what should I change?" via relevant-set coverage (of what cxpak
//! deemed relevant, what fraction fit the budget), the budget-margin scores
//! (lowest included vs. highest excluded), and a silent-unless-actionable advisory.
use serde::Serialize;

/// Built-in, dated input-token rates (USD per 1M tokens). Updated 2026-06-14.
/// Opt-in only; never shown unless a model is explicitly requested.
const RATES_USD_PER_MTOK: &[(&str, f64)] = &[
    ("claude-opus-4-8", 15.0),
    ("claude-sonnet-4-6", 3.0),
    ("claude-haiku-4-5", 0.80),
];

const RATES_DATED: &str = "2026-06-14";

pub struct EfficiencyInputs {
    pub repo_tokens: usize,
    pub selected_tokens: usize,
    pub budget_total: usize,
    pub budget_used: usize,
    pub filtered_tokens: usize,
    /// Count of files cxpak deemed relevant (post-noise `kept` set).
    pub relevant_total: usize,
    /// Of those, how many were packed into the final context.
    pub relevant_covered: usize,
    /// Lowest relevance score among INCLUDED files (the budget margin from above).
    pub marginal_included_score: Option<f64>,
    /// Highest relevance score among EXCLUDED files (the budget margin from below).
    pub marginal_excluded_score: Option<f64>,
    pub cost_model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CostEstimate {
    pub model: String,
    pub input_usd: f64,
    pub rates_dated: &'static str,
}

#[derive(Debug, Serialize)]
pub struct EfficiencyReport {
    pub repo_tokens: usize,
    pub selected_tokens: usize,
    /// Headline: of the relevant set, what fraction was packed. Actionable.
    pub relevant_coverage: f64,
    pub relevant_total: usize,
    pub relevant_covered: usize,
    /// Demoted sanity field: selected/repo. Always ~tiny on large repos; not actionable.
    pub absolute_coverage: f64,
    pub budget_total: usize,
    pub budget_used: usize,
    pub budget_utilization: f64,
    pub marginal_included_score: Option<f64>,
    pub marginal_excluded_score: Option<f64>,
    pub tokens_saved_filtering: usize,
    pub cost_estimate: Option<CostEstimate>,
    /// Human-readable, derived guidance (empty when the selection is healthy).
    pub advisory: Vec<String>,
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

pub fn compute_efficiency(i: EfficiencyInputs) -> EfficiencyReport {
    let cost_estimate = i.cost_model.as_ref().and_then(|m| {
        RATES_USD_PER_MTOK
            .iter()
            .find(|(name, _)| *name == m)
            .map(|(name, rate)| CostEstimate {
                model: (*name).to_string(),
                input_usd: i.selected_tokens as f64 / 1_000_000.0 * rate,
                rates_dated: RATES_DATED,
            })
    });
    let relevant_coverage = ratio(i.relevant_covered, i.relevant_total);
    let budget_utilization = ratio(i.budget_used, i.budget_total);

    // Derived advisory: only speak when there is something to act on.
    let mut advisory = Vec::new();
    // Starving: budget is the binding constraint AND the cut is well above the noise floor.
    if relevant_coverage < 1.0 {
        if let (Some(inc), Some(exc)) = (i.marginal_included_score, i.marginal_excluded_score) {
            // A near-tie at the boundary is healthy (cut at the natural margin);
            // a large gap excluded means we're dropping clearly-relevant files.
            if exc > 0.5 && (inc - exc) < 0.15 {
                advisory.push(format!(
                    "Budget is the binding constraint: {} of {} relevant files packed; \
                     the highest-scoring excluded file ({exc:.2}) is close to the lowest included ({inc:.2}). \
                     Raise --tokens to include more.",
                    i.relevant_covered, i.relevant_total
                ));
            }
        }
    }
    // Headroom: budget barely used AND files were filtered — threshold may be too strict.
    if budget_utilization < 0.5 && i.filtered_tokens > 0 {
        advisory.push(format!(
            "{:.0}% budget headroom with {} tokens filtered out — lower the relevance \
             threshold or raise --tokens to use the spare budget.",
            (1.0 - budget_utilization) * 100.0,
            i.filtered_tokens
        ));
    }

    EfficiencyReport {
        repo_tokens: i.repo_tokens,
        selected_tokens: i.selected_tokens,
        relevant_coverage,
        relevant_total: i.relevant_total,
        relevant_covered: i.relevant_covered,
        absolute_coverage: ratio(i.selected_tokens, i.repo_tokens),
        budget_total: i.budget_total,
        budget_used: i.budget_used,
        budget_utilization,
        marginal_included_score: i.marginal_included_score,
        marginal_excluded_score: i.marginal_excluded_score,
        tokens_saved_filtering: i.filtered_tokens,
        cost_estimate,
        advisory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs() -> EfficiencyInputs {
        EfficiencyInputs {
            repo_tokens: 100_000,
            selected_tokens: 9_700,
            budget_total: 50_000,
            budget_used: 9_700,
            filtered_tokens: 12_000,
            relevant_total: 10,
            relevant_covered: 10,
            marginal_included_score: Some(0.42),
            marginal_excluded_score: None,
            cost_model: None,
        }
    }

    #[test]
    fn efficiency_reconciles_coverage_and_savings() {
        let r = compute_efficiency(base_inputs());
        assert_eq!(r.repo_tokens, 100_000);
        assert!((r.relevant_coverage - 1.0).abs() < 1e-9); // 10/10 relevant packed
        assert!((r.absolute_coverage - 0.097).abs() < 1e-9); // demoted sanity field
        assert!((r.budget_utilization - 0.194).abs() < 1e-9);
        assert_eq!(r.tokens_saved_filtering, 12_000);
        assert!(r.cost_estimate.is_none());
    }

    #[test]
    fn advisory_warns_when_starving_at_a_tight_margin() {
        // 7/10 relevant packed, and the best excluded file (0.80) is right next to
        // the worst included (0.83) → budget is the binding constraint.
        let r = compute_efficiency(EfficiencyInputs {
            relevant_total: 10,
            relevant_covered: 7,
            marginal_included_score: Some(0.83),
            marginal_excluded_score: Some(0.80),
            budget_used: 50_000, // fully used
            ..base_inputs()
        });
        assert!(r.advisory.iter().any(|a| a.contains("binding constraint")));
    }

    #[test]
    fn advisory_warns_on_headroom_with_filtering() {
        let r = compute_efficiency(EfficiencyInputs {
            budget_total: 50_000,
            budget_used: 5_000,
            filtered_tokens: 8_000,
            ..base_inputs()
        });
        assert!(r.advisory.iter().any(|a| a.contains("budget headroom")));
    }

    #[test]
    fn healthy_selection_is_silent() {
        // full coverage, budget well-used → no nagging.
        let r = compute_efficiency(EfficiencyInputs {
            relevant_total: 10,
            relevant_covered: 10,
            budget_total: 50_000,
            budget_used: 40_000,
            filtered_tokens: 0,
            ..base_inputs()
        });
        assert!(r.advisory.is_empty());
    }

    #[test]
    fn cost_estimate_only_when_model_requested() {
        let r = compute_efficiency(EfficiencyInputs {
            cost_model: Some("claude-opus-4-8".to_string()),
            ..base_inputs()
        });
        let est = r
            .cost_estimate
            .expect("cost estimate present when model given");
        assert_eq!(est.model, "claude-opus-4-8");
        assert!(est.input_usd > 0.0);
    }

    #[test]
    fn ratios_zero_denominator_are_zero_not_nan() {
        let r = compute_efficiency(EfficiencyInputs {
            repo_tokens: 0,
            selected_tokens: 0,
            budget_total: 0,
            budget_used: 0,
            filtered_tokens: 0,
            relevant_total: 0,
            relevant_covered: 0,
            marginal_included_score: None,
            marginal_excluded_score: None,
            cost_model: None,
        });
        assert_eq!(r.relevant_coverage, 0.0);
        assert_eq!(r.absolute_coverage, 0.0);
        assert_eq!(r.budget_utilization, 0.0);
        assert!(r.advisory.is_empty());
    }
}
