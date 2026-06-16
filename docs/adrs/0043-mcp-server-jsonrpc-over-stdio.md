---
id: '0043'
title: MCP server mode as JSON-RPC over stdio (cxpak serve --mcp)
status: ACCEPTED
date: 2026-03-12
triggered_by: Need zero-config access from any MCP client (Claude Code, Cursor, etc.)
loop: planning
---

# ADR-0043: MCP server mode as JSON-RPC over stdio (cxpak serve --mcp)

## Context

Released in v0.8.0. Beyond the HTTP API, cxpak should be reachable by any MCP-compatible tool. The MCP standard transport is JSON-RPC over stdio, so the same overview/trace/diff queries the CLI already serves can be re-exposed as MCP tools without inventing a new protocol or managing a port.

Note on a divergence captured here for accuracy: the v0.8.0 design doc specified a 30k token default for `cxpak_diff` (chosen for PR readability), but the shipped MCP tool uses the same 50k default as `cxpak_overview` and `cxpak_trace` (`src/commands/serve.rs`).

## Options considered

- **Option A — JSON-RPC over stdio, MCP tools `cxpak_overview`/`cxpak_trace`/`cxpak_diff`:** A `--mcp` flag on `serve` that, instead of starting HTTP, reads JSON-RPC from stdin and writes responses to stdout, implementing `initialize` / `tools/list` / `tools/call` and exposing three tools with JSON input schemas (tokens default 50k, trace requiring `target`). Pros: standard MCP transport, zero-config for any MCP client, reuses the query engine, no port to manage. Cons: stdio cannot multiplex with HTTP in the same process; protocol bookkeeping for initialize/list/call. Someone could prefer it for exactly the reason chosen — it is the universally supported MCP transport.
- **Option B — MCP over HTTP/SSE transport:** A reasonable alternative would have been to expose MCP via an HTTP-based transport instead of stdio. Pros: could coexist with the HTTP API on a port. Cons: less universally supported than stdio at the time and more setup. Someone could prefer it to run a single long-lived endpoint serving both the raw HTTP API and MCP. Not formally evaluated in the design.

## Decision

Add `cxpak serve --mcp`, which runs an MCP server speaking JSON-RPC over stdio (`initialize` / `tools/list` / `tools/call`), exposing `cxpak_overview`, `cxpak_trace`, and `cxpak_diff` with JSON input schemas. The `tokens` parameter defaults to 50k across all three tools (the design doc proposed 30k for `cxpak_diff`, but the shipped tool uses 50k); `cxpak_trace` requires a `target`. Clients like Claude Code configure it via `mcpServers` with command `cxpak` and args `[serve, --mcp, .]`.

## Consequences

### Positive
- Zero-config integration for any MCP client over the standard stdio transport.
- Reuses the hot-index query engine; the MCP tool surface grew to 26 tools by v2.0.0.

### Negative
- stdio mode cannot also serve HTTP from the same process.

### Neutral
- `--mcp` replaces HTTP for that process (stdio transport).
- Tool input schemas carry per-tool token defaults; the design's intended 30k `cxpak_diff` default shipped as 50k, matching overview/trace.

## Revisit if
- MCP adds or standardizes a streaming HTTP transport worth supporting.
- The tool surface grows enough to need schema generation.
