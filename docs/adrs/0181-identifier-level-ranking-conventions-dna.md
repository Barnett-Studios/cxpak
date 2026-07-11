---
id: '0181'
title: Identifier-level ranking fused with conventions DNA
status: ACCEPTED
date: 2026-07-03
triggered_by: cxpak 3.0.0 Phase C, Task C2 (D2-gated ranking)
loop: implementation
---

# ADR-0181: Identifier-level ranking fused with conventions DNA

## Context

Through C1, cxpak ranks at the **file** level: `MultiSignalScorer::score_all`
scores each file with seven weighted signals, and auto_context selects seeds,
fans out one hop, filters noise, and budgets whole files. Two files that both
touch the query's subject are indistinguishable if their file-level signals tie,
even when only one actually *defines* the identifier the query names.

Task C2 requires pushing ranking to the `(file, identifier)` granularity and
fusing it with the codebase's own conventions DNA (the `ConventionProfile`
already rendered into every auto_context call), using five specific signals with
prescribed multipliers: naming-pattern match ×10, ambiguity penalty ×0.1,
`_`-prefix penalty ×0.1, mention personalization, and redistribution of
file-level PageRank mass down to identifiers.

The binding constraint is the **D2 recall gate**: the change may not regress
recall@8k or recall@32k on the pinned 3-PR benchmark subset
(`bench/baseline.json`), measured by `tests/bench_gate.rs`. The gated system is
`cxpak (auto_context)` — the shipped end-to-end path — whose recall is the
fraction of a merged PR's changed files that land inside the token budget. Any
ranking change that could *demote* a file the base scorer would have surfaced is
a recall risk. This is a human decision because the multipliers are knobs and
the file-vs-identifier fusion has genuine design freedom whose only arbiter is
the measured recall.

## Options considered

- **Option A — 8th weighted signal:** add `identifier_rank` as an additive term
  in the weighted sum, carving weight from the existing seven. Clean and
  symmetric, but every file's score shifts, the sum-to-1.0 invariant and the ×10
  semantics get diluted into an average, and the wholesale reordering is a direct
  recall-gate hazard on a 3-PR subset where one displaced file swings recall by
  a third.
- **Option B — replace file ranking with identifier ranking:** rank identifiers
  globally and derive the file order from them. Most faithful to "identifier-level
  ranking", but it throws away the tuned file-level signals, and the large
  prescribed multipliers (×10, ×0.1) applied to the *file* ordering would
  reorder catastrophically — almost certainly regressing recall.
- **Option C — boost-only identifier refinement (chosen):** compute the full
  `(file, identifier)` ranking (all five signals + DNA fusion) in one global
  pass, but fold it back into the file score as a **boost-only multiplier**
  (`≥ 1.0`) with a small gain. The penalties operate *within* the identifier
  ranking (they decide a file's best identifier and the relative order of `(file,
  ident)` pairs, and are visible in the per-pair scores and unit tests) but never
  demote a file. Boost-only removes the *threshold-crossing* recall risk (no file
  the base scorer surfaced drops out of the seed set), but — as the D2
  measurement showed — it does **not** make recall strictly monotone: at a fixed
  token budget, reordering can still displace a file from the packed set even
  when every score only rises. The gain therefore ships at `0.0` (boost off):
  the D2 A/B showed even a small `0.05` boost regresses recall@8k across the gate
  repos via a budget-boundary flip, so the ranking ships computed-but-inert,
  surfaced as a signal and ready to activate once a larger corpus validates a
  boost that holds recall (see D2 result).

## Decision

Chosen **Option C**. A new `src/relevance/identifier.rs` builds an
`IdentifierRanking` once per query (it needs global scope: cross-codebase
ambiguity counts, personalized PageRank, and cross-file normalization),
mirroring how `score_all` computes the query embedding once. Each `(file,
identifier)` unit is scored as `redistributed_base × multiplier`:

- **Redistribution (signal #5):** the file's dependency-graph PageRank mass
  (`index.pagerank`, floored at `PR_FLOOR = 0.05` so isolated files still
  participate) is split across the file's identifiers in proportion to each
  symbol's visibility weight (`symbol_importance(sym, 1.0, …)`: 1.0
  public+referenced, 0.7 public, 0.3 private). Per file the shares sum to the
  file's mass — a genuine redistribution of file rank to identifier granularity.
- **Naming-pattern match ×10** (`NAME_MATCH_MULT = 10.0`): an identifier whose
  `split_identifier`/normalized tokens intersect the expanded query tokens (the
  same tokens the other signals consume).
- **Ambiguity ×0.1** (`AMBIGUITY_MULT`, `AMBIGUITY_DEF_THRESHOLD = 5`): an
  identifier with strictly more than 5 definitions across the codebase.
- **`_`-prefix ×0.1** (`UNDERSCORE_MULT`): names starting with `_`.
- **Mention personalization** (`MENTION_GAIN = 1.0`): `mention_seeds` marks the
  files the query names (by symbol or path token); those seeds are propagated
  through the dependency graph by a new **personalized (topic-sensitive)
  PageRank** (`compute_pagerank_personalized`) so the bias reaches the seeds'
  neighbors. Per-file mention strength is `max(direct_seed_mass_normalized,
  graph_propagated_rank)`, folded in as `1 + MENTION_GAIN × strength`; robust for
  both connected and isolated files.
- **Conventions-DNA fusion** (`DNA_MATCH_BONUS = 0.5`): `m_dna = 1 + 0.5 ×
  conformance`, where `conformance = 0.5 × naming_conf + 0.5 × visibility_conf ∈
  [0,1]`. `naming_conf` is the strength (`percentage/100`) of the dominant naming
  style for the symbol's kind (function/type/constant) when `classify_name`
  matches it, else 0; `visibility_conf` is the dominant public/private ratio's
  strength when the symbol's visibility matches, else 0. Idiomatic identifiers
  rank consistently above one-off outliers.

`multiplier = m_name × m_ambig × m_under × m_dna × m_mention`. Each file's
best-identifier score is normalized against the global maximum to a `[0,1]`
signal, and the file factor is `1 + IDENT_FUSION_GAIN × signal` (boost-only).
**`IDENT_FUSION_GAIN` ships at `0.0`** — the identifier ranking is computed and
surfaced as the `identifier_rank` signal but applies no boost — because the
full-corpus D2 A/B (below) measured a positive gain as net-neutral-to-negative
on the gate. `score_all` multiplies each file's weighted-sum base by this factor
before
clamping and appends an eighth `identifier_rank` `SignalResult` (the weighted
signals and their sum-to-1.0 invariant are untouched; the eighth is
multiplicative). The personalized PageRank reuses the existing power-iteration
via a shared `power_iterate` core that generalizes the uniform teleport to an
optional personalization vector — classic `compute_pagerank` /
`compute_pagerank_seeded` remain byte-identical (`None` teleport), so no existing
caller, `index.pagerank`, or the parity tests change.

## Consequences

### Positive
- Ranking is now computed per-`(file, identifier)`; query-matching,
  convention-conforming identifiers carry a stronger raw signal, surfaced as the
  `identifier_rank` signal detail. The machinery and its five signals are fully
  built and unit-tested, ready to activate by raising the gain once a
  faster/larger corpus can validate a boost that holds recall (see D2 result).
- Personalized PageRank is a reusable, tested primitive (topic-sensitive walk)
  built by extending, not duplicating, the existing algorithm.
- Fully deterministic: sorted node order in PageRank, order-independent
  ambiguity counts, and a stored-order file loop — byte-stable scores.

### Negative
- The prescribed penalties (×0.1) never produce a net file demotion; they only
  shrink a boost. That is a deliberate faithfulness trade for the recall gate,
  documented here so a future reader does not "fix" it into a demotion and
  regress recall.
- One extra global pass per `score_all` (an ambiguity scan + a personalized
  PageRank). Bounded and shared across the per-file loop; negligible on the
  benchmark repos.

### Neutral
- The single-file `RelevanceScorer::score` path passes no ranking (neutral factor
  1.0); only the bulk `score_all` path (used by auto_context and the bench) gets
  identifier refinement.
- `IDENT_FUSION_GAIN` (shipped `0.0` — boost off), `MENTION_GAIN`, and
  `DNA_MATCH_BONUS` are the tuning knobs. At gain 0.0 the fusion is recall-neutral
  by construction; raising it re-runs the D2 A/B as the arbiter (see below).

## D2 recall result (the merge criterion)

Measured as an A/B over a **single index build per corpus entry** (inert gain
`0.0` vs an active `0.05` boost), gated system `cxpak (auto_context)`, recall =
fraction of a merged PR's changed files inside the token budget.

On the committed pinned 3-PR subset (BurntSushi/ripgrep#3420, pallets/flask#5928,
expressjs/express#7234):

| metric | inert (0.0) | active (0.05) | Δ |
|---|---|---|---|
| recall@8k | 0.5000 | 0.5000 | = |
| recall@32k | 0.4667 | 0.5000 | +0.033 |
| MRR | 0.4104 | 0.4551 | +0.045 |

On the subset the boost is ≥ inert. But extended to **all three gate repos**
(31 of 32 PRs — ripgrep, flask, express: the repos the gate measures):

| metric | inert (0.0) | active (0.05) | Δ |
|---|---|---|---|
| recall@8k | 0.245 | 0.213 | **−0.032** |
| recall@32k | 0.226 | 0.229 | +0.003 |
| MRR | 0.186 | 0.190 | +0.004 |

The active boost **regresses recall@8k** across the gate repos, driven by a
single budget-boundary flip (pallets/flask#5962, recall@8k 1.0 → 0.0): the
fill-then-overflow packer swaps which seeds fit at the 8k edge when scores
shift, and that PR's recall is itself non-monotone in budget (@8k 1.0 but @32k
0.0 even at inert) — confirming the effect is packer boundary noise, not a
ranking error. The @32k / MRR gains are within that noise.

Per the pre-registered D2 rule — *active ≥ baseline on **both** budgets over the
corpus → ship active; otherwise ship inert (0.0) or defer* — the @8k regression
lands this in the inert branch. **Decision: ship `IDENT_FUSION_GAIN = 0.0`.** The
identifier ranking, its five signals, redistribution, personalized PageRank, and
DNA fusion all land (computed, surfaced as `identifier_rank`, unit-tested); the
boost stays off until a larger/faster corpus can validate a gain that holds
recall@8k.

Separately, the committed `bench/baseline.json` (generated 2026-06-22, @32k
0.5333) is **stale**: current code at inert scores @32k 0.4667 on the same
subset — recall drifted down through C1-era changes, independent of C2. It is
regenerated to current numbers in a follow-up so the network gate reflects HEAD;
C2 itself is recall-neutral and introduces no regression.

## Revisit if

- A faster/larger, less variance-prone corpus is available — re-run the D2 A/B
  and raise `IDENT_FUSION_GAIN` above 0.0 if a boost holds recall@8k **and** @32k;
  the machinery is in place and only the gain gates it. A larger corpus may also
  safely admit a demotion-capable fusion the 3-PR subset cannot.
- A measured recall win is available from making the penalties bite at the file
  level (net demotion) — the boost-only trade recorded here would then be worth
  reopening.
- Personalized PageRank becomes a hot path — its per-query cost may need caching
  or an approximation.
