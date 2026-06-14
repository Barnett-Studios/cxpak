---
id: '0067'
title: Extend SymbolKind with 17 structural variants for Tier 2 languages
status: ACCEPTED
date: 2026-03-18
triggered_by: Config/markup languages have no functions or classes to extract
loop: planning
---

# ADR-0067: Extend SymbolKind with 17 structural variants for Tier 2 languages

## Context

Part of the cxpak v0.10.0 language-coverage expansion. Tier 2 languages (CSS, SCSS, Markdown, JSON, YAML, TOML, HTML, Makefile, Dockerfile, HCL, Protobuf, GraphQL, etc.) have no traditional functions, classes, or methods to extract. Their meaningful structure is selectors, headings, keys, tables, blocks, message/service definitions, and so on.

The rest of the cxpak pipeline (index, budget, output, relevance) operates on `Vec<Symbol>` generically, regardless of symbol kind. The design needed a way to represent Tier 2 structure without forcing it into existing code-symbol kinds (which would be semantically wrong) and without building a parallel data model (which would fork the pipeline).

## Options considered

- **Option A — Add 17 new structural variants to the existing `SymbolKind` enum:** Tier 2 `extract()` implementations return `Symbol` structs carrying the new kinds (Selector, Mixin, Variable, Heading, Section, Key, Table, Block, Target, Rule, Element, Message, Service, Query, Mutation, Type, Instruction). The downstream pipeline is unchanged because it operates on `Vec<Symbol>` regardless of kind. Pros: zero downstream changes; reuses all existing index/budget/output/relevance machinery. Cons: enlarges a shared enum with config- and markup-specific concepts, mixing code-symbol and config-structure semantics in one type. (Grounded — this is the design as written and shipped.)

- **Option B — A separate `StructuralUnit` type for Tier 2:** Keep `SymbolKind` focused on code symbols and model non-code structure with a distinct type. A reasonable alternative would have been this, since it keeps the code-symbol vocabulary clean. Cons: it forces a parallel pipeline — index, budget, output, and relevance would each need to handle two data models instead of one. (Reconstructed; not formally evaluated in the design doc.)

## Decision

Add 17 new Tier 2 structural variants to `SymbolKind` in `src/parser/language.rs`. Tier 2 `extract()` returns `Symbol` structs with these kinds; the rest of the pipeline works unchanged because it operates on `Vec<Symbol>` regardless of kind.

Confirmed shipped: the variants (Selector, Mixin, Variable, Heading, Section, Key, Table, Block, Target, Rule, Element, Message, Service, Query, Mutation, Type, Instruction) are present at `src/parser/language.rs:28-44`, extending the existing `SymbolKind` rather than forking a new type. This decision later became load-bearing for v0.11.0's `concept_priority` taxonomy, which maps these exact variants to priority tiers (`src/context_quality/degradation.rs:38-64`).

## Consequences

### Positive
- Index, budget, output, and relevance work unchanged on `Vec<Symbol>`.
- Provided the structural vocabulary later reused by the v0.11.0 concept-priority taxonomy.

### Negative
- `SymbolKind` now mixes code-symbol and config-structure concepts in one shared enum.

### Neutral
- `concept_priority()` must include a catch-all arm for `Import` and any future variants.

## Revisit if
- A Tier 2 language needs a structural unit not covered by the 17 variants.
- The enum grows large enough to warrant a sub-enum split (e.g. separating code kinds from structural kinds).
