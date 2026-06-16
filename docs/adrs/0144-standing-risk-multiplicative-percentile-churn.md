---
id: '0144'
title: 'Standing per-file risk = floored-percentile-churn x normalized-blast (no floor) x floored-lack-of-tests'
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.2.0 need for a risk-ranked file list (cxpak_risks tool) to flag which files need care before a refactor
loop: implementation
---

# ADR-0144: Standing per-file risk = floored-percentile-churn x normalized-blast (no floor) x floored-lack-of-tests

## Context

cxpak v1.2.0 needed a "standing risk" ranking independent of any current change, to flag which files warrant care before a refactor (surfaced via the `cxpak_risks` tool). Raw churn counts are outlier-prone. A multiplicative formula can collapse to zero if any factor is zero — and the shipped design deliberately accepts that collapse for the blast dimension rather than flooring it.

## Options considered

- **Option A — Multiplicative risk with percentile-rank churn (chosen):** `risk = max(norm_churn, 0.01) * norm_blast * max(1 - test_coverage, 0.01)`, with churn normalized by percentile rank across all files. Pros: percentile rank is robust to churn outliers; the multiplicative form means a file scores high only when churn, blast, and lack-of-tests are all present. Cons: binary `test_coverage` (0/1) in v1.2.0 is coarse. Preferred for outlier robustness and the all-factors-must-be-present interaction.
- **Option B — Additive weighted sum of the three signals:** A reasonable alternative would have been `risk = w1*churn + w2*blast + w3*untested`. Pros: no collapse-to-zero behavior; no floor needed. Cons: loses the "all three must be present for high risk" multiplicative interaction the team wanted. Someone could prefer it where partial signals should still accumulate visible risk.
- **Option C — Raw churn count without normalization:** A reasonable alternative would have been to use absolute 30-day modification counts directly. Pros: trivial to compute. Cons: dominated by a few hot files; percentile rank was chosen specifically for robustness against outliers. Someone could prefer raw counts when absolute change volume is the quantity of interest.

## Decision

Compute standing risk per file as a product of three factors:

- **Percentile-rank-normalized 30-day churn**, floored at 0.01.
- **Blast radius normalized by total file count**, with NO floor.
- **(1 - binary `test_coverage`)**, floored at 0.01.

Sort descending; cap at top 10 in `AutoContextResult.risks`. The shipped minimum is 0.0, not a positive floor: files with zero dependents (blast = 0) collapse to exactly 0 risk by design (`risk.rs`: "NO floor — files with no dependents are architecturally isolated and contribute 0 risk from the blast dimension"). The unit test `test_files_with_zero_blast_have_zero_risk` asserts this. This deliberately keeps no-reverse-edge files (README, Cargo.toml, CSS) out of `top_risks`.

## Consequences

### Positive
- Robust to churn outliers via percentile rank.
- The multiplicative form ensures untested, high-churn, high-blast files rank highest; the zero-blast collapse excludes architecturally isolated files.
- Deterministic ordering (paths sorted before ranking).

### Negative
- Binary test coverage is coarse in v1.2.0.
- Zero-blast files collapse to exactly 0 risk, so they are not ranked at all (intentional).

### Neutral
- Implemented in `src/intelligence/risk.rs`; surfaced via the `cxpak_risks` MCP tool with default limit 20.

## Revisit if
- Test coverage becomes a continuous ratio rather than binary.
- Users report the zero-blast collapse hides files that should still rank.

## Sources

- `2026-04-01-v120-implementation-plan.md`: "/// Formula: risk = max(norm_churn, 0.01) * max(norm_blast, 0.01) * max(1.0 - test_coverage, 0.01)" (the plan formula; the shipped code removed the blast floor — see `src/intelligence/risk.rs`).
- `2026-04-01-v120-implementation-plan.md`: "/// norm_churn: percentile rank across all files (robust against outliers)"
- `src/intelligence/risk.rs`: "norm_blast: ... NO floor — files with no dependents are architecturally isolated and contribute 0 risk from the blast dimension; this keeps README / Cargo.toml / CSS / etc. out of top_risks"
