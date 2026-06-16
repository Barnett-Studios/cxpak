---
id: '0063'
title: Add cxpak_search regex MCP tool with line-based content search
status: ACCEPTED
date: 2026-03-18
triggered_by: No content-search surface existed across the indexed codebase
loop: planning
---

# ADR-0063: Add cxpak_search regex MCP tool with line-based content search

## Context

Through v0.10.0 there was no way to search the content of the indexed codebase via
MCP. The design adds a `cxpak_search` tool that searches indexed file content with
Rust `regex`-crate patterns, returning matching lines with configurable context lines,
a result limit, and a `focus` prefix. To support it, the `regex` crate is promoted from
a transitive dependency (pulled in via the `ignore` crate) to a direct dependency.
Search is case-sensitive by default with `(?i)` available; binary and empty-content
files are skipped.

## Options considered

- **Option A — regex content search over `index.files`, line by line:** Compile a
  `regex::Regex`, iterate the in-memory index honoring `focus`, capture each matching
  line plus a context window, and cap results at `limit`. Pros: reuses the in-memory
  index, adds no new index structures, and `regex` is already in the dependency tree.
  Cons: each query is a linear scan; there is no precomputed search index.

- **Option B — build an inverted/trigram search index:** A reasonable alternative
  would have been to precompute a dedicated search structure for fast lookups. Someone
  could prefer it for faster repeated queries. Rejected as added build cost and memory
  that is over-engineered for this use case.

## Decision

Implement `cxpak_search` as a regex line-search over `index.files` content, with
`limit`, `focus`, and `context_lines` params and a `truncated` flag, defaulting to
case-sensitive matching. Promote the `regex` crate to a direct dependency. Shipped;
handler at `src/commands/serve.rs:2949`.

## Consequences

### Positive
- Content search is available via MCP with no new index structures.
- No new compile cost, since `regex` was already a transitive dependency.

### Negative
- Each search is a full linear scan of file contents.

### Neutral
- The case-sensitive default matches developer expectations; `(?i)` is provided as an
  escape hatch.

## Revisit if
- Search latency on large repositories becomes a problem.
- Users need fuzzy or semantic search instead of regex.
