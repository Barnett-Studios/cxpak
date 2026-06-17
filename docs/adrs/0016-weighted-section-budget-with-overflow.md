---
id: '0016'
title: Weighted per-section token budget with fixed metadata floor (design-time overflow not implemented)
status: ACCEPTED
date: 2026-03-05
triggered_by: A fixed token budget must be divided across the seven overview output sections by importance
loop: planning
---

# ADR-0016: Weighted per-section token budget with fixed metadata floor

## Context

The overview output has seven sections of differing value, and a fixed token budget must be
divided across them by importance. This decision was taken during initial design (the overview
MVP, pre-v0.2.0; there is no v0.1.0 release tag). The design assigns each section a percentage
weight (Function/Type Signatures 30%, Module Map 20%, Key Files 20%, Dependency Graph 15%, Git
10%, Directory Tree 5%) plus a fixed ~500-token metadata floor.

The design doc also specified a surplus-overflow scheme — if a section underfills, its
remainder should flow to the next section down or distribute proportionally. The weighted split
with the fixed metadata floor shipped; the overflow/redistribution behavior remained a design
intention and was never implemented (see Consequences).

## Options considered

- **Option A — fixed metadata + weighted percentages (+ proposed overflow):** Metadata gets a
  fixed ~500 tokens; the remaining budget is split by section weights; per the design, surplus
  was to flow down or distribute proportionally. Pros: important sections get space first, and
  the scheme is simple to reason about. Cons: the weights are hand-tuned heuristics, and the
  overflow half added complexity that ultimately was not built. The weighted split shipped;
  overflow did not.

- **Option B — equal split across sections:** A reasonable alternative would have been to
  divide the budget evenly. Pros: trivial. Cons: ignores that signatures and the module map
  carry most of the core value. Not formally evaluated.

- **Option C — pure priority fill (no fixed weights):** A reasonable alternative would have
  been to fill the highest-priority section fully, then the next, until the budget is
  exhausted. Pros: maximizes top-priority content. Cons: low-priority sections can be starved
  entirely, producing a less balanced bundle. Not formally evaluated.

## Decision

Allocate the token budget with a fixed ~500-token metadata floor and distribute the remainder
by per-section weights (signatures 30%, module map 20%, key files 20%, dependency graph 15%,
git 10%, directory tree 5%). The design doc additionally specified that an underfilling
section's surplus should flow to the next section down or distribute proportionally; that
overflow scheme was specified but NOT implemented in `BudgetAllocation::allocate()` or
`overview.rs`.

## Consequences

### Positive
- The highest-value sections (signatures, module map) get the most space.

### Negative
- Weights are static heuristics, not tuned per repo.
- The surplus-redistribution part of the design was never coded: `allocate()` computes the
  static percentage slices once, and `overview.rs` passes each slice straight to its renderer
  with no carry-over or reclaim. Underspent budget in one section is simply wasted rather than
  flowing to another.
- `auto_context` later moved to a separate fill-then-overflow allocator
  (`auto_context/briefing.rs`) for its own pipeline, distinct from `BudgetAllocation`.

### Neutral
- Encoded as `BudgetAllocation::allocate()` with `METADATA_FIXED = 500` and the exact
  percentage constants from the design table; the surplus-redistribution part of the design
  was never coded.

## Revisit if
- Static weights produce poor bundles for atypical repos.
- A learned/dynamic allocation outperforms fixed weights.
