---
id: '0032'
title: Default 50k token budget, always prompt the user (except clean)
status: ACCEPTED
date: 2026-03-11
triggered_by: Every context-producing command needs a token budget; cxpak budgets and truncates against it
loop: planning
---

# ADR-0032: Default 50k token budget, always prompt the user (except clean)

## Context

cxpak (v0.4.0) allocates and truncates its output against a token budget. The Claude Code plugin standardizes on a 50k default and a policy that every context-producing skill and command asks the user for the budget before invoking cxpak. The `clean` command is the deliberate exception — it has no budget concept.

The implementation encodes this: budget-bearing entry points contain both `50k` and an explicit ask-for-budget instruction, while the `clean` test asserts the absence of any budget/token mention.

## Options considered

- **Option A — 50k default, always ask (clean exempt) (chosen):** Prompt for the budget each time, defaulting to 50k; `clean` asks nothing. Pros: the user controls the cost/coverage tradeoff explicitly each run; consistent UX across entry points. Cons: adds a prompt round-trip on every invocation. Someone could prefer this for the explicit per-call control it gives.
- **Option B — silent fixed default:** A reasonable alternative would have been using 50k without asking, for less friction. Rejected because the user cannot then tune coverage at call time; someone might prefer it for the lower interaction cost.

## Decision

Standardize a 50k token-budget default and require every context-producing skill/command to ask the user for the budget before invoking cxpak. The `clean` command is the sole exception with no budget question, enforced by a test asserting it never mentions budget or token.

## Consequences

### Positive
- Explicit user control over context size/cost per call.
- Consistent UX across all entry points.
- `clean` stays friction-free.

### Negative
- (none identified)

### Neutral
- A prompt round-trip is incurred on every budgeted invocation.

## Revisit if
- The always-ask prompt proves annoying in practice.
- A different sensible default emerges from usage.
