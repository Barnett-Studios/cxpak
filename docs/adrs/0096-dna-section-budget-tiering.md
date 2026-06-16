---
id: '0096'
title: DNA section is step 0 of auto_context, deducted before fill-then-overflow, never degraded, budget-tiered
status: ACCEPTED
date: 2026-03-27
triggered_by: Including the ~1000-token Repository DNA in every auto_context call without it competing with code sections
loop: planning
---

# ADR-0096: DNA section is step 0 of auto_context, deducted before fill-then-overflow, never degraded, budget-tiered

## Context

Released in v1.1.0 (Repository DNA). The Repository DNA section describes the codebase's conventions and applies globally to all generated code, making it the highest signal-per-token content `auto_context` can provide. But it must coexist with packed source code inside a fixed token budget, and very small budgets cannot afford it at all. The question was how DNA should interact with the existing fill-then-overflow section allocator.

## Options considered

- **Option A — Deduct DNA before section allocation, never degrade it, tier by budget size (chosen):** Render DNA as step 0, subtract its tokens from the budget before `allocate_and_pack`, and store it as a separate `dna` field on `AutoContextResult` (a peer of `sections`, not a `PackedSection`). Skip DNA entirely below 2000 tokens; use a compact ~300-token version (top 3 conventions) for 2000–5000; use the full ~1000-token version above 5000. Pros: the universal constraint is protected from budget pressure; clean separation from the section priority system; degrades gracefully on small budgets. Cons: DNA is a special case outside the normal section allocator, and ~2% of budget is always consumed on larger requests. Someone could prefer this so conventions can never be dropped under pressure.
- **Option B — Treat DNA as a normal priority section:** Let DNA compete in the fill-then-overflow allocator like tests, schema, and blast radius. Pros: one uniform allocation mechanism. Cons: DNA could be degraded or dropped under budget pressure, defeating its purpose as a universal constraint that applies to all generated code. Someone could prefer it for the single uniform allocation path.

## Decision

Add DNA as step 0 of the `auto_context` pipeline, stored in a new `pub dna: String` field on `AutoContextResult` (a peer of `sections`, not a section). Its token count is subtracted from the budget BEFORE fill-then-overflow allocation; `allocate_and_pack` receives `token_budget - dna_tokens` with no signature change. DNA is never degraded or dropped. Budget tiering: skip below 2000 tokens; emit a compact ~300-token summary (top 3 conventions) for 2000–5000; emit the full ~1000-token DNA above 5000.

## Consequences

### Positive
- Conventions reach the LLM on every sufficiently-budgeted call.
- DNA is protected from section-level degradation.
- Small budgets still produce useful output by degrading or omitting DNA.

### Negative
- DNA is a special path outside the section allocator.
- It always consumes ~1000 tokens on large-budget calls.

## Revisit if
- The 2000/5000 token tier thresholds prove poorly calibrated.
- DNA token cost meaningfully starves code sections on common budgets.
