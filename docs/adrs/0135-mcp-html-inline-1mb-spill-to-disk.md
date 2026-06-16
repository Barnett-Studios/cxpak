---
id: '0135'
title: Spill MCP visual HTML responses over 1MB to disk, return file path
status: ACCEPTED
date: 2026-04-01
triggered_by: cxpak_visual MCP tool response sizing (Task 15)
loop: implementation
---

# ADR-0135: Spill MCP visual HTML responses over 1MB to disk, return file path

## Context

Shipped in v2.0.0. Returning large self-contained HTML inline through the MCP tool channel can blow context/transport limits. The `cxpak_visual` tool (Task 15) needed a threshold strategy to keep payloads bounded.

## Options considered

- **Option A — inline under threshold, spill to disk above:** When `format == "html"` and the rendered HTML exceeds a 1MB limit (`MCP_INLINE_LIMIT = 1048576`), write the file to `.cxpak/visual/` and return its path instead of inline content. Pros: bounds MCP payload size; keeps small outputs inline for convenience; threshold value matches the documented 1MB design intent. Cons: callers must read a file for large outputs; the 1MB threshold is a heuristic. The design doc proposed making the limit configurable via a `.cxpak.json` key `mcp_inline_limit_bytes`, but that did not ship — the shipped code uses a hardcoded constant. This option is what shipped (minus the unimplemented configurability).
- **Option B — always return inline content:** A reasonable alternative would have been to embed HTML in the tool result regardless of size. It keeps everything in a single response with no filesystem involvement, but risks exceeding MCP/context transport limits on large dashboards. Reconstructed alternative; not formally evaluated.

## Decision

`handle_cxpak_visual` returns HTML inline only when under a hardcoded threshold (`const MCP_INLINE_LIMIT = 1_048_576` / 1MB in `serve.rs`); larger HTML is written to `.cxpak/visual/` and the file path is returned. NOTE: the cited design doc proposed a configurable `.cxpak.json` key `mcp_inline_limit_bytes`, but the shipped code uses a hardcoded constant — configurability was never implemented.

## Consequences

### Positive
- MCP payloads stay within transport limits.
- Threshold value matches the documented 1MB design intent.

### Negative
- Callers must handle a path-vs-inline branch.
- The 1MB threshold is a fixed heuristic.
- Documented configurability (`.cxpak.json` `mcp_inline_limit_bytes`) was specified in the design doc but never shipped; the threshold is a compile-time constant.

### Neutral
- The spill check is gated strictly on `format == "html"`.

## Revisit if
- MCP transport limits change.
- Inline vs path handling confuses tool consumers.
