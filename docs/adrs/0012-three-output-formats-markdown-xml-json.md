---
id: '0012'
title: Render to three output formats (markdown default, XML, JSON) via a format-dispatch boundary
status: ACCEPTED
date: 2026-03-05
triggered_by: The context bundle must be consumable both by humans and by programmatic/agent consumers
loop: planning
---

# ADR-0012: Render to three output formats (markdown default, XML, JSON) via a format-dispatch boundary

## Context

Different consumers of the v0.1.0 context bundle prefer different shapes: markdown for human/LLM prose, XML for structured prompt embedding, JSON for tooling. The design offers all three with markdown as the default, dispatched behind a single `render()` entry point that delegates to per-format renderers. Section content is pre-rendered as strings so all three formats share the same `OutputSections` payload.

## Options considered

- **Option A — Three renderers behind one dispatch (md/xml/json):** `render(sections, format)` delegates to markdown/xml/json modules over a shared `OutputSections`. Pros: one bundle serves multiple consumers, with a clean per-format module boundary. Cons: three renderers to keep in sync. Someone could prefer this to serve humans, prompt-embedding, and tooling from a single computed bundle.
- **Option B — Markdown only:** A reasonable alternative would have been emitting a single human/LLM-oriented format. Pros: less code. Cons: no structured form for tooling or agents. Someone could prefer it to ship faster with one well-polished format.

## Decision

Support markdown (default), XML, and JSON output via a `render(sections, format)` dispatch that delegates to dedicated per-format renderer modules sharing a pre-rendered `OutputSections` payload.

## Consequences

### Positive
- The same bundle serves human, prompt-embedding, and tooling consumers.
- The per-format module boundary keeps renderers isolated.

### Negative
- Three renderers must be kept behaviorally consistent.

### Neutral
- JSON uses serde with skip-empty fields; XML uses quick-xml as the XML dependency with manual escaping in the renderer; detail-file extensions later key off the chosen format in Pack Mode.

## Revisit if
- A new consumer needs a format not expressible from the shared `OutputSections` string payload.
