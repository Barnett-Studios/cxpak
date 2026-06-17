---
id: '0116'
title: Recency becomes the 5th relevance signal at weight 0.05 with linear 90-day decay, rebalancing all weights
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.2.0 wants recently-changed files to rank slightly higher in relevance scoring
loop: planning
---

# ADR-0116: Recency becomes the 5th relevance signal at weight 0.05 with linear 90-day decay, rebalancing all weights

## Context

Released in v1.2.0. The relevance scorer carried a `recency_boost` signal at weight 0.0 — defined but effectively disabled. Recent activity is a strong relevance cue (files touched recently are more likely relevant to the current task), so the signal should be activated. Because the scorer maintains two weight configurations — 6 signals without embeddings and 7 signals with embeddings — and each must sum to 1.0, activating recency requires rebalancing both vectors rather than just flipping one constant.

## Options considered

- **Option A — activate recency at 0.05 with linear 90-day decay, rebalance both configs (chosen):** Change `recency_boost` from 0.0 to 0.05; score files 1.0 on the day they change, decaying linearly to 0.0 at 90 days, sourced from git log (most recent commit per file); reduce `term_frequency` to absorb the 0.05 delta in both configs so each stays normalized. Pros: recently-touched files get a modest nudge without overwhelming structural signals; both vectors remain summed to 1.0. Cons: weight values are hand-tuned; `term_frequency` influence is reduced to fund recency.
- **Option B — leave recency disabled at 0.0:** A reasonable alternative would have been to keep the signal inert. Pros: no weight rebalancing needed. Cons: a strong relevance cue (recent activity) is ignored entirely. Rejected: the value of the cue outweighs the rebalancing cost.

## Decision

Activate the recency signal (#5) by changing its weight from 0.0 to 0.05. The signal is sourced from git log (most recent commit per file) and scores 1.0 for files changed today, decaying linearly to 0.0 at 90 days. Rebalance both configurations so each sums to 1.0:

- without embeddings (6 signals): `term_frequency` drops 0.19 → 0.14, the freed 0.05 goes to `recency_boost`
- with embeddings (7 signals): `term_frequency` drops 0.16 → 0.11, the freed 0.05 goes to `recency_boost`

Implemented across `src/relevance/mod.rs` (weight vectors), `src/relevance/signals.rs` (`recency_boost_signal`), and `src/intelligence/recent_changes.rs` (`recency_score_for_file`, computing `(1.0 - days/90.0).clamp(0.0, 1.0)`).

## Consequences

### Positive
- Recently-changed files receive a modest relevance boost.
- Both the 6-signal and 7-signal weight vectors remain normalized to 1.0.

### Negative
- Weight values remain hand-tuned, not learned.
- `term_frequency` influence is reduced to fund recency.

### Neutral
- Files with no recoverable commit date contribute 0.0 from this signal (graceful fallback).

## Revisit if
- The 0.05 recency weight or the 90-day decay window needs recalibration.
- Signal weights are ever empirically learned rather than hand-set.
