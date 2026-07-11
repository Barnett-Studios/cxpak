---
id: '0176'
title: Deterministic graph-query capability projected to four surfaces from one core
status: ACCEPTED
date: 2026-06-30
triggered_by: cxpak 3.0.0 Task B1 (Phase B start) — make the typed dependency graph queryable
loop: implementation
---

# ADR-0176: Deterministic graph-query capability projected to four surfaces from one core

## Context

Phase A made the dependency graph *richer* (live DB introspection, column-level
lineage, per-edge [`EdgeConfidence`]). It was still not *queryable*: a caller
could get packed code paths (`trace`), the call graph (`call_graph`), or a
drawing (visual), but no surface returned a direct answer to "what are this
node's neighbours?", "is there a path from A to B?", or "give me the subgraph
within N hops". The Task 0.5 `graph` schema and the Task 0.6 catalog entry both
existed but the capability was declared on **zero** surfaces — an anchor with no
retrieval.

Task B1 must expose four query primitives — **node**, **neighbors**, **path**,
**subgraph** — *identically* from MCP, LSP, CLI, and HTTP, fully deterministic
(byte-stable output, with an explicit tiebreak when several shortest paths
exist). Two human decisions are forced: (1) **how** the four surfaces stay
identical without four hand-rolled adapters drifting apart, and (2) **where**
the MCP projection lives given the hard ≤8-MCP-tool budget (the 26→8 legacy
migration is Task C3, explicitly out of scope here).

## Options considered

- **Option A — Four parallel surface implementations:** each surface (MCP
  handler, LSP method, CLI command, HTTP route) computes the query itself
  against `index.graph`. Simple per-surface code, but the "identical across four
  surfaces" property becomes a *runtime coincidence* that drifts the moment one
  surface tweaks sorting, confidence rendering, or the path tiebreak. A
  reasonable person picks this for short-term velocity.
- **Option B — One core + catalog projection (chosen):** put the query engine in
  `intelligence::graph_query` as pure functions over `&DependencyGraph`, exposed
  through a single `execute(graph, op, params)` entry point. Register `graph` in
  the capability catalog with the four surface bits set; every surface is a thin
  shim that calls `execute` and adapts transport. `tests/surface_conformance.rs`
  then proves identity *structurally*, not by luck.
- **Option C — One core, but route the live MCP server through it now:** same
  core, but also migrate `serve.rs`'s legacy 26-tool MCP handler onto the catalog
  adapter as part of B1. Maximally "wired", but it breaks the ≤8 budget framing
  early, perturbs the 26-tool contract test, and front-runs Task C3. A
  stakeholder could prefer it to avoid a temporary asymmetry — but it violates
  the explicit task boundary.

## Decision

Chose **Option B**. The graph-query core is `intelligence::graph_query` with a
single `execute` dispatch; the four primitives are pure and deterministic over
`&DependencyGraph`. The catalog's `graph` capability now declares
`mcp/lsp/cli/http = true` (`visual` stays false — the visual views *draw* the
graph, they do not return the query contract). CLI (`cxpak graph`), HTTP
(`POST /v1/graph`), and LSP (`cxpak/graph`) are live thin shims that call
`execute`. The **MCP** projection is the `cxpak_graph` intent-tool emitted by the
existing catalog adapter (`capability::adapter::mcp_tools`), which keeps the
budget at four catalog tools (≤8); wiring the *live* `serve.rs` MCP server onto
the adapter is deferred to C3 with the rest of the 26→8 consolidation.

Determinism, including the explicit path tiebreak:

- **node / neighbors / subgraph** — output is sorted (`neighbors` by
  `(node, direction, edge_type, confidence)`; `subgraph` nodes sorted, edges
  induced over the included set in `(from, to, edge_type)` order). The graph's
  `BTreeMap`/`BTreeSet` backing (ADR rationale in `determinism_ties`) means no
  `HashMap`/`HashSet` iteration order ever leaks into output. Edge confidence is
  rendered honestly (`extracted`/`inferred` + bool), reusing `EdgeType::label()`
  and `EdgeConfidence::is_inferred()`.
- **path** — the **lexicographically-smallest shortest path** is canonical. We
  compute each node's distance-to-target by a reverse BFS from `to`, then greedily
  walk from `from`, at every step taking the smallest out-neighbour (the out-edge
  `BTreeSet` is sorted by target) whose distance is exactly one less. Choosing the
  smallest next node at each position yields the lex-min node sequence. A diamond
  fixture (`a→b, a→c, b→d, c→d`) proves `path(a, d)` is always `[a, b, d]`, across
  100 repeats and byte-identical serialization.

## Consequences

### Positive
- "Identical across four surfaces" is a structural invariant enforced by
  `surface_conformance` + the single `execute`, not a drift-prone coincidence.
- The ≤8 MCP budget is preserved by construction (`mcp_tool_budget` stays green at
  four catalog tools); C3 can migrate the live MCP server without re-deriving
  graph-query.
- Byte-determinism is testable in isolation (the core is pure), and the path
  tiebreak is provably canonical, not "whatever BFS happened to visit first".

### Negative
- Temporary asymmetry: CLI/HTTP/LSP are wired to their live legacy servers while
  MCP graph-query is only reachable through the catalog adapter until C3. The
  catalog `mcp: true` bit reflects the adapter projection, not the live
  `serve.rs` server.
- A second query path over the graph now exists alongside `trace`/`call_graph`;
  callers must pick the right tool (graph-query answers structural questions,
  trace packs code).

### Neutral
- `graph_query` results are a *query* contract (node/neighbors/path/subgraph),
  distinct from the Task 0.5 `graph` schema (the `DependencyGraph` shape). The
  catalog capability anchors the latter; reconciling the two contracts, if ever
  needed, is future work.

## Revisit if

- Task C3 migrates the live MCP server onto the catalog adapter — at which point
  the CLI/HTTP/LSP-live vs MCP-adapter asymmetry disappears and this ADR's
  "Negative" note should be struck.
- A caller needs path semantics other than directed out-edge shortest path (e.g.
  undirected reachability, or weighted/confidence-aware costs) — the tiebreak and
  BFS direction recorded here would need to be re-decided.
- Output determinism is ever observed to vary across processes for graph-query
  (it must not) — that would contradict the `BTreeMap`/`BTreeSet` + explicit-sort
  rationale recorded here and is a bug, not a regen.
