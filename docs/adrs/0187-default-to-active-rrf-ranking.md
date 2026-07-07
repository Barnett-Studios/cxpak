---
id: '0187'
title: Default to Active (RRF fusion) ranking after full-corpus recall A/B
status: ACCEPTED
date: 2026-07-07
triggered_by: Phase R task R-D1 (full-corpus recall A/B; controller + user gate)
loop: implementation
---

# ADR-0187: Default to Active (RRF fusion) ranking after full-corpus recall A/B

## Context

ADR-0184 shipped the D1 semantic upgrade — weighted, scale-normalized Reciprocal
Rank Fusion (`rrfᵢ = Σⱼ wⱼ·(K+1)/(K+rankᵢⱼ)`, `K=60`) replacing the linear
weighted-sum combine — behind a runtime `RelevanceMode { Inert, Active }` control,
with `DEFAULT_RELEVANCE_MODE = Inert`. The machinery was built, unit-tested, and
measurable index-once, but flipping the default to `Active` was explicitly gated
on a full-corpus recall A/B validating `recall@budget(active) ≥ recall@budget(inert)`
— a controller + user decision (the C2 precedent: ship the machinery inert, flip
the default once the gate clears), not something the code could resolve on its own.

The controller ran the full runnable-corpus recall A/B: 31 real PRs (ripgrep 11,
flask 10, express 10; spring-boot / TypeScript / cli repos OOM-excluded),
index-once, 6-signal RRF default path, `auto_context_with_mode` Inert vs Active at
the 8k and 32k budgets.

| repo    | n  | inert@8k | active@8k | inert@32k | active@32k |
|---------|----|----------|-----------|-----------|------------|
| ripgrep | 11 | 0.227    | 0.636     | 0.227     | 0.636      |
| flask   | 10 | 0.320    | 0.655     | 0.320     | 0.665      |
| express | 10 | 0.000    | 0.150     | 0.000     | 0.150      |
| **ALL** | **31** | **0.184** | **0.485** | **0.184** | **0.489** |

Active measured **+0.30 recall at both budgets (+164%)**, with a per-PR record of
**17 wins / 12 ties / 2 losses**. Both losses are flask (#5903 0.50→0.00, #5928
0.60→0.50 @8k) — the known `SEED_THRESHOLD` edge (ADR-0184 D1-R Imp-1). The USER
approved the flip.

## Options considered

- **Option A — flip the default to `Active`:** make RRF fusion the shipped ranking.
  Pro: +164% corpus recall, the gate's stated success condition, cleanly met at
  both budgets. Con: two isolated flask regressions, and on large repos (`n ≳ 200`)
  the RRF worst-file floor `(K+1)/(K+n) ≈ 0.17–0.24` exceeds `SEED_THRESHOLD = 0.10`,
  so threshold filtering degenerates to the top-50 seed cap rather than the 0.10
  cutoff.
- **Option B — keep `Inert`, ship `Active` opt-in only:** conservative; preserves
  the byte-stable pre-D1 ranking as the default surface. Pro: zero regression risk
  for existing callers; a cautious stakeholder could prefer never changing a
  shipped default without a longer bake. Con: leaves a measured +164% recall win
  on the table for every default caller; the gate was defined precisely to license
  this flip once it cleared, and it cleared decisively.
- **Option C — flip only above a repo-size threshold:** switch to `Active` for
  small repos, keep `Inert` for large ones where the `SEED_THRESHOLD` degeneration
  bites. Pro: sidesteps the large-repo floor issue. Con: the corpus A/B shows
  Active wins net across the whole runnable corpus including the larger repos; a
  size-gated default adds a mode-selection branch and a discontinuity in behavior
  with no measured recall justification — complexity the data does not support.

## Decision

Set `DEFAULT_RELEVANCE_MODE = RelevanceMode::Active` (Option A). The full-corpus
recall A/B cleared the ADR-0184 gate (+164% at both budgets) and the controller
and USER approved the flip. `Inert` remains a first-class, byte-stable control
reachable via `MultiSignalScorer::with_mode(Inert)` and
`auto_context::auto_context_with_mode(.., Inert)` — the A/B harness continues to
score both modes from a single index build.

## Consequences

### Positive
- Default `auto_context` / `overview` / MCP / HTTP / LSP retrieval now ranks with
  RRF fusion — +0.30 recall (+164%) at both the 8k and 32k budgets over the
  corpus.
- The Inert weighted-sum path stays available and byte-stable as an A/B control;
  the D1 A/B guarantee (both modes reproducible index-once) is preserved.
- The golden determinism fixture (`spa_output_matches_golden_fixture`) is
  **unaffected** — it renders the VISUAL / PageRank path (`visual::spa::render_spa`),
  independent of the relevance scorer — so it stays byte-identical across the flip.

### Negative
- Two isolated flask regressions (#5903 0.50→0.00, #5928 0.60→0.50 @8k), the known
  `SEED_THRESHOLD` edge.
- On large repos (`n ≳ 200`) the `(K+1)`-normalization prevents seed COLLAPSE but
  does NOT preserve threshold DISCRIMINATION: the RRF worst-file floor
  `(K+1)/(K+n) ≈ 0.17–0.24` exceeds `SEED_THRESHOLD = 0.10`, so Active-mode
  threshold filtering degenerates to the top-50 seed cap. This degeneration is
  MEASURED and net-positive across the corpus; ADR-0184's D1-R Imp-1 wording is
  softened accordingly (RRF keeps scores in-range so seed selection cannot
  collapse — NOT that it keeps `SEED_THRESHOLD` calibrated).

### Neutral
- Contextual retrieval (ADR-0184 piece 2/3) is now reachable in production when
  embeddings are configured (`.cxpak.json`) AND mode is `Active` — R-E1 (ADR-0186)
  wired `build_embedding_index` as an opt-in background build. Its recall impact is
  not re-measured here (opt-in, correctness-gated per R-E1).
- Two 2-file synthetic expansion unit tests (`..._uses_expansion_for_auth_terms`,
  `..._expansion_synonym_boosts_score`) were re-expressed: under RRF the fused
  top-1 slot on a degenerate 2-file index is decided by the alphabetical
  rank-tiebreak across the ~5 dead-zero signals, so the target file loses the slot
  by ~0.006 despite winning every signal that actually fired. The assertions now
  test the invariant expansion genuinely provides — the synonym-fed discriminating
  signals (path_similarity + term_frequency) give the target strictly more evidence
  than the unrelated file — not an exact fused ordering RRF no longer guarantees on
  a toy index.

## Revisit if

- The `SEED_THRESHOLD` degeneration on large repos (`n ≳ 200`) is shown to cost
  recall on a bigger corpus than the 31-PR runnable set measured here — a rank- or
  percentile-based seed cutoff would then be worth designing.
- A future corpus A/B measures `Active` regressing recall net (not just the 2
  isolated flask PRs) at any budget — the flip would be reconsidered or gated.
- Contextual retrieval is measured (embeddings-on corpus A/B) and materially
  changes the Active-vs-Inert delta.
