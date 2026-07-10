---
id: '0177'
title: Cypher + GraphML dependency-graph export
status: ACCEPTED
date: 2026-06-30
triggered_by: cxpak 3.0.0 Phase B (Task B2) — make the graph queryable (B1) and exportable (B2)
loop: implementation
---

# ADR-0177: Cypher + GraphML dependency-graph export

## Context

Phase B makes the dependency graph not just queryable (B1) but portable: users want
to load cxpak's typed dependency graph into external graph tooling (Neo4j, Gephi,
yEd, NetworkX) for ad-hoc exploration that cxpak itself does not provide. Two
interchange formats cover that surface: **Cypher** (Neo4j's import language) and
**GraphML** (the de-facto XML graph interchange format consumed by Gephi/yEd/
NetworkX).

These join the existing `visual --format` exporters (`html|mermaid|svg|png|c4|json`).
But unlike those — which serialize the laid-out `ComputedLayout` of *module* nodes
and positional `EdgeVisualType` edges — a graph-interchange export must carry the
**honest typed edges**: `EdgeType` and the per-edge `EdgeConfidence`
(`Extracted` vs `Inferred`, ADR-0097 / ADR-0175). The positional layout drops that
metadata. So the question is *what to serialize and how to encode it*, and it is a
human decision because it trades fidelity, idempotency, escaping-safety, and
testability against each other — none of which the code can resolve on its own.

Node ids are partly attacker-influenced: a repository can contain a file whose path
holds a quote, backslash, newline, or angle bracket. The string-literal escaping is
therefore a correctness **and** injection boundary, not a cosmetic concern.

## Options considered

- **Option A — serialize `ComputedLayout` (reuse the visual exporters' input):**
  symmetric with the other `--format` arms; one code path. But `LayoutEdge` carries
  only `EdgeVisualType` (no confidence), the layout is module-granular and
  positionally biased, and it would silently launder away the `Extracted/Inferred`
  honesty the rest of Phase A/B is built on. A stakeholder optimizing for code
  symmetry could prefer it; rejected because it produces a *dishonest* graph export.
- **Option B — serialize the `DependencyGraph` directly (chosen):** iterate the
  existing `index.graph` (`BTreeMap`/`BTreeSet` adjacency of `TypedEdge`), emitting
  every node and every typed edge with its `EdgeType::label()` + `EdgeConfidence`.
  Reuses the graph that already exists (no re-derivation) and is fully deterministic
  by construction. Cost: a second input shape in `export.rs` alongside the
  `ComputedLayout` exporters.
- **Option C — add a graph-interchange crate (e.g. a GraphML/Cypher writer dep):**
  off-the-shelf correctness. Rejected: violates the no-new-deps constraint, GraphML
  is plain XML already served by the module's existing `xml_escape`, and Cypher is
  plain string generation — a crate would be more surface than the ~40 lines it
  replaces.

### Cypher sub-decisions

- **MERGE vs CREATE:** `MERGE` for both nodes and relationships. `MERGE` makes the
  script idempotent — re-running it against an existing Neo4j graph does not
  duplicate nodes (identity is the `id` property) — which matches how operators
  actually re-import evolving snapshots. `CREATE` would have been one token shorter
  but duplicates on re-run.
- **Relationship type:** a single fixed `:DEPENDS_ON` relationship type carrying the
  real edge label in a `type` property, rather than promoting the label to the Cypher
  relationship-type identifier. The honest label can be `cross_language:HttpCall`,
  which is not a valid bare relationship-type identifier; a fixed type sidesteps a
  second escaping problem while keeping the label visible in `type` /
  `confidence` / `inferred` properties.
- **Node labeling:** `:File` for indexed source paths, `:Column` for the synthetic
  `col:{table}.{column}` lineage nodes (Task A2 / ADR-0174). The `col:` prefix can
  never collide with a real file path, so prefix-matching the id is sufficient.
- **Escaping:** single-quoted Cypher literals; `\` → `\\`, `'` → `\'`, and the
  control characters `\n`/`\r`/`\t` are escaped so no statement can be split across
  lines or broken out of its literal.

### GraphML key schema

Four `<key>` declarations: `d_kind` (node, string), `d_type` (edge, string),
`d_confidence` (edge, string), `d_inferred` (edge, boolean). Every id/value is
XML-escaped through the module's existing `xml_escape` helper (shared with the SVG
exporter) — no new dependency. Edges receive sequential `e{n}` ids in canonical
order.

## Decision

Add `to_cypher(&DependencyGraph, repo_name)` and `to_graphml(&DependencyGraph,
repo_name)` to `src/visual/export.rs` (Option B), serializing the existing
`index.graph` with honest `EdgeType` + `EdgeConfidence` metadata. Cypher uses
idempotent `MERGE`, a fixed `:DEPENDS_ON` relationship type with `type`/`confidence`/
`inferred` properties, kind-labeled nodes, and control-character-safe single-quote
escaping. GraphML emits well-formed XML with a four-`<key>` schema reusing
`xml_escape`. Both are wired into the `visual --format` CLI enum and the
`cxpak_visual` MCP `format` param, under the existing `visual` feature flag (no new
flag, no new dependency).

### No-live-Neo4j testing

The plan's "imports into a Neo4j fixture" is satisfied without a live database or
network: the tests assert the export *would* import — i.e. it is **syntactically
valid and deterministic** — rather than standing up Neo4j. Concretely: every
relationship endpoint id is asserted to be a declared (`MERGE`'d) node; no statement
is split across lines; adversarial quote/backslash/newline paths are escaped; the
output is byte-identical across two runs. GraphML is asserted structurally
well-formed (balanced `<node>`/`<edge>`/`<graph>`/`<graphml>` tags, one edge per
graph edge) and XML-escaped. A live-DB integration test was rejected as
out-of-scope: it would add a network/daemon dependency to a deterministic,
offline-by-design tool for no additional correctness signal over structural +
escaping + determinism assertions.

## Consequences

### Positive
- Honest, portable graph export: external tooling sees the same `Extracted/Inferred`
  edge confidence the rest of cxpak surfaces.
- Deterministic by construction (`BTreeMap`/`BTreeSet` adjacency + sorted node set);
  does not perturb the SPA determinism golden.
- Zero new dependencies; reuses `index.graph` and `xml_escape`.
- Idempotent Cypher re-import via `MERGE`.

### Negative
- `export.rs` now serves two input shapes (`ComputedLayout` for the visual formats,
  `DependencyGraph` for the interchange formats); a reader must know which formats
  consume which.
- Cypher relationship type is fixed (`:DEPENDS_ON`); consumers wanting type-as-label
  must read the `type` property rather than the relationship type.

### Neutral
- Synthetic `col:` nodes appear as first-class `:Column` / `kind=Column` nodes in
  the export, mirroring their status in the in-memory graph.
- No live Neo4j/GraphML validation in CI; validity is enforced by structural +
  escaping + determinism unit assertions.

## Revisit if

- Consumers need the edge label as the native Cypher relationship type (would force
  per-label relationship-type sanitization/escaping, superseding the fixed
  `:DEPENDS_ON` decision).
- A streaming/very-large-graph export is required (the current functions build the
  whole string in memory; fine for the file-count scale cxpak indexes, but a
  multi-million-node graph would want a writer that streams).
- A real GraphML/Cypher consumer reports a validity gap that the structural unit
  assertions did not catch — at which point a parser-backed conformance test (still
  no live DB) would be justified.
