---
id: '0129'
title: Cross-language bridge detection across six bridge types, injected post-build as graph edges
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.5.0 detecting cross-language boundaries (HTTP, FFI, gRPC, GraphQL, shared schema, command exec)
loop: implementation
---

# ADR-0129: Cross-language bridge detection across six bridge types, injected post-build as graph edges

## Context
Introduced in v1.5.0. Polyglot systems connect through implicit bridges — HTTP calls, FFI, gRPC, GraphQL, shared database schema, subprocess execution — that the import graph never captures. The design needed to detect these and represent them as first-class graph edges. Cross-language detection requires the fully built index (routes from `api_surface`, schema edges, proto/graphql definitions), so it cannot run during graph construction where that data is not yet available.

## Options considered
- **Option A — Six pattern-matched detectors, run on the fully built index, injected post-build via `add_edge` and stored on `cross_lang_edges`:** HTTP (fetch/axios/reqwest URL matched to a detected route), FFI (extern C/ctypes vs target symbol), gRPC (client call vs proto service), GraphQL (query/mutation vs schema type), SharedSchema (two languages embedding SQL to the same table), CommandExec (subprocess/exec vs known binary). Pros: reuses existing api_surface/schema/proto data, detection sees the complete index, results are both graph edges and a dedicated list. Cons: pattern matching is heuristic and can miss or misattribute bridges. Someone could prefer it because it cleanly reuses already-computed data and surfaces bridges in two complementary forms.
- **Option B — Change `build_dependency_graph` to emit cross-language edges inline:** Detect bridges during graph construction. Pros: a single pass. Cons: bridge detection needs api_surface/schema that are not available yet during graph build; the plan explicitly rejected this. Someone could prefer it to avoid a second post-build pass, if the required inputs were available earlier.

## Decision
Add `src/intelligence/cross_lang.rs` detecting six `BridgeType`s by pattern-matching against existing `api_surface` routes, schema tables, and proto/graphql definitions. Run `detect_cross_lang_edges` on the fully built index (not inside `build_dependency_graph`) and inject results post-build via `graph.add_edge(..., EdgeType::CrossLanguage(bridge_type))`, while also storing them on a new `CodebaseIndex.cross_lang_edges` field. `auto_context` surfaces them as a dedicated `cross_language_edges` section, capped at `min(remaining, 500)` tokens and never truncated, rather than tagging architecture-map edges.

## Consequences
### Positive
- Implicit polyglot connections become first-class, queryable graph edges.
- Detection reuses already-computed routes/schema/proto data.
- A dedicated `cross_language_edges` section gives the LLM a focused bridge list.
### Negative
- Pattern matching is heuristic; FFI/gRPC/GraphQL matching can be brittle.
- Post-build injection means cross-lang edges are absent from any code that reads the graph before the injection step.
### Neutral
- Exposed via the `cxpak_cross_lang` MCP tool and `/cross_lang` HTTP route; consumed by `data_flow`'s `crosses_language_boundary` flag.

## Revisit if
- A seventh bridge mechanism appears (message queues, websockets).
- Heuristic detection accuracy proves insufficient.
