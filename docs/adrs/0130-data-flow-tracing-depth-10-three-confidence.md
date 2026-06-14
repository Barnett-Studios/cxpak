---
id: '0130'
title: 'Structural data-flow tracing over the call graph: max depth 10, three-level confidence, four boundary flags'
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.5.0 adding source-to-sink data-flow tracing (cxpak_data_flow)
loop: implementation
---

# ADR-0130: Structural data-flow tracing over the call graph: max depth 10, three-level confidence, four boundary flags

## Context
Introduced in v1.5.0. The tool needed to trace how a value flows from source to sink across calls. This is fundamentally a structural (static) walk over the v1.3.0 call graph, not runtime dispatch. Cycles and dynamic dispatch must not cause infinite loops or false certainty, and the call graph might be absent in older indices. Note: the v1.5.0 implementation plan originally specified `Speculative`-on-`Approximate`; this was changed during implementation so an `Approximate` call edge maps to `FlowConfidence::Approximate`, with `Speculative` reserved for unresolved-parameter paths. This ADR records the shipped behavior.

## Options considered
- **Option A — BFS over call graph, depth clamped to 10, Exact/Approximate/Speculative confidence, boundary flags, graceful no-call-graph degradation:** Classify nodes Source/Transform/Sink/Passthrough by callee-name keywords; `Approximate` when a call edge is `Approximate`; `Speculative` when a parameter can't be name/position matched (unresolved); flag module/language/security boundary crossings; return `truncated = true` when the depth limit prunes paths. Pros: bounded and cycle-safe, honest confidence about dynamic dispatch, reuses the existing call graph and v1.4.0 security surface for the security-boundary flag. Cons: structural only — misses runtime dispatch through closures/trait objects (tagged Speculative). Someone could prefer it because it terminates reliably while remaining honest about path certainty.
- **Option B — Unbounded flow tracing:** A reasonable alternative would have been to follow all paths to completion. Pros: complete paths. Cons: non-termination on cycles and explosive path counts. Someone could prefer it for completeness in small, acyclic graphs.

## Decision
Add `src/intelligence/data_flow.rs` with `trace_data_flow(symbol, sink, depth, index)` doing a BFS over `CodebaseIndex.call_graph`, depth clamped to a maximum of 10. Nodes are classified Source/Transform/Sink/Passthrough by callee-name keyword heuristics. Each path carries a `FlowConfidence`: `Approximate` is set when any hop's `CallEdge` is `CallConfidence::Approximate`, and `Speculative` is reserved for paths where a parameter cannot be matched by name/position (an unresolved parameter); `Exact` when every hop is a directly resolved call with a forwarded parameter. Paths also flag `crosses_module_boundary`, `crosses_language_boundary`, and `touches_security_boundary` (computed once per trace from the v1.4.0 security surface). `truncated = true` when the depth limit prunes any path. When `call_graph` is absent, return empty paths rather than panic.

## Consequences
### Positive
- Bounded, cycle-safe traversal with explicit truncation reporting.
- Three-level confidence keeps the LLM from over-trusting dynamic-dispatch paths.
- Reuses the call graph and security surface; the security boundary is computed once per trace, not N×M.
### Negative
- Structural only; runtime dispatch through closures/trait objects is invisible (tagged Speculative when a parameter is unresolved).
- Node classification is keyword-heuristic.
### Neutral
- Exposed via `cxpak_data_flow` (MCP) and `/data_flow` (HTTP) with documented limitations prepended to the response.

## Revisit if
- The depth-10 ceiling proves too shallow for real call chains.
- Keyword-based node classification needs language-specific refinement.
