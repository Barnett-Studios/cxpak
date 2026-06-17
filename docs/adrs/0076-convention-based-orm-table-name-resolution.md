---
id: '0076'
title: Resolve ORM model table names by convention with deterministic override detection, no inflector dependency
status: ACCEPTED
date: 2026-03-21
triggered_by: ORM models (Django, SQLAlchemy, TypeORM, ActiveRecord, Prisma) must map to table names to link models to schema
loop: planning
---

# ADR-0076: Resolve ORM model table names by convention with deterministic override detection, no inflector dependency

## Context
v0.12.0's schema module links ORM models to physical tables. Each supported ORM has
an explicit override mechanism (`db_table`, `__tablename__`, `@Entity("X")`,
`@@map("X")`) and a default naming convention. The design resolves table names with
deterministic per-framework detection plus a small rule-based pluralizer for
ActiveRecord, rather than pulling in an inflection library. It adds a SQLAlchemy
import guard (to avoid tagging unrelated `Base` subclasses) and a TypeORM
member-decorator heuristic (because `@Entity` sits on a sibling AST node).

## Options considered
- **Option A — convention plus explicit override detection, rule-based
  pluralization:** detect each ORM by class signature/decorators; read the override
  (`db_table`/`__tablename__`/`@Entity`/`@@map`) if present, else apply the default;
  ActiveRecord pluralizes via `s` / `y`->`ies` / `x,ch,sh`->`es` rules. Pros: fully
  deterministic; covers ~95% of pluralization without a dependency; the SQLAlchemy
  import guard prevents false positives. Cons: the pluralizer misses irregular
  plurals; TypeORM must be detected via member decorators because `@Entity` is a
  sibling AST node. Someone could prefer it to avoid an inflection dependency.
  (Chosen.)
- **Option B — inflector crate for pluralization:** use an English inflection
  library for ActiveRecord table names. Pros: handles irregular plurals correctly.
  Cons: an extra dependency; the design judged simple rules sufficient for ~95% of
  cases. Someone could prefer it for correctness on irregular plurals.

## Decision
Detect ORM models per framework (Django `models.Model`, SQLAlchemy `(Base)` plus a
`sqlalchemy` import guard, TypeORM via member decorators, ActiveRecord `<
ActiveRecord::Base`/`ApplicationRecord`, Prisma struct). Resolve the table name from
the explicit override when present, otherwise the convention default; pluralize
ActiveRecord names with a small hand-rolled rule set instead of an inflector crate.
Shipped in `src/schema/detect.rs` with no inflection dependency in `Cargo.toml`.

## Consequences
### Positive
- Deterministic mapping with no extra dependency.
- The import guard avoids tagging non-ORM `Base` subclasses as SQLAlchemy models.
- TypeORM is detected reliably despite `@Entity` living on a sibling node.

### Negative
- The rule-based pluralizer fails on irregular plurals (~5%).
- Per-framework heuristics must be maintained as ORMs evolve.

### Neutral
- ActiveRecord field extraction is inferred from migrations, not the model body.

## Revisit if
- Irregular-plural mismatches cause real linking failures.
- A new ORM framework needs support.
