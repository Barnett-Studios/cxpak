---
id: '0202'
title: Graph query — node-enumeration op, subgraph seed validation, documented id format
status: ACCEPTED
date: 2026-07-16
triggered_by: issue #20 (no node-id enumeration op; subgraph echoes non-existent seeds)
loop: planning
---

# ADR-0202: Graph query — node enumeration, seed validation, id-format docs

## Context

The deterministic graph-query surface (`intelligence::graph_query`, ADR-0176;
projected to CLI `graph`, HTTP `/v1/graph`, LSP `cxpak/graph`, MCP `cxpak_graph`)
cannot be bootstrapped by a fresh consumer (issue #20, reproduced in REPRO.md):

1. **No enumerate op.** `execute` (graph_query.rs:398) offers only
   node|neighbors|path|subgraph; `node`/`neighbors` require an `--id`, `path`
   requires `--from/--to`, and there is no op that lists valid ids. The id format
   (repo-relative file path) is undocumented — `--help` says only "Node id".
2. **`subgraph` doesn't validate seeds.** `subgraph` (graph_query.rs:323) inserts
   every seed into the BFS frontier at distance 0 without `graph.contains_node`
   (lines 331-333), so a non-existent seed is **echoed back as a node** with no
   error — inconsistent with `node` (line 189, which reports `exists:false`), and
   the one free-form-id op cannot be used to confirm ids because it accepts
   garbage silently.

Human decision: it adds a capability to a semver-relevant multi-surface core and
picks a seed-validation contract (drop vs. error vs. annotate) that machine
clients will code against.

## Options considered

### Enumerate op
- **Option A — `subgraph` with no seeds dumps the whole graph:** no new op, but
  overloads `subgraph`'s meaning and still returns edges/BFS structure when a
  caller just wants a flat id list; awkward for "what ids exist?".
- **Option B — new `nodes` op (chosen):** `pub fn nodes(graph) -> NodeList` — all
  node ids, sorted (BTree order → deterministic), each with in/out degree. A
  `"nodes"` arm in `execute`, projected to every surface (CLI `graph nodes`,
  `/v1/graph`, `cxpak/graph`, MCP `graph_op:"nodes"`). One capability, four
  projections, no new MCP tool (≤8 holds). Clear, discoverable, matches `node`'s
  vocabulary. **No prefix/`focus` filter in 3.1.1** — the issue asked only for
  enumeration; a filter would need a new CLI `--prefix` arg + a capability param
  entry + a filtered cross-surface test to be reachable and advertised, none of
  which the bare enumerate op requires (YAGNI; add later if a large-monorepo
  consumer needs it, alongside paging).

### Seed validation for `subgraph`
- **Option A — error on any unknown seed:** strict; but a mixed real+bogus seed
  set then yields nothing, and a client probing ids gets an error instead of data.
- **Option B — silently drop unknown seeds:** clean output, but a caller can't
  tell a typo from an empty region — the same silent-failure class the issue is
  about, just inverted.
- **Option C — partition: real seeds drive the BFS, unknown seeds returned in a
  new `unknown_seeds:[id]` field, never emitted as `nodes` (chosen):** consistent
  with `node --id` (`exists:false`), non-breaking (adds a field; real-seed
  `nodes`/`edges` semantics unchanged), and still usable to probe ids (submit a
  guess, read whether it landed in `unknown_seeds`). A strict client can treat a
  non-empty `unknown_seeds` as an error; a lenient one ignores it. The new field is
  `#[serde(default)]` so JSON produced by an older cxpak still deserializes. Note
  the honest meaning: "unknown" = **not an edge-participating node** — a real file
  with zero resolved edges (the ADR-0203 `main.rs` class) also lands here, exactly
  as `node --id` reports `exists:false` for it; this is documented, not a
  contradiction.

### Id-format documentation
- Update `graph --help` for `--id`/`--seeds`/`--from`/`--to` to "repo-relative
  file path (enumerate with `graph nodes`)"; mirror in the MCP/HTTP schema
  descriptions and the CLAUDE.md command list (docs-with-code, same commit).

## Decision

`nodes` enumerate op (Option B) + `subgraph` seed partition into `unknown_seeds`
(Option C) + documented id format across `--help`, schema, and CLAUDE.md. All in
the single graph core with the four projections; total order path-asc throughout.
The `use <pkg>::` self-package resolution gap (why cxpak's own `main.rs` reports
`exists:false`) is out of scope here — see ADR-0203.

## Consequences

### Positive
- A fresh consumer can enumerate ids (`graph nodes`) and gets honest feedback on
  unknown seeds — the surface is self-bootstrapping.
- `subgraph` is now consistent with `node` on unknown ids.
- Deterministic and cross-surface single-source by construction (ADR-0153/0176).

### Negative
- `Subgraph` gains a field (`unknown_seeds`) — additive, but every surface's
  golden/round-trip expectations for `subgraph` update in lockstep.
- Slightly more output for `nodes` on large repos; bounded by file count and
  prefix-filterable.

### Neutral
- No new dependency. Graph path only — the visual golden fixture is unaffected.

## Revisit if
- The id scheme ever stops being the repo-relative path (e.g. symbol-level nodes)
  — then `nodes`/`node`/`--help` id docs move together.
- A consumer needs streaming enumeration for very large monorepos — add paging to
  `nodes` rather than dumping.
