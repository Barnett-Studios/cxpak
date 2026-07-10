---
id: '0174'
title: Column-level lineage and column-granular blast radius
status: ACCEPTED
date: 2026-06-24
triggered_by: cxpak 3.0.0 Phase A, Task A2
loop: implementation
---

# ADR-0174: Column-level lineage and column-granular blast radius

## Context

Through v2.x the dependency graph resolved schema relationships at *table*
granularity: an embedded `SELECT email FROM users` produced a single
`EmbeddedSql` edge from the source file to the `users` table-definition file,
and `compute_blast_radius` could answer "what depends on the `users` table?"
but not "what depends on `users.email`?". For the Phase A goal — making "alter
`users.email`" actionable — table granularity is too coarse: it flags every
file that touches *any* column of `users`, drowning the few files that touch
`email` in noise and producing false positives for a column-scoped change.

The requirement (Task A2) is precision: changing one column must resolve to the
specific queries / ORM models / endpoints / tests that reference THAT column,
and a different column's blast must EXCLUDE the email-only files. That demands
columns be first-class, addressable nodes in the dependency graph, plus an
`endpoint → query → table → column` edge chain. The decisions below are human
calls because they trade attribution *precision* against parser *complexity*
and the risk of silently mis-attributing references — a correctness property
the code cannot choose for us.

## Options considered

### 1. Column node identity

- **Option A — namespaced string id `col:{table}.{column}` (lowercased) [chosen]:**
  A synthetic graph node keyed exactly like the existing table map (extraction
  lowercases identifiers) with a `col:` prefix that no real file path can carry.
  Pros: zero new graph machinery — column nodes ride the existing
  `HashMap<String, BTreeSet<TypedEdge>>` and BFS unchanged; collision-safe by
  construction; deterministic (pure function of inputs). Cons: a string node is
  weakly typed — a typo'd prefix would silently create a "file" node.
- **Option B — a distinct `ColumnNode` struct / separate node registry:** a typed
  node kind alongside file nodes. Pros: type-safe, self-documenting. Cons:
  forks `DependencyGraph` into two node kinds, touching PageRank, blast BFS,
  serialization, and the visual layer — a large blast radius for a Phase A task,
  and a real risk of perturbing the determinism golden. A reasonable person
  optimizing for long-term graph richness could prefer this.
- **Option C — encode columns as edge metadata, not nodes:** keep one edge per
  file→table but tag it with the column set. Pros: no new nodes. Cons: the blast
  BFS is node-seeded; a column seed would have no node to start from, forcing a
  parallel column-indexed traversal — more code than a node, and it cannot
  express the `column → table` anchor cleanly.

### 2. Column-reference extraction

- **Option A — reuse the embedded-SQL regex layer, add SELECT-list / WHERE / SET
  / INSERT column extraction, resolve against the detected tables [chosen]:**
  Pros: builds on the proven `detect_embedded_sql` table detector and stays
  dependency-free and deterministic; no new tree-sitter grammar wiring. Cons:
  regex SQL parsing is heuristic — aliases, subqueries, and CTEs are only
  partially handled, so embedded-SQL column edges are marked `Inferred`.
- **Option B — full tree-sitter-sequel column-resolution pass:** parse the SQL
  AST and resolve column→table bindings precisely. Pros: higher precision on
  complex queries. Cons: embedded SQL lives inside *other* languages' string
  literals (already the reason `detect_embedded_sql` is regex-based); extracting
  and re-parsing those fragments through the SQL grammar is substantial new
  machinery for marginal gain at this scale. Defensible if column precision on
  gnarly queries becomes the bottleneck.

The honest precision policy (the part the code must not get silently wrong):
- **`SELECT *` / `t.*`:** columns are unnameable, so we **fan out to every column
  of each detected table** and mark those edges `Inferred`. Explicit
  over-attribution, never a silent drop.
- **Qualified `t.col`:** attributed to detected table `t` when it owns `col`,
  else to any detected table that owns `col`.
- **Bare `col`:** attributed only when **exactly one** detected table owns it;
  zero owners → unknown column (dropped, never invented); more than one owner →
  ambiguous (dropped rather than mis-attributed).
- **ORM fields:** each non-relation field whose name matches a declared column
  maps to that column (`Extracted` — structurally proven). Relation fields are
  skipped (they reference another model, not a scalar column).

### 3. New `EdgeType::ColumnReference` vs reuse

- **Option A — add `EdgeType::ColumnReference` [chosen]:** a dedicated variant for
  file→column and column→table edges. Pros: blast categorization, labels, and
  the impact weight table can treat column lineage distinctly; existing edge
  semantics are untouched. Cons: every exhaustive `match` on `EdgeType` gains an
  arm (mechanical, compiler-enforced).
- **Option B — reuse `EmbeddedSql` / `OrmModel` for column edges:** Pros: no new
  variant. Cons: a column-target edge and a table-target edge would be
  indistinguishable, defeating the precision goal and breaking the
  `is_schema_edge` / weight semantics that assume a file→file shape.

Because `ColumnReference` confidence is genuinely per-edge (heuristic SQL refs =
`Inferred`; structural ORM-field and `column→table` anchors = `Extracted`),
`build_schema_edges` now returns a 4-tuple carrying explicit `EdgeConfidence`,
and `DependencyGraph::add_edge_with_confidence` stamps it. `default_confidence`
maps `ColumnReference` to the conservative `Inferred`; the structural cases
override it explicitly.

## Decision

Promote columns to addressable graph nodes via `column_node_id(table, column)`
→ `col:{table}.{column}` (lowercased, prefix-namespaced). Add
`EdgeType::ColumnReference`. Extend `build_schema_edges` to emit
`query/ORM → col:table.col` edges plus a once-per-column `col:table.col → table_file`
anchor, with per-edge confidence (heuristic SQL = `Inferred`, structural ORM /
anchor = `Extracted`). Add `compute_column_blast_radius(table, column, …)` that
seeds the existing reverse-edge BFS at the column node — surfacing exactly the
files touching that column. The file/table-level `compute_blast_radius` is
unchanged, and table-level `EmbeddedSql` edges are still emitted alongside the
new column edges (no regression).

## Consequences

### Positive
- "alter `users.email`" resolves to the email-touching queries / models /
  endpoints / tests at column resolution; a `users.name` blast excludes
  email-only files (verified by the contract + negative-column tests).
- Column lineage is deterministic (sorted, deduplicated edges; BTree-backed
  graph) and dependency-free.
- Existing file/table blast and table-level edges are preserved; the
  determinism golden (`spa_determinism`) is byte-identical (the fixture has no
  schema, so no column edges are produced).

### Negative
- Embedded-SQL column attribution is heuristic: aliases, subqueries, CTEs, and
  ambiguous bare columns across multi-table joins are conservatively dropped
  rather than guessed — so recall on complex queries is below 100%. These edges
  carry `Inferred` confidence to signal that.
- Every exhaustive `match` on `EdgeType` gains a `ColumnReference` arm.

### Neutral
- `build_schema_edges` now returns 4-tuples; its sole production caller and the
  schema/edge-confidence tests were updated to thread confidence through
  `add_edge_with_confidence`.

## Revisit if

- Column precision on complex SQL (CTEs, subqueries, aliasing) becomes a
  measured bottleneck — promote extraction from regex (Option 2A) to a
  tree-sitter-sequel resolution pass (Option 2B).
- The graph grows a second genuinely-typed node kind for another reason —
  reconsider the string-id column node (Option 1A) in favor of a typed
  `ColumnNode` (Option 1B) to unify node handling.
- A downstream consumer needs to distinguish `SELECT *` fan-out edges from named
  column refs beyond the `Inferred`/`Extracted` confidence split recorded here.
