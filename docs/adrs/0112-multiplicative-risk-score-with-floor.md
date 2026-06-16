---
id: '0112'
title: 'Standing risk score is multiplicative with a 0.01 floor and percentile-rank churn normalization'
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 per-file risk ranking
loop: planning
---

# ADR-0112: Standing risk score is multiplicative with a 0.01 floor and percentile-rank churn normalization

## Context

Introduced in v1.2.0 for per-file risk ranking. Risk combines churn, blast radius, and test coverage. A purely additive score lets a high value in one factor mask zeros in the others — a file can look fine while being untested. A purely multiplicative score lets any single zero factor zero out the score; for the blast dimension this is intentional (an architecturally isolated file with no dependents should drop out of the top risks), but for churn and test coverage a hard zero would discard otherwise-meaningful signal, so those factors get a small floor. Raw churn counts are also outlier-sensitive.

## Options considered

- **Option A — Multiplicative with a 0.01 floor on churn and test-coverage, percentile-rank churn:** `risk = max(norm_churn, 0.01) × norm_blast × max(1 - test_coverage, 0.01)`. `norm_churn` is percentile rank across files; `norm_blast = blast_radius_count / total_files` (no floor); `test_coverage` is binary 0/1. Pros: no single churn or coverage zero hides a risky file, but the small floor avoids inflating zero-input files; an isolated file (zero blast) intentionally zeroes out, keeping README/Cargo.toml/CSS out of top risks; percentile rank is robust against churn outliers; blast normalization is semantically meaningful. Cons: multiplicative scores compress into a small range [0.0, 1.0]; binary test coverage is coarse. Someone could prefer it for its resistance to single-factor masking.

- **Option B — Weighted additive sum:** A reasonable alternative would have been `risk = w1·churn + w2·blast + w3·(1 - coverage)`. Pros: wider dynamic range, intuitive weighting. Cons: a high single factor masks zeros in the others — a file can look fine while being untested. Someone could prefer it for the more legible numeric spread.

## Decision

Compute standing risk multiplicatively with a 0.01 floor on the churn and test-coverage factors only: `risk = max(norm_churn, 0.01) × norm_blast × max(1 - test_coverage, 0.01)`, effective range [0.0, 1.0]. `norm_blast` is NOT floored — a file with zero dependents is architecturally isolated and zeroes out by design, which keeps low-coupling files like README/Cargo.toml/CSS out of the top risks. `norm_churn` uses percentile rank across all files (outlier-robust); `norm_blast = blast_radius_count / total_files`; `test_coverage` is binary (0.0 = no mapped tests, 1.0 = has tests). Named "standing risk", explicitly distinct from `compute_blast_impact()`'s change/blast-impact risk in `blast_radius.rs`. Top 10 shown in auto_context, full list via `cxpak_risks`.

## Consequences

### Positive
- No single churn or test-coverage zero hides a risky file (those factors are floored); the blast factor is intentionally allowed to zero out for isolated files.
- Percentile-rank churn resists a single high-churn file compressing all others.
- Clear naming separates standing risk from change/blast-impact risk.

### Negative
- The multiplicative product compresses into a small range and collapses to 0.0 for isolated files; range is [0.0, 1.0].
- Binary test coverage is coarse until line coverage is available.

### Neutral
- "Standing risk" and `compute_blast_impact()`'s change risk are two distinct measures kept deliberately separate.

## Revisit if
- Line-level coverage data becomes available to replace the binary signal.
- The compressed numeric range [0.0, 1.0] hurts legibility.
