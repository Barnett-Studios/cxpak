---
id: '0073'
title: Five-level progressive degradation model for budget-constrained rendering
status: ACCEPTED
date: 2026-03-19
triggered_by: Need to fit the most useful context into a fixed LLM token budget
loop: planning
---

# ADR-0073: Five-level progressive degradation model for budget-constrained rendering

## Context
cxpak packs codebase context into a fixed LLM token budget. v0.11.0 introduced a
hybrid degradation model that separates storage from rendering: the index always
stores whole symbols (so `trace`/`search` keep full fidelity), and the budget
allocator degrades *rendering* through five discrete detail levels —
`Full`, `Trimmed`, `Documented`, `Signature`, `Stub`.

A fast path skips degradation entirely when the raw token sum already fits the
budget. The slow path renders everything at Level 0 (Full), then steps dependency
files down levels 0–4 lowest-priority-first, then steps selected files down but
never below Level 2 (Documented), then drops dependency files as a last resort.

Shipped in `src/context_quality/degradation.rs`.

## Options considered
- **Option A — five discrete detail levels with role-aware degradation:**
  `Full`/`Trimmed`/`Documented`/`Signature`/`Stub`; degrade dependencies first,
  then selected files (with a Documented floor), and drop dependencies only as a
  last resort. Pros: meaningfully distinct levels and a budget filled with the
  highest-value detail first; respects user intent via per-role floors. Cons: more
  rendering logic and per-step token recomputation. Someone could prefer this for
  its granularity and its protection of user-selected files. (Chosen.)
- **Option B — binary include/exclude per file:** a file is either fully included
  or dropped. A reasonable alternative would have been the simplest possible
  allocator. Pros: trivial to implement. Cons: wastes budget on the all-or-nothing
  boundary and loses the partial-context value of a trimmed or signature-only view.
  Someone could prefer it purely for implementation simplicity.
- **Option C — store pre-degraded representations in the index:** keep trimmed
  forms in the index rather than whole symbols. Pros: less render-time work. Cons:
  `trace`/`search` lose full fidelity; the design explicitly keeps full data
  available. Someone could prefer it to avoid recomputing detail at render time.

## Decision
Adopt the hybrid model: the index stores whole symbols, and the budget allocator
degrades through five levels with role-based minimums — `Selected` files never go
below `Documented` (Level 2), `Dependency` files are droppable. A fast path skips
degradation when content already fits. Implemented in
`src/context_quality/degradation.rs` via `allocate_with_degradation()`, which
recomputes total tokens after each degradation step.

## Consequences
### Positive
- Full symbol data remains available for `trace`/`search`.
- The budget is filled with the highest-value detail first.
- User-selected files are protected from over-degradation by the Documented floor.

### Negative
- Degradation requires recomputing total tokens after each step.
- Five levels add rendering complexity (doc-comment extraction, signature
  reduction, stub generation).

### Neutral
- The fast path optimizes the common small-repo / large-budget case.

## Revisit if
- Token recomputation becomes a performance bottleneck.
- A detail level proves redundant in practice.
- LLM context windows grow large enough that degradation is rarely triggered.
