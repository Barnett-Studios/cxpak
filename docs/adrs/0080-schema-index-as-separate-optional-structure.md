---
id: '0080'
title: Model the data layer as a separate Option<SchemaIndex> on CodebaseIndex, not embedded in Symbol
status: ACCEPTED
date: 2026-03-21
triggered_by: Need a place to store extracted tables, views, functions, ORM models and migrations without polluting the parse-result data model
loop: planning
---

# ADR-0080: Model the data layer as a separate Option<SchemaIndex> on CodebaseIndex, not embedded in Symbol

## Context

cxpak v0.12.0 adds data-layer awareness, which extracts tables, columns, foreign keys, ORM
models, and migrations. This metadata is a distinct concern from the per-symbol parse data
that the tree-sitter pass produces, and the question is where to store it.

The design stores it as a dedicated `SchemaIndex`, built after the base `CodebaseIndex` is
already constructed, attached as an optional field so repositories with no data layer pay
nothing. `SchemaIndex` is defined in `src/schema/mod.rs`; the field lives on `CodebaseIndex`
in `src/index/mod.rs`.

## Options considered

- **Option A — separate `Option<SchemaIndex>` field on `CodebaseIndex`:**
  `SchemaIndex { tables, views, functions, orm_models, migrations }`, stored as
  `pub schema: Option<SchemaIndex>`, built in a post-parse pass and `None` when no schema
  files exist. Most pipeline consumers never touch schema data, so isolating it keeps the
  parser and the common path clean; non-database repos incur zero overhead, and the feature
  degrades gracefully. The cost is an extra build step and a field to thread through callers.
  Chosen.

- **Option B — embed schema info in `Symbol`:** attach table/column metadata to each parsed
  `Symbol`. Someone could prefer this to avoid introducing a new structure at all. Rejected:
  it couples schema to the parser data model, and most consumers of `Symbol` do not need
  schema information.

- **Option C — always-present (non-optional) `SchemaIndex`:** store an empty `SchemaIndex`
  even for repos with no data layer. A reasonable alternative would have been to keep the
  field non-optional to spare callers the `Option` unwrap. Rejected: it adds overhead and
  noise for the common case of a repository with no data layer.

## Decision

Define `SchemaIndex` as a separate struct holding `tables`, `views`, `functions`,
`orm_models`, and `migrations`, stored as `pub schema: Option<SchemaIndex>` on
`CodebaseIndex`. Populate it during `build()` / `build_with_content()` by running schema
detection and extraction after the base index is built, then pass `index.schema.as_ref()`
into `build_dependency_graph()`. The field is `None` for repositories with no schema files.

## Consequences

### Positive
- Parsers stay simple; schema is a separate concern.
- Features degrade gracefully — no overhead for non-database repos.
- `SchemaIndex` is serde-serializable like the rest of the data model.

### Negative
- Adds a post-parse pass to index construction.
- Callers that want schema-aware edges must pass `index.schema.as_ref()`.

### Neutral
- `build_dependency_graph()` is extended to accept `Option<&SchemaIndex>`.

## Revisit if
- Schema information needs to be available during parsing rather than after.
- An always-on schema summary becomes cheaper than the `Option` indirection.
