---
id: '0087'
title: Allocate the auto_context token budget by strict-priority fill-then-overflow, not fixed proportions
status: ACCEPTED
date: 2026-03-22
triggered_by: auto_context must pack five section types into one token budget deterministically
loop: planning
---

# ADR-0087: Allocate the auto_context token budget by strict-priority fill-then-overflow

## Context

`auto_context` (v1.0.0) must pack five section types — target files, test files, schema context, API surface, and blast radius — into a single token budget, deterministically. The chosen approach packs sections in strict priority order: each level receives whatever budget remains after higher-priority levels are satisfied, and content that overflows a level is line-truncated or dropped rather than rebalanced across sections.

## Options considered

- **Option A — Fill-then-overflow by priority:** Pack priority 1 through 5; each level uses the remaining budget; files within a level are ordered by composite score; the overflowing file is line-truncated, then the section stops. Pros: deterministic, maximizes density, never wastes budget on empty sections. Cons: low-priority sections (blast radius, API surface) can be squeezed out entirely under tight budgets. This was the chosen option.
- **Option B — Fixed proportions per section:** Reserve a fixed share per section (e.g., 50% targets / 20% tests / …). Pros: predictable section sizes. Cons: wastes budget on empty or small sections; someone could prefer it for predictability, but density suffers.
- **Option C — Adaptive allocation:** Dynamically rebalance the budget across sections by content. Pros: potentially the optimal fit. Cons: non-deterministic and harder to reason about, which conflicts with the determinism requirement.

## Decision

Implement `allocate_and_pack()` to fill sections in strict priority order (target files, test files, schema context, API surface, blast radius), giving each level the budget remaining after higher levels, ordering files within a level by descending composite score. When a section's content overflows, the packer packs full files until the budget tightens, then line-truncates the single overflowing file (via the local `truncate_to_budget` helper, which appends a `// ... (truncated)` marker) before moving to the next level. A section that does not fit at all is skipped rather than partially rendered.

## Consequences

### Positive
- Deterministic and density-maximizing.
- No budget wasted on empty sections.
- Highest-value target files are always packed first.

### Negative
- Under tight budgets, blast radius and API surface may be omitted entirely.
- The priority ordering is fixed and opinionated.

### Neutral
- Within-section overflow is handled by line-level truncation (the local `truncate_to_budget`), not symbol-level degradation; the emitted `detail_level` is only `full` or `truncated`. (The v0.11.0 progressive symbol degradation in `context_quality` is not invoked by this packer.)

## Revisit if
- Users need configurable section priorities.
- The fixed ordering starves a section that matters for some task type.
