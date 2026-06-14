---
id: '0097'
title: Conventions report what IS (evidence-based), never what SHOULD BE (prescriptive)
status: ACCEPTED
date: 2026-03-27
triggered_by: Defining the philosophy of the convention extraction engine
loop: planning
---

# ADR-0097: Conventions report what IS (evidence-based), never what SHOULD BE (prescriptive)

## Context
The v1.1.0 convention extraction engine (`src/conventions/`) had to choose between
two stances. It could prescribe best practices — an opinionated linter that flags
deviations from a curated ruleset — or it could describe the codebase's own observed
patterns, every finding backed by evidence. The former injects external opinion that
is not portable across project styles; the latter is deterministic and defensible.
The design doc fixed the principle up front: "report what IS, never what SHOULD BE."

This is a human decision because it sets the engine's epistemic contract: whether the
tool asserts judgments or supplies evidence and lets the LLM draw its own conclusions.
The shipped `PatternObservation` (name, dominant, count, total, percentage, strength,
exceptions) and `PatternStrength` (Convention ≥90%, Trend 70–89%, Mixed 50–69%) encode
that contract directly.

## Options considered
- **Option A — evidence-based descriptive extraction:** every finding is backed by
  counts, percentages, exceptions, and git history; the tool never asserts what the
  code should do. Pros: deterministic, defensible, no opinions to maintain; the LLM
  draws its own conclusions; works for any codebase regardless of its style. Cons:
  cannot catch a genuinely bad-but-consistent pattern — a codebase that uniformly does
  something wrong has it reported as a "convention." Someone could prefer this for its
  portability and the absence of any opinion to defend. (Chosen.)
- **Option B — prescriptive best-practice linting:** ship a curated ruleset of
  recommended practices and flag deviations from it. A reasonable alternative would
  have been to bundle opinionated rules. Pros: catches objectively bad patterns even
  when they are consistent across the codebase. Cons: opinionated, not portable across
  project styles, and requires maintaining a ruleset over time. Someone could prefer it
  precisely because it would surface uniformly-wrong patterns that descriptive
  extraction reports as conventions.

## Decision
The engine reports what IS, never what SHOULD BE. Every finding is backed by counts,
percentages, and exceptions; the LLM draws conclusions from the evidence rather than
the tool prescribing rules. Implemented in `src/conventions/` via `PatternObservation`
and `PatternStrength`, with `render.rs` emitting evidence-backed findings and
`verify.rs` constructing violation evidence from the codebase's own dominant pattern
rather than an external rule.

## Consequences
### Positive
- Deterministic and defensible output.
- Portable across any codebase style — no bundled opinion.
- No external ruleset to maintain.

### Negative
- A uniformly-wrong codebase has its mistakes reported as conventions; any ≥50%
  dominant pattern is reported as a convention or trend regardless of quality.

### Neutral
- Conclusions about whether a convention is good are left to the LLM consuming the
  evidence.

## Revisit if
- Users want an opinionated best-practice mode layered on top of descriptive
  extraction.
- A class of objectively harmful patterns proves common enough that silently reporting
  them as conventions causes measurable harm.
