---
id: '0182'
title: Consolidate 26 MCP tools into ≤8 intent-parameterized tools (BREAKING)
status: ACCEPTED
date: 2026-07-03
triggered_by: cxpak 3.0.0 Task C3 (final Phase-C task; the release's only breaking change)
loop: implementation
---

# ADR-0182: Consolidate 26 MCP tools into ≤8 intent-parameterized tools (BREAKING)

## Context

Through v2.x the MCP server (`src/commands/serve.rs`) advertised **26 hand-rolled
top-level tools** (`cxpak_auto_context`, `cxpak_overview`, `cxpak_health`, …),
each with its own `inputSchema` in `list_tools` and its own arm in the
`call_tool` dispatch. Two problems:

1. **Context cost.** An MCP client loads every tool's name + description +
   schema into the model's context on connect. 26 tools is a large, growing tax
   that competes with the very token budget cxpak exists to protect.
2. **Drift.** Task 0.6 built a capability **catalog + MCP adapter**
   (`src/capability/{mod,adapter}.rs`) that projects capabilities into ≤8
   *intent-tools*, and locked a CI budget gate (`tests/mcp_tool_budget.rs`) — but
   the *live* server kept its 26 hand-rolled tools. A1/A2/A3, B1, C1, C2 landed
   their capabilities in the catalog while the live surface stayed frozen. The
   catalog was a promise the shipped product did not yet keep.

C3 makes the live MCP surface **be** the catalog's ≤8 projection. This is a
human decision because it is a **breaking public API change**: the choice of
whether (and how) to break MCP clients — versus carrying 26 tools indefinitely —
is a product trade-off, not something the code can resolve on its own.

The 3.0.0 plan referenced a "26→8 mapping appendix" that never existed, and a
stale "ADR-0173" number. Both are resolved here: this ADR **is** the mapping's
rationale (the enumerated table lives in `docs/MIGRATION-3.0.md`), and the
correct number is **0182** (0181 = C2's predecessor line; next-available).

## Options considered

- **Option A — Intent-parameterized tools (chosen).** Five `cxpak_<intent>`
  tools (Context / Graph / Data / Review / Insight); a capability is selected by
  a required `op` argument. Tool count is a structural constant (`Intent::ALL`),
  so it can never regress past the ≤8 gate. Cost: clients must learn the `op`
  discriminator, and one tool's `inputSchema` is a union over its ops' params.
- **Option B — Keep 26 tools, cap the count by review.** No client break, but
  the context tax stays and the count keeps drifting up; the 0.6 catalog work is
  wasted. A maintainer optimizing purely for "no breaking change" could prefer
  this.
- **Option C — Fewer than five, deeply nested ops.** e.g. one `cxpak` tool with
  a two-level `op`. Minimizes tool count hardest, but collapses all analysis
  into one opaque schema — worse discoverability, and a single `additionalProperties`
  grab-bag. Someone optimizing purely for tool count could prefer it; we judged
  five intents the better legibility/□budget balance.

## Decision

Adopt **Option A**. `list_tools` now emits
`crate::capability::adapter::mcp_tool_schemas()` — the ≤8 intent-tools projected
from `catalog()`, in deterministic catalog order, each advertising
`annotations.readOnlyHint: true` and a required `op` enum. `call_tool` routes
`(intent-tool, op)` through `dispatch_capability_op`, whose arms are the former
26 tool bodies **re-keyed to their capability `op` id** — same underlying
intelligence functions, a surface reshape not a logic change.

**No dropped functionality.** Every one of the 26 former tools maps to exactly
one `(intent-tool, op)` pair (enumerated in `docs/MIGRATION-3.0.md` and asserted
live by `tests/mcp_live_surface.rs::every_legacy_tool_reachable_via_intent_op`).
Where a legacy tool had no catalog capability, the capability was **added** to
`catalog()` (21 new capabilities) rather than dropped. `graph`, `retrieval` and
`data` are newly MCP-surfaced cores (`graph_op`/`retrieval_op` sub-selectors so
they don't collide with the intent `op`; `data` returns the indexed
`SchemaIndex`).

**Deprecated aliases.** The 26 old tool *names* remain accepted by `call_tool`
as undiscoverable aliases (absent from `tools/list`) for one release, then
removed. This softens the break without re-inflating the advertised surface.

**Three deferred items folded in:**

- **A3 (ADR-0175) edge confidence on the graph surface** — the `graph` op's
  `neighbors`/`path`/`subgraph` output carries per-edge `edge_type` +
  `confidence` (`inferred`); pinned by
  `graph_op_surfaces_edge_type_and_confidence_a3`.
- **B1 M2 graph/data catalog bits** — `data`'s all-false surface set is flipped
  to `mcp: true` with a real `SchemaIndex`-derived core; `surface_conformance`
  gains real-core round-trip tests for both `graph` and `data`
  (`migrated_{graph,data}_op_real_core_round_trips_all_surfaces`).
- **C1 readOnly wiring** — the `read_only` capability field (ADR-0180) now drives
  each intent-tool's `annotations.readOnlyHint`, advertised in `tools/list`.

## Consequences

### Positive
- MCP context cost drops from 26 tool schemas to 5.
- The ≤8 ceiling is now a structural invariant of the *live* surface, not just
  the isolated adapter; `mcp_live_surface.rs` asserts `live == adapter`.
- The catalog is finally the single source of truth for the MCP surface — new
  capabilities ride as an `op`, never a new tool.
- Read-only intent is machine-advertised to clients.

### Negative
- **Breaking change** for MCP clients: calls must add `op`; discovery returns
  five tools, not 26. Mitigated by the migration guide + transitional aliases.
- One intent-tool's `inputSchema` is a `op`-enum + `additionalProperties: true`
  union rather than a tight per-tool schema; per-op params are documented in the
  migration guide, not the schema.

### Neutral
- The 26 dispatch bodies are unchanged in behavior — only their match key moved
  from `cxpak_<tool>` to the capability `op` id.
- CLI, HTTP `/v1/*`, and LSP `cxpak/*` surfaces are untouched (they already
  projected the same cores); their tests stay green.
- The determinism golden fixture (SPA output) is unaffected — the MCP surface is
  not part of it, and no rendered `auto_context` path changed.

## Revisit if

- The MCP spec adds first-class tool namespacing/grouping that makes five flat
  tools redundant.
- A future capability is genuinely *not* read-only (would flip an intent-tool's
  `readOnlyHint` automatically via the `read_only` AND in `mcp_tools`).
- Telemetry shows clients cannot resolve the `op` discriminator and per-op
  schemas become necessary (would argue for splitting an intent-tool or emitting
  per-op `oneOf` schemas).
- The transitional legacy-name aliases are removed (a follow-up breaking change);
  revisit the deprecation window then.
