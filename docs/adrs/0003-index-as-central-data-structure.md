---
id: '0003'
title: The Index is the single source of truth populated by Scanner/Parser and read by Budget/Output
status: ACCEPTED
date: 2026-03-05
triggered_by: Need a clear data-ownership model across the Scanner -> Parser -> Index -> Budget -> Output pipeline
loop: planning
---

# ADR-0003: The Index is the single source of truth populated by Scanner/Parser and read by Budget/Output

## Context

In v0.1.0, the pipeline has distinct stages: Scanner, Parser, Index, Budget, Output. These stages need a data-ownership model. The design designates a central `CodebaseIndex` that the upstream stages (Scanner, Parser) populate and the downstream stages (Budget, Output) read from, rather than threading ad-hoc state from one stage to the next: "Index is the central data structure — Scanner and Parser populate it, Budget reads from it, Output renders from it. Single source of truth."

## Options considered

- **Option A — Central Index as single source of truth:** A `CodebaseIndex` holds files, language stats, token counts, and the dependency graph; upstream stages populate it and downstream stages query it. Pros: clear ownership, one place to query global state, and decoupling of rendering from parsing. Cons: a large central struct that nearly everything depends on. Someone could prefer this for the queryable global view it gives Budget and Output.
- **Option B — Stage-to-stage streaming with no central store:** A reasonable alternative would have been to let each stage pass its output directly to the next with no shared index. Pros: lower peak memory, since intermediate results need not all be retained. Cons: no global queryable view, making holistic budgeting and rendering harder. (This alternative was not formally evaluated in the design doc; it is reconstructed here as the natural counterfactual.)

## Decision

Make `CodebaseIndex` the central data structure and single source of truth. Scanner and Parser populate it, Budget reads from it, and Output renders from it. It carries the files, per-language stats, token counts, and the dependency graph; token counts are computed once during the index pass and cached, with no re-tokenization downstream.

## Consequences

### Positive
- Downstream stages share one queryable global view of the codebase.
- Token counts are computed once during the index pass and cached on the index.

### Negative
- The whole-repo index is held in memory, so large repositories pay the full cost up front.

### Neutral
- `CodebaseIndex` grew across versions to also carry `SchemaIndex`, `co_changes`, PageRank inputs, and more — all hanging off this same central struct.

## Revisit if
- Memory pressure on very large monorepos forces a streaming or on-disk index instead of an in-memory one.
