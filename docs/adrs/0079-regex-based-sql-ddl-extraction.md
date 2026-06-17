---
id: '0079'
title: Extract SQL column-level schema via regex on the DDL body rather than tree-sitter re-parse
status: ACCEPTED
date: 2026-03-21
triggered_by: tree-sitter-sql (v0.0.2) child node types for column definitions are unverified/immature
loop: planning
---

# ADR-0079: Extract SQL column-level schema via regex on the DDL body rather than tree-sitter re-parse

## Context

cxpak v0.12.0 introduces data-layer awareness, which requires column-level extraction
(column names, types, constraints, foreign keys, primary keys) from `CREATE TABLE` bodies.
The schema is already located as a `Symbol` by the tree-sitter pass, but turning the symbol
body into structured table/column metadata is a separate problem.

The data layer is detected and indexed in `src/schema/`, and the column extraction logic
lives in `src/schema/extract.rs` (`extract_table_schema`, `extract_view_schema`,
`extract_function_schema`). The constraint is that the SQL grammar crate, `tree-sitter-sql`,
is at v0.0.2 and its child node types for column definitions are unverified — re-walking
the AST for column structure is not a dependable foundation.

## Options considered

- **Option A — Regex/text extraction on the DDL body (Option B in the design doc: deep DDL parsing):**
  split column definitions on top-level commas, take the first token as the column name, take
  tokens until the first constraint keyword as the (possibly multi-word) type, then scan the
  remainder for constraints and `REFERENCES table(col)` foreign keys. DDL has exactly one correct
  parse, so a text parser is deterministic and immune to grammar immaturity; dialect-specific type
  strings are captured as raw text. The cost is a hand-rolled micro-parser whose edge cases (quoted
  identifiers, multi-word types, `CHECK`) must be handled by hand. Chosen.

- **Option B — tree-sitter re-parse of column definitions:** walk `tree-sitter-sql` child nodes for
  `column_definition`. Someone could prefer this for a structured parse that avoids maintaining a
  bespoke parser. Rejected: at v0.0.2 the grammar is immature and the child node types for column
  definitions are unverified, so the structure can shift or be wrong underneath us.

- **Option C — DML query analysis:** infer schema from `SELECT`/`INSERT` usage across application
  code. Someone could prefer this for capturing how the schema is actually used at runtime. Rejected:
  DML is context-dependent and has no single correct parse, unlike DDL.

## Decision

Implement `extract_table_schema`, `extract_view_schema`, and `extract_function_schema` as
regex/text parsers over `Symbol.body`. For each table, split column definitions at the top
paren level, capture the column name (first token), accumulate the type as every token up to
the first constraint keyword (so multi-word types like `TIMESTAMP WITH TIME ZONE` and
`DOUBLE PRECISION` and paren groups like `VARCHAR(255)` and `DECIMAL(10,2)` are preserved),
then capture constraints and `REFERENCES table(col)` foreign keys. Type strings are stored as
raw text. Dialect-specific clauses (ClickHouse `ENGINE`, Cassandra `WITH`) sit after the closing
paren and are skipped without error.

## Consequences

### Positive
- Deterministic parse, independent of the maturity of `tree-sitter-sql`.
- Multi-word types, quoted identifiers, and schema-qualified foreign keys are handled correctly.
- Dialect-specific clauses are skipped gracefully, producing no errors.

### Negative
- Maintains a bespoke DDL micro-parser instead of leaning on the grammar.
- The constraint-keyword list (`NOT`, `NULL`, `UNIQUE`, `PRIMARY`, `DEFAULT`, `REFERENCES`,
  `CHECK`, `CONSTRAINT`, `COLLATE`, `GENERATED`, `AUTO_INCREMENT`) must stay complete; an
  omission mis-types a column by swallowing its constraint into the type string.

### Neutral
- CQL reuses the same SQL extraction path (the `.cql` extension routes to SQL extraction in
  `src/schema/detect.rs`); ClickHouse is handled by skipping the `ENGINE` clause.

## Revisit if
- `tree-sitter-sql` matures to a version with stable, verified column-definition node types.
- Regex extraction proves unreliable on a real-world dialect that warrants a structured parse.
