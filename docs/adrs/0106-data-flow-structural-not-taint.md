---
id: '0106'
title: Data flow is structural call-graph tracing with confidence tagging, explicitly not taint analysis
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.5.0 'Deep Flow' data flow analysis
loop: planning
---

# ADR-0106: Data flow is structural call-graph tracing with confidence tagging, explicitly not taint analysis

## Context
The v1.5.0 ("Deep Flow") work traces how values move through a system. There are two broad ways to do this: full taint / type-inference analysis (precise, very expensive, language-specific) or structural tracing over the existing call graph and symbol extraction (approximate, cheap, reuses infrastructure cxpak already has). The design picks the structural route and is explicit, in tool output, that it is not taint analysis.

## Options considered
- **Option A — Structural tracing over the call graph with `FlowConfidence` tags:** follow named values through function parameters, return values, and assignments via the call graph; classify each node (`Source` / `Transform` / `Sink` / `Passthrough`); tag each path `Exact` / `Approximate` / `Speculative`; stop at sinks or a max depth (default 10); tag closure, higher-order-function, and dynamic-dispatch hops as `Speculative`; and display limitations prominently in tool output. Pros: reuses the call graph and symbol extraction, so it is cheap; honest about its limits via confidence tags and prominent limitation docs; works approximately across all language tiers. Cons: not runtime-accurate; parameter matching is heuristic (by name/position); chains break through collections and dynamic dispatch. (Grounded — this is the shipped design.)
- **Option B — Full taint / type-inference data flow:** A reasonable alternative would have been precise dataflow with type inference and dynamic-dispatch resolution, which would be runtime-accurate and catch flows through collections and dispatch. Someone could prefer it for security use cases that demand precision. It was not formally evaluated as an alternative — the source only names taint analysis and type inference as things this feature is explicitly *not* — but it is rejected on cost grounds: it is extremely expensive, per-language, and far beyond cxpak's structural infrastructure. (Reconstructed — appears in the source only as a negative contrast, not as a weighed alternative.)

## Decision
Implement data flow as structural tracing: follow named values through function parameters, return values, and assignments using the call graph plus symbol extraction. This is explicitly **not** full taint analysis.

Start from a symbol, identify external-input parameters (route-handler first parameters; parameters named `input` / `request` / `body` / `data` / `payload` and similar), follow them through the call graph classifying each node as `Source` / `Transform` / `Sink` / `Passthrough`, and stop at sinks or the max depth (default 10). Each path carries a `FlowConfidence` (`Exact` / `Approximate` / `Speculative`); closure, HOF, and dynamic-dispatch hops are tagged `Speculative`. Limitations are displayed prominently in tool output, not buried.

## Consequences
### Positive
- Reuses the existing call-graph and symbol infrastructure cheaply.
- Confidence tags and prominent limitation docs set honest expectations.
- Provides approximate coverage across all language tiers.

### Negative
- Not runtime-accurate; parameter matching is heuristic.
- Chains break through collections and dynamic dispatch.

### Neutral
- The same structural engine is wired uniformly into the MCP tool, the `/v1/data_flow` HTTP route, and the `cxpak/dataFlow` LSP method.

## Revisit if
- Users need precise taint tracking for security use cases.
- Heuristic parameter matching produces too many wrong paths in practice.
