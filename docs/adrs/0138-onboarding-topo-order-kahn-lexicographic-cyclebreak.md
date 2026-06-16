---
id: '0138'
title: Dependency-first onboarding order via Kahn topological sort with lexicographic cycle-break
status: ACCEPTED
date: 2026-04-01
triggered_by: Onboarding reading-order computation (Tasks 18-20)
loop: implementation
---

# ADR-0138: Dependency-first onboarding order via Kahn topological sort with lexicographic cycle-break

## Context

Shipped in v2.0.0 (Tasks 18-20). The onboarding guide must order files so dependencies are read before dependents, while remaining deterministic and robust to dependency cycles.

## Options considered

- **Option A — Kahn's algorithm with lexicographic cycle-break:** In-degree BFS (over the reversed graph) produces leaves-before-importers order, with seeds sorted lexicographically for determinism; on cycle detection the remaining in-degree>0 nodes are appended sorted by file path. Phases are grouped by 2-segment module prefix, ordered by aggregate PageRank descending, with files kept in topological reading order within a phase, and modules exceeding the 7±2 cognitive limit (max 9 files) split into `(N/M)` sub-phases. Pros: deterministic, never panics on cycles, simple and well-understood, stable output for snapshot tests. Cons: cycle members get an arbitrary (lexicographic) order rather than a semantically meaningful one. This is what shipped.
- **Option B — DFS post-order topological sort:** A reasonable alternative would have been a recursive DFS producing post-order. It also produces dependency order, but needs explicit cycle handling, raises recursion-depth concerns, and has a less obvious determinism tie-break. Reconstructed alternative; not discussed in the plan.

## Decision

`topological_sort_files` uses Kahn's algorithm (in-degree BFS) to produce dependency-first order; on cycle detection it appends remaining in-degree>0 nodes sorted lexicographically. Phases are one-per-module (2-segment prefix), ordered by descending aggregate PageRank, with files within a phase kept in topological (dependency-first) order — the within-module token-count sort was explicitly rejected (regression bugfix 46ced99). Modules over the 7±2 limit are split into `(N/M)` sub-phases.

## Consequences

### Positive
- Deterministic, snapshot-testable reading order.
- No panic on cyclic dependency graphs.

### Negative
- Files inside a cycle are ordered only by path, not by semantic priority.

### Neutral
- Reading time is computed at a fixed 200 tokens/min; test files and generated/vendored files are excluded (reusing the auto_context noise blocklist).

## Revisit if
- Cycle-member ordering needs to be smarter than lexicographic.
- Module grouping at 2-segment depth proves wrong for some repos.
