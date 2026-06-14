---
id: 'NNNN'                # quoted — bare 0001 is parsed by YAML as integer 1, breaking cross-references
title: <short decision title>
status: PROPOSED          # PROPOSED | ACCEPTED | DEPRECATED | SUPERSEDED by ADR-MMMM
date: YYYY-MM-DD          # ISO-8601 date the decision was made
triggered_by: <command / agent / PR / design doc / version that raised this decision>
loop: planning            # planning | implementation — when this decision became visible
---

# ADR-NNNN: Title

## Context

What is the issue motivating this decision? Quote the requirement, constraint, or observation that forced the choice. State *why it belongs to a human decision* rather than being something the code could resolve on its own.

## Options considered

- **Option A — <name>:** brief description — pros / cons / what a stakeholder might legitimately prefer about this option
- **Option B — <name>:** brief description — pros / cons / what a stakeholder might legitimately prefer
- **Option C — <name>:** brief description — pros / cons / what a stakeholder might legitimately prefer

Each option must be one a reasonable person could choose. No strawmen padding a single real choice. For each *rejected* option, state honestly why someone could have preferred it.

## Decision

What we chose, in one or two sentences. Name the deciders if it matters.

## Consequences

### Positive
-

### Negative
-

### Neutral
-

## Revisit if

The conditions that would legitimately reopen this decision:

- <a load profile or scale assumption changes that invalidates the chosen trade-off>
- <a dependency / upstream constraint is removed or added>
- <a measured outcome contradicts the rationale recorded here>

This section is what makes the decision auditable as it ages. Leave it concrete and observable, not aspirational.
