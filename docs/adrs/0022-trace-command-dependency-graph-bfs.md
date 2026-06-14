---
id: '0022'
title: Trace command locates a symbol then walks the dependency graph (1-hop default, full BFS with --all)
status: ACCEPTED
date: 2026-03-09
triggered_by: Users need code-path context around a specific symbol or error string without dumping the whole repo into the budget.
loop: implementation
---

# ADR-0022: Trace command locates a symbol then walks the dependency graph (1-hop default, full BFS with --all)

## Context

v0.3.0 adds a `trace` command that reuses the scanner/parser/index pipeline but starts from a target symbol rather than the whole repo, then collects relevant files by walking the dependency graph. This gives focused, budget-bounded context around a symbol or error string without dumping the entire codebase into the token budget.

## Options considered

- **Option A — Symbol-name lookup with content-match fallback, then graph BFS:** Try `find_symbol` (case-insensitive) first; if empty, fall back to `find_content_matches` on raw content; then walk dependencies and dependents. Pros: handles both symbol names and error strings, reuses the existing pipeline, and keeps the default scope bounded. Cons: the content fallback can match unrelated text. Chosen.
- **Option B — Symbol-name lookup only:** A reasonable alternative would have been to require an exact parsed symbol and fail if not found. Pros: precise, with no false positives. Cons: cannot trace error-message strings that are not symbols; someone could prefer it for stricter, predictable matching.
- **Option C — Always full BFS (no 1-hop default):** A reasonable alternative would have been to always traverse the entire reachable set. Pros: maximally complete context. Cons: blows the token budget on large graphs — `--all` exists precisely to opt into this when wanted; someone could prefer it if completeness mattered more than budget.

## Decision

Implement `trace` via `index.find_symbol(target)` (case-insensitive) with a `find_content_matches` fallback for error strings; exit non-zero if neither matches. Walk the `DependencyGraph` following both dependencies and dependents — 1 hop by default, full `reachable_from` BFS when `--all` is passed. Output target info, matched-symbol source bodies, relevant signatures, and the dependency subgraph, each token-budgeted.

Confirmed shipped in `src/commands/trace.rs` (symbol-first with content fallback, `Err` on no match) and `reachable_from` in `src/index/graph.rs`.

## Consequences

### Positive
- A single command yields focused, budget-bounded context around a symbol.
- Reuses the existing scan/parse/index pipeline.
- Works for error-message strings, not just symbols.

### Negative
- The content-match fallback may pull in incidental matches.
- The 1-hop default can miss transitively relevant files unless `--all` is used.

### Neutral
- Trace output reuses the `OutputSections` slots: source bodies go in `key_files`, signatures in `signatures`, the subgraph in `dependency_graph`; `module_map` is left empty.

## Revisit if
- The content fallback proves too noisy in practice.
- The 1-hop default is too narrow for common queries.
