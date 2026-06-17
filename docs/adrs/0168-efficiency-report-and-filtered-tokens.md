---
id: '0168'
title: Efficiency report as decision-support (relevant-set coverage + budget-margin), aggregated from existing data; cost estimate opt-in
status: ACCEPTED
date: 2026-06-16
triggered_by: v2.3.0 W2 — no token-efficiency or cost reporting exists, despite the data being mostly collected already
loop: planning
---

# ADR-0168: Efficiency report from existing data + a token field on FilteredFile

## Context

`auto_context` already collects most of what a savings report needs. `AutoContextResult` carries `budget: BudgetSummary{total, used, remaining}`, `sections: PackedSections` (whose `PackedFileSection` has a `tokens: usize` field and `files: Vec<PackedFile{path, score, …}>`), and `filtered_out: Vec<FilteredFile>`; the index carries `total_tokens`; and at assembly the `kept` candidate list (post-noise `ScoredFileEntry{path, score, …}`, mod.rs:119) and every file's relevance score (`scorer.score_all`, mod.rs:96) are in scope. What is missing is (a) any aggregation/reporting of these into an efficiency view, and (b) a token count on filtered files — `FilteredFile` is `{path, reason}` (noise.rs), so "tokens saved by filtering" is not derivable today.

A naive coverage metric (`selected_tokens / repo_tokens`) is near-useless: on any large repo it is always a tiny single-digit percentage, and that is *correct* behavior — so the number tells the caller nothing actionable. The report should instead answer **"is this context good enough, and what should I change?"** — which requires metrics relative to the *relevant set*, not the whole repo.

## Options considered

- **Option A — decision-support report from existing data + add `FilteredFile.tokens` (chosen):** add `efficiency: EfficiencyReport` whose **headline is `relevant_coverage`** = (relevant candidates packed) / (relevant candidates = the post-noise `kept` set), with `selected/repo` retained only as a demoted `absolute_coverage` sanity field. Add **`marginal_included_score` / `marginal_excluded_score`** (lowest score in vs. highest score out, over `kept`) — a near-tie means the budget cut sits at the natural margin (healthy); a wide gap means the context is starved. Add a derived **`advisory: Vec<String>`** that speaks only when actionable (starving at a tight margin → raise budget; budget headroom + files filtered → lower threshold) and is silent when healthy. Plus `BudgetSummary` utilization and a new `tokens: usize` on `FilteredFile` (the noise filter has the count at filter time) for filtering savings. Cost estimate is **opt-in only** (`--cost <model>` / `cost_model` MCP param) using a small, dated rate table. All decision-grade signals compute at assembly (mod.rs:278) from data already in scope — **no `briefing.rs` change**; `PackedFile` already carries `score`. Pros: actionable guidance, not a passive dashboard; honest numbers from data we already have plus one field; cost figures never shown unless asked. Cons: one additive field; a few more report fields; rate table needs occasional updates. Chosen as the minimal way to make the report *useful*, not merely descriptive.
- **Option B — descriptive metrics only (`selected/repo` coverage, budget, savings), no decision-grade signals:** Pros: simplest; zero new computation. Cons: `selected/repo` is near-meaningless on large repos, and the report tells the caller nothing to *act* on. Rejected — a metric nobody acts on is decoration.
- **Option C — efficiency without filtering savings:** report only coverage and budget, leave `FilteredFile` unchanged. Pros: zero schema change. Cons: omits one of the most compelling numbers (tokens filtered as noise). Someone could prefer it to avoid touching the struct.
- **Option D — full cost dashboard with live pricing:** Pros: rich. Cons: live pricing is drift-prone and a maintenance/availability liability; scope creep. Rejected.

## Decision

Option A. `EfficiencyReport` is **decision-support**: relevant-set coverage as the headline, budget-margin scores, and a silent-unless-actionable advisory, all aggregated from existing fields at assembly; add `tokens` to `FilteredFile`; cost estimate opt-in with clearly-dated built-in rates.

## Consequences

### Positive
- The report answers "is this context good enough, and what do I change?" — coverage *of the relevant set*, the budget margin, and one line of guidance — instead of a passive readout.
- `relevant_coverage` + marginal scores localize the binding constraint (budget vs. relevance threshold), so the caller can act with one knob.
- A measurable "saved N tokens by filtering" story from data already collected plus one field.
- Cost numbers appear only on explicit request, so stale rates never silently mislead.

### Negative
- `FilteredFile` gains a field (additive; serialized output grows slightly).
- The built-in rate table will need periodic updates; it is dated and opt-in to contain the risk.

### Neutral
- Rendered through the existing md/json/xml renderers; off the selection critical path.

## Revisit if
- Rates change often enough that a built-in table is a burden — externalize it (config / fetched table) behind the same opt-in.
- Users want a per-section or per-file cost breakdown — extend `EfficiencyReport` rather than add a parallel report.
- The `kept`-based relevant set proves too narrow — add a **structural-closure coverage** signal (of the 1-hop dependency closure of the included files, what fraction is also included), reusing the graph, as an additional field rather than replacing `relevant_coverage`.
- The advisory's thresholds (`OMISSION`-style tuning: the 0.5 budget-headroom cutoff, the 0.15 margin gap) prove noisy or too quiet — tune against real runs, keeping them covered by the silent-when-healthy test.
