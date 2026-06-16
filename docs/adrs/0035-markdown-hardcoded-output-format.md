---
id: '0035'
title: Hardcode --format markdown for all plugin invocations
status: ACCEPTED
date: 2026-03-11
triggered_by: cxpak supports markdown/json/xml output; plugin must pick one for in-context injection
loop: planning
---

# ADR-0035: Hardcode --format markdown for all plugin invocations

## Context

cxpak (v0.4.0) can emit markdown, JSON, or XML. The Claude Code plugin always feeds output into Claude's context window. The design hardcodes markdown as the format for every skill and command rather than exposing the choice, on the rationale that markdown is native to the context window and is the most token-efficient form to inject.

The implementation enforces this for skills with a test asserting `--format markdown` appears in each SKILL file. The command files also hardcode markdown but are not test-enforced for it.

## Options considered

- **Option A — hardcode markdown (chosen):** Always pass `--format markdown`. Pros: native to the context window, no user decision, testable invariant. Cons: loses JSON/XML for programmatic downstream use. Someone could prefer this for minimal in-context token overhead.
- **Option B — expose format choice:** A reasonable alternative would have been letting the user pick markdown/json/xml per invocation for flexibility. Rejected because it adds prompt friction and JSON/XML are token-heavier in-context; one could prefer it when piping output to a structured consumer.

## Decision

Always invoke cxpak with `--format markdown` across all skills and commands; do not expose the format flag to the user. Tests grep each SKILL file for `--format markdown` (the command files hardcode it but are not test-enforced for it).

## Consequences

### Positive
- Minimal token overhead in-context.
- No format decision burden on the user.
- Enforced by tests (for the SKILL files).

### Negative
- (none identified)

### Neutral
- JSON/XML formats remain available via the CLI but not through the plugin.

## Revisit if
- A consumer needs structured JSON output through the plugin.
