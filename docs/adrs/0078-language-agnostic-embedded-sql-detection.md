---
id: '0078'
title: Detect embedded SQL by scanning string literals with a structural-keyword guard, not via framework/API awareness
status: ACCEPTED
date: 2026-03-21
triggered_by: Need to link application code that queries tables back to schema files across all 42 languages
loop: planning
---

# ADR-0078: Detect embedded SQL by scanning string literals with a structural-keyword guard, not via framework/API awareness

## Context
v0.12.0 emits `EmbeddedSql` edges from application code files to schema files. To
work uniformly across all 42 supported languages, the design scans raw string
literals in symbol bodies (and in raw file content) for SQL, rather than
recognizing specific DB-client APIs. To suppress natural-language false positives,
a string must contain both a DML/DDL keyword and a structural keyword to be treated
as SQL.

## Options considered
- **Option A — string-literal scan plus structural-keyword guard:** find quoted
  strings, require a DML keyword (`SELECT`/`INSERT`/`DELETE`/`CREATE`) *and* a
  structural keyword (`FROM`/`INTO`/`TABLE`/`SET`/`UPDATE`/`JOIN`), then extract
  table names by position after `FROM`/`JOIN`/`INTO`/`UPDATE`/`TABLE`. Pros:
  language-agnostic, no framework treadmill, deterministic; the structural guard
  prevents false positives like `"SELECT the best option"`; comment lines (`//`,
  `#`, `--`, `*`) are skipped as an additional guard. Cons: may miss SQL built via
  string concatenation or ORM query builders. Someone could prefer it to avoid
  per-framework maintenance. (Chosen.)
- **Option B — API-aware detection:** recognize `db.execute`/`cursor.execute` and
  specific client libraries per language. Pros: fewer false positives on non-SQL
  strings. Cons: a framework treadmill — every DB client across every language must
  be tracked. Someone could prefer it for precision on a known stack.

## Decision
Implement `detect_embedded_sql()` to scan quoted strings (in symbol bodies and raw
file content) for a DML keyword (`SELECT`/`INSERT`/`DELETE`/`CREATE` — `UPDATE` is
deliberately excluded from the DML guard to avoid `.update()` method false
positives) plus a structural keyword (`FROM`/`INTO`/`TABLE`/`SET`/`UPDATE`/`JOIN`),
then extract table names by position after `FROM`/`JOIN`/`INTO`/`UPDATE`/`TABLE`,
filtering out variables like `$1`/`?`. Comment lines are skipped. Results are
deduplicated by table per file. Applies to all languages. Shipped in
`src/schema/link.rs`.

## Consequences
### Positive
- Works uniformly across all languages with no per-framework code.
- The structural guard (and comment-line skipping) suppresses natural-language
  false positives.
- Catches module-level SQL outside parsed symbols by also scanning `file.content`.

### Negative
- Cannot detect SQL assembled by concatenation or query builders.
- Pure string heuristics can still miss or overmatch unusual SQL.

### Neutral
- Deduplication is needed between the symbol-body and file-content scans.

## Revisit if
- The false-positive/false-negative rate proves too high in practice.
- A dominant ORM query-builder pattern warrants targeted detection.
