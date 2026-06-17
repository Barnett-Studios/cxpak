---
id: '0077'
title: Order migrations by directory pattern + filename sequence, content-reading only for Alembic
status: ACCEPTED
date: 2026-03-21
triggered_by: Need framework-agnostic migration ordering to emit MigrationSequence edges
loop: planning
---

# ADR-0077: Order migrations by directory pattern + filename sequence, content-reading only for Alembic

## Context
v0.12.0's schema module emits `MigrationSequence` edges between consecutive
migrations. Eight migration frameworks — Rails, Alembic, Flyway, Django, Knex,
Prisma, Drizzle, and a Generic fallback — are detected by directory pattern
(`db/migrate/`, `alembic/versions/`, etc.) and filename convention, then sorted by
an extracted sequence (timestamp or number). Alembic is the lone exception that
requires reading the migration file body, because its filenames use a non-sortable
hash prefix and the real order is carried in a `revision = "..."` field.

## Options considered
- **Option A — directory plus filename sequence sorting:** match the directory
  pattern and a filename regex per framework, sort by the extracted sequence, and
  chain consecutive entries. Pros: framework-agnostic, deterministic, no content
  parsing for 7 of 8 frameworks. Cons: Alembic's hash-prefixed filenames force
  reading `revision` from the file body. Someone could prefer it for being cheap and
  deterministic. (Chosen.)
- **Option B — parse migration file contents for a dependency graph:** read each
  migration's up/down and declared parent revision to build an exact order. A
  reasonable alternative would have been to model the real dependency edges. Pros:
  exact ordering, including branches and merges. Cons: expensive content parsing for
  every framework, not just Alembic. Someone could prefer it where branching
  histories must be tracked accurately.

## Decision
Detect migration chains by directory pattern plus filename convention for the eight
frameworks and sort by the extracted sequence, creating `MigrationSequence` edges
between consecutive entries. Only Alembic reads file content (the `revision = "..."`
field) because its hash-prefixed filenames are not sortable. The Generic fallback
applies to any directory with three or more sequenced SQL files. Shipped in
`src/schema/detect.rs` and `src/schema/link.rs`.

## Consequences
### Positive
- No content parsing for 7 of 8 frameworks.
- Deterministic chronological ordering.
- The Generic fallback handles any directory with 3+ sequenced SQL files.

### Negative
- Alembic requires reading file bodies — the one content-dependent path.
- Branching/merge migrations are not modeled beyond a linear sequence.

### Neutral
- `MigrationSequence` edges carry the lowest edge weight (0.5) in risk scoring.

## Revisit if
- Branching migration histories need accurate parent tracking.
- A framework's filenames defeat sequence extraction.
