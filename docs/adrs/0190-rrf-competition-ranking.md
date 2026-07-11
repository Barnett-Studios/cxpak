---
id: '0190'
title: RRF fuses competition ranks, not ordinal ranks, so uniform signals are inert
status: ACCEPTED
date: 2026-07-09
triggered_by: Adversarial correctness review of the 3.0.0 branch (C1) + human decision to fix
loop: implementation
---

# ADR-0190: RRF fuses competition ranks, not ordinal ranks

## Context

RRF (`src/relevance/rrf.rs`, ADR-0184) assigned each file a distinct **ordinal**
rank per signal — its 1-based position after sorting by `(score desc, path asc)`
— and the module doc noted the path tiebreak was "as the brief prescribes."

The adversarial correctness review (C1) showed that ordinal ranking makes a
*uniform* weighted signal actively harmful. When a signal scores every file
identically (a common case: `recency_score_for_file` returns a constant for
files not in the churn list, and on a repo with no git history for *all* files;
`symbol_match` / `term_frequency` are uniformly 0 for a query whose tokens match
nothing), ordinal ranking still spreads those tied files across ranks `1..n`
purely by path. That injects a path-descending term `w·(K+1)/(K+rank)` that
leaks the signal's **full weight** as an alphabetical gradient. With enough
uniform weight (the 6-signal vector can reach `0.32 + 0.14 + 0.05 = 0.51`
uniform), the worse file can win by filename: e.g. `aaa_loser` outscoring
`zzz_winner` despite `zzz_winner` being strictly better on the only informative
signal.

Because the path-tiebreak-in-fusion was plan-mandated, whether to change it was
a human decision (per the subagent-driven-development protocol). The maintainer
chose to fix it, accepting that it changes the flagship Active-mode default
output and requires regenerating the bench baseline.

## Options considered

- **Competition ("1224") ranking (chosen):** every file tied on a signal shares
  the rank `1 + (files strictly greater)`. A uniform signal gives every file
  rank 1, contributing the identical constant `w·(K+1)/(K+1) = w` to each — it
  cannot reorder anything. Pro: principled (a non-discriminating signal is
  inert), path-independent fusion, minimal code. Con: changes Active-mode
  rankings wherever a signal has ties (i.e. most real queries), so the bench
  baseline and any Active-mode order assertions must be regenerated.
- **Keep ordinal + path tiebreak (the plan-mandated original):** rejected — it
  is the defect; a zero-information signal must not decide the ranking by
  filename.
- **Special-case only fully-uniform signals (skip them like weight-0):**
  rejected — brittle (an "almost uniform" signal still leaks a gradient), and
  competition ranking subsumes it correctly without a threshold to tune.
- **Drop the path tiebreak from the sort entirely:** rejected — the sort still
  needs a deterministic iteration order; competition ranking already makes that
  order irrelevant to the score, so keeping the cheap path key costs nothing.

## Decision

Assign competition ranks in `fuse`: after the `(score desc, path asc)` sort,
walk tie groups (`total_cmp == Equal`) and give every member of a group starting
at sorted position `i` the shared rank `i + 1`. The path-asc key now fixes only
a deterministic iteration order, not the score; fusion is fully path-independent.
The deterministic path-asc tiebreak that resolves equal *fused* scores lives
solely in seed selection (`(score desc, path asc)`, ADR-0188).

This supersedes the "ties fall back to path order, as the brief prescribes"
detail of the RRF design (ADR-0184). The `(K+1)`-normalization, `K = 60`,
weighted-RRF, and weight-0-skip properties are unchanged.

## Consequences

### Positive
- A uniform (non-discriminating) signal is provably inert — it can no longer
  overturn an informative signal by filename.
- Fusion is path-independent; the only path dependence in the whole pipeline is
  the one explicit downstream tiebreak (ADR-0188), which has unique keys (file
  paths) and is therefore fully deterministic across processes.
- The `Σweights = 1 ⇒ score ∈ [0,1]` invariant is preserved (rank ≥ 1 ⇒ each
  term ≤ w; a file rank-1 in every signal still scores exactly 1.0).

### Negative
- Active-mode rankings change wherever a signal has ties (most real queries), so
  the bench baseline is regenerated and Active-mode order assertions updated.
- The +164% Active-over-Inert recall figure (ADR-0187) was measured under
  ordinal ranking; it is re-validated on the recall-gate subset here, and a full
  31-PR A/B re-run remains the way to re-establish the headline number precisely.

### Measured (recall-gate subset: ripgrep#3420, flask#5928, express#7234)
- `cxpak (auto_context)` recall@8k/@32k: **0.6444 → 0.7889 (+22%)** — dense
  ranking *improves* the gated recall metric, so it strengthens rather than
  weakens the Active-over-Inert recall win.
- MRR: 0.7333 → 0.4286. Expected: flattening a uniform signal's contribution
  packs more relevant files into budget (higher recall) at the cost of a less
  sharply-ranked single top file (lower MRR). For context packing — where the
  LLM consumes the whole bundle — recall is the primary metric, which is why the
  gate keys on recall, not MRR. Baseline regenerated (`bench/baseline.json`) and
  the gate re-run in a fresh process reproduced the same recall (cross-process
  deterministic, ADR-0188).

### Neutral
- The determinism golden `spa_output_matches_golden_fixture` is unaffected
  (visual/PageRank path, not RRF).
- Inert mode (weighted sum) is untouched.

## Revisit if
- A signal is added whose semantics make "tied score" mean "genuinely
  incomparable" rather than "equally (ir)relevant" — then competition ranking
  may need a per-signal policy.
- A full-corpus A/B shows competition ranking materially changes the
  Active-vs-Inert conclusion (rank the change against ADR-0187's measurement).
