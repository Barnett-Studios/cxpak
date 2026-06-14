---
id: '0062'
title: 'Two-tool MCP surface: cxpak_context_for_task (rank) + cxpak_pack_context (bundle)'
status: ACCEPTED
date: 2026-03-17
triggered_by: Replace Claude Code's discover-then-act (5-10 Glob/Grep/Read calls) with precomputed context
loop: planning
---

# ADR-0062: Two-tool MCP surface: cxpak_context_for_task (rank) + cxpak_pack_context (bundle)

## Context

The v0.9.0 goal is to replace Claude Code's "discover-then-act" pattern (5-10
Glob/Grep/Read calls) with precomputed, dependency-aware context delivered in 1-2
tool calls. The open question was whether to expose this as one monolithic tool or
to split ranking from bundling. This raised the v0.9.0 MCP tool inventory from 4
tools to 6.

## Options considered

- **Option A — two separate tools with an optional handshake:** `cxpak_context_for_task`
  returns ranked candidates (paths, scores, per-signal breakdown, dependencies, token
  counts, and a hint); `cxpak_pack_context` bundles selected files within a token
  budget. The two-phase handshake — Claude acting as an optional re-ranker between the
  phases — is optional, and each tool is independently useful (supporting cold-start,
  warm, and standalone usage patterns). Pros: lets Claude re-rank candidates before
  paying for full file content; supports three distinct usage patterns; each tool is
  useful alone. Cons: the full flow needs two round-trips, and the client must wire
  the handshake.

- **Option B — single tool returning packed context directly:** A reasonable
  alternative would have been one tool that takes a task and returns the packed bundle
  in a single call. Someone could prefer it for one round-trip and a simpler client.
  Rejected because it offers no LLM-in-the-loop re-ranking and cannot reuse ranking
  without packing, nor pack without re-ranking.

## Decision

Expose two MCP tools. `cxpak_context_for_task` scores and ranks files and returns
candidates with a per-signal breakdown, dependencies, token counts, and a hint to
review and call `cxpak_pack_context`. `cxpak_pack_context` packs selected files within
a token budget with an optional `include_dependencies` flag. The handshake between them
is optional, supporting cold-start, warm, and standalone usage patterns.

## Consequences

### Positive
- Claude can re-rank candidates before paying for full file content.
- Three distinct usage patterns (cold-start, warm, standalone) supported from one tool pair.
- Each tool is independently useful.

### Negative
- The full ranking-then-packing flow needs two MCP round-trips.

### Neutral
- The total MCP tool count goes from 4 to 6 in v0.9.0.

## Revisit if
- Telemetry shows the handshake is never used (always direct pack, or always standalone).
- A single combined tool would cut latency materially.
