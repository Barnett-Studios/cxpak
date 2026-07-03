---
id: '0183'
title: Conventions surface honors token budget (no unbudgeted dump)
status: ACCEPTED
date: 2026-07-03
triggered_by: C4 task brief (cxpak 3.0.0 Phase C dogfood fix, spec §34)
loop: implementation
---

# ADR-0183: Conventions surface honors token budget (no unbudgeted dump)

## Context

The `conventions` capability op — exposed via MCP (`cxpak_conventions`), HTTP
(`POST /v1/conventions`), and LSP (`cxpak/conventions`) — previously serialised
the entire `ConventionProfile` as pretty JSON and returned it unbudgeted.  On a
large real-world repo the serialised profile reaches ~230 k tokens, roughly 46×
a sane MCP client per-message limit.  The `git_health.co_changes` field
(O(N²) file pairs for active repos), plus `churn_30d`/`churn_180d` and
`bugfix_density` (one entry per file), are the dominant contributors.

The three surfaces had separate render paths that each did the same unbudgeted
dump; the fix needed to go in one place and be shared.

## Options considered

- **Option A — Add a single budgeted core in `render.rs`, share it across all
  three surfaces:** The existing `src/conventions/render.rs` already owns the
  DNA rendering helpers and imports `TokenCounter`.  Adding the budgeted-render
  helper here keeps all convention-output logic co-located, and every surface
  calls the same function.  Pros: single source of truth, easy to test in
  isolation.  Cons: `render.rs` now has two concerns (markdown DNA + JSON
  budget), but the co-location is intentional.

- **Option B — Put the budget logic in each surface handler inline:** Avoids
  touching `render.rs` but triples the implementation surface and makes the
  logic drift-prone across HTTP/LSP/MCP.  Rejected — the brief names
  `render.rs` explicitly and shared core is the right architecture.

- **Option C — Add a new `src/conventions/budget.rs` module:** Clean
  separation, but adds a module for what is logically a rendering detail.
  Rejected — the additional indirection is not justified given the small scope.

## Decision

Add `MAX_MCP_CONVENTIONS_TOKENS: usize = 5_000` and `render_budgeted_conventions`
to `src/conventions/render.rs`.  All three surfaces (MCP op, HTTP handler, LSP
method) call this shared function after applying their existing
category/strength/focus filters.  The token budget is accepted as an optional
`tokens` parameter on every surface, defaulting to the constant.

**Default cap value — 5 000 tokens:**

5 000 was chosen because:
1. It is ~46× smaller than the observed unbudgeted ceiling (~230 k tokens),
   reducing worst-case context bleed from a full conventions query to a single
   compact page.
2. It is enough to carry every Convention- and Trend-strength pattern
   observation across all eight convention categories, plus a representative
   top-20 of git-health churn entries — the information a developer actually
   needs to understand a repo's conventions.
3. It fits comfortably within a typical MCP client per-message limit (~8 k
   tokens) without requiring the caller to think about budget at all.
4. It is intentionally different from the 50 k default used by briefing/
   overview ops: those ops stream file content, which warrants a larger
   budget; the conventions op streams structured metadata, which is far denser
   and requires far less volume to be actionable.

**Degradation order (deterministic; most-impactful first):**

All "fits?" checks use `token_budget − MARKER_RESERVE` (200 tokens) as the
effective ceiling so that the `_omitted` marker injected immediately after does
not push the final output over budget.  This is `budget_with_headroom` in the
code.

### Main stages (Steps 1–5)

1. Drop `git_health.co_changes` — grows as O(N²) file pairs; often the sole
   contributor to budget overflow on active repos.
2. Truncate `git_health.churn_30d` / `churn_180d` to 20 entries — Vecs are
   already ordered by `modifications desc` at build time, so truncation is
   stable.
3. Clear `git_health.bugfix_density` and `git_health.churn_trend` — both are
   `HashMap<String, _>` whose iteration order is non-deterministic; clearing
   the entire map is cheaper and safer than sorting-then-truncating.
4. Drop `additional` observation arrays from all categories — low-value
   catch-all fields.  Iterated in fixed category order (`CATEGORIES` constant)
   so output is deterministic.
5. Clear `testing.coverage_by_dir` — per-directory coverage data.

### Terminal stages (Steps 6–10; reached only on very tight budgets)

6. Clear `functions.by_directory` — O(directories) on large repos; the
   second-largest bulk source after `git_health.co_changes`.
7. Drop `dependencies.strict_layers` — O(layer-pairs).
8. Drop `dependencies.circular_deps` — O(detected cycles).
9. Truncate `git_health.churn_30d` / `churn_180d` further to 5 entries;
   drop `git_health.reverts`.
10. Clear all remaining churn arrays entirely.

After each step the response is re-measured; degradation stops as soon as the
output (minus marker headroom) fits.  No step is applied unless the budget is
still exceeded.

### Minimal-skeleton backstop

If all 10 steps are exhausted and the output still exceeds the budget, the
entire value is replaced with `{}` and the `_omitted` marker is injected.
Estimated output ≤ 280 tokens.

### Output guarantee

**`token_budget ≥ MIN_BUDGET_FLOOR` (300 tokens):** the returned output —
marker included — is guaranteed to be ≤ `token_budget`.

**Below 300 tokens:** the function still returns the minimal skeleton, but
because the `_omitted` marker itself costs ≈ 120–250 tokens the strict ≤
invariant cannot be maintained.  These budgets are not actionable anyway.

**Omission marker format:**

When content is dropped, a `_omitted` key is injected at the top level:

```json
{
  "_omitted": {
    "applied_budget": 5000,
    "original_tokens": 14200,
    "steps_applied": [
      "dropped git_health.co_changes (250 entries)",
      "truncated git_health.churn_30d to 20 entries (80 dropped)",
      "truncated git_health.churn_180d to 20 entries (80 dropped)"
    ],
    "note": "Response trimmed from ~14200 to fit within the 5000-token budget. ..."
  }
}
```

The marker is absent when the result fits under the budget without any
degradation (no false omission markers).

**Surfaces sharing the core:**

| Surface | Location | Budget param |
|---|---|---|
| MCP (`cxpak_conventions`) | `serve.rs` "conventions" arm | `tokens` string arg via `parse_token_count` or u64 |
| HTTP (`POST /v1/conventions`) | `v1_conventions_handler` | `tokens` field in JSON body |
| LSP (`cxpak/conventions`) | `lsp/methods.rs` | `tokens` field in params object |

The HTTP handler was upgraded from a parameterless POST to accept an optional
JSON body (`V1ConventionsParams`) matching the MCP interface.  An empty `{}`
body still works, preserving backward compatibility with the existing
`v1_conventions_returns_profile` test.

## Consequences

### Positive
- MCP clients no longer receive 230 k-token responses from a single conventions
  query; worst case is the configured budget (default 5 000).
- All three surfaces share the same degradation logic — a single fix, no drift.
- Callers that need more detail can pass `"tokens": "50k"` and receive the
  full profile.
- The omission marker makes the truncation transparent and actionable.

### Negative
- The HTTP handler now requires a JSON body (`Content-Type: application/json`);
  previously it accepted an empty body.  The test already sends `{}` so the
  break is minimal.
- `render.rs` now owns both markdown-DNA rendering and JSON budget logic; this
  is acceptable given the co-location rationale above.

### Neutral
- `auto_context` uses `render_dna_section` and `render_compact_dna` directly;
  `render_budgeted_conventions` is a separate function on a separate code path.
  The golden fixture `spa_output_matches_golden_fixture` is unchanged.

## Revisit if

- The default cap (5 000) proves too small for callers that need the full
  profile by default — raise the constant and re-run the acceptance tests.
- A new large field is added to `ConventionProfile` that the degradation order
  does not cover — add a step (between 5 and 6, or at 11) and update this ADR.
- The HTTP handler needs to accept GET requests (e.g. for browser tooling) —
  add a GET variant that applies the default cap.
- `MARKER_RESERVE` (200 tokens) proves too small as step counts grow and the
  `steps_applied` array in the marker lengthens — increase the constant and
  raise `MIN_BUDGET_FLOOR` accordingly.
- The LSP surface adds new parameters that also benefit from string-form token
  parsing — extend the `parse_token_count` pattern to those parameters.
