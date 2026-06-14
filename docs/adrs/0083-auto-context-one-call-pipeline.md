---
id: '0083'
title: Compose all cxpak intelligence into a single cxpak_auto_context tool with a fixed JSON briefing
status: ACCEPTED
date: 2026-03-22
triggered_by: v1.0.0 capstone — give the LLM one tool that returns optimal context for a task without manual tool chaining
loop: planning
---

# ADR-0083: Compose all cxpak intelligence into a single cxpak_auto_context tool with a fixed JSON briefing

## Context

cxpak v1.0.0 is the capstone release. Rather than make the LLM chain together sub-tools
(overview, trace, blast radius, API surface, and so on) by hand, this release adds a single
tool that returns optimal context for a task in one call.

`auto_context` runs a 10-step pipeline — query expansion, 7-signal relevance scoring, seed
selection, noise filtering, test enrichment, schema context, blast radius, API surface,
fill-then-overflow budget allocation, and annotations — and always returns a structured JSON
briefing. It is added as MCP tool #10 but positioned first in `tools/list`, with no format
parameter and no task classifier. The orchestration lives in `src/auto_context/mod.rs`.

## Options considered

- **Option A — single one-call pipeline, structured JSON, listed first:**
  orchestrate the existing infrastructure, always return JSON with the raw annotated source
  embedded, place the tool at position 0 in `tools/list`, and expose no format parameter. One
  tool covers most tasks, the LLM sees it first, and it reuses every existing component. The
  cost is less manual control than calling sub-tools individually and an opinionated fixed
  ordering. Chosen.

- **Option B — task-classified response formats:** classify the task and emit different
  bundles or formats per task type. Someone could prefer this for output tailored to the task.
  Rejected: relevance scoring already handles emphasis, so a separate classifier is unnecessary.

- **Option C — markdown/JSON/XML format parameter like `overview`:** let the caller pick the
  output format. Someone could prefer this for consistency with `cxpak_overview`. Rejected: MCP
  transports JSON and the content is itself source code, so a markdown wrapper is unnecessary.

## Decision

Implement `auto_context()` composing query expansion, 7-signal relevance scoring, seed
selection, the noise filter, test enrichment, schema context, blast radius, API surface,
fill-then-overflow budget allocation, and annotations, returning a fixed structured-JSON
briefing (`sections` + `budget` + `filtered_out`). Add it as MCP tool #10 but position it
first in `tools/list`; provide no format parameter and no task classifier. Deprecate no
existing tools.

## Consequences

### Positive
- One call yields optimal context and reuses every prior subsystem.
- Listed first, so the LLM is most likely to reach for it.
- No breaking changes; the sub-tools remain available for fine-grained control.

### Negative
- Opinionated, fixed section ordering and format.
- The pipeline binds many subsystems together, raising the integration surface.

### Neutral
- The `filtered_out` array gives the LLM transparency to override via `pack_context`.
- Annotations are applied inside `allocate_and_pack`.

## Revisit if
- Task-specific bundles measurably beat the universal format.
- A format parameter is needed for non-MCP consumers.
