---
id: '0131'
title: 'Dead-code detection: zero callers AND not an entry point AND no test reference, ranked by liveness score'
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.3.0 dead-code detection consuming the new call graph plus test_map, pagerank, and api_surface
loop: implementation
---

# ADR-0131: Dead-code detection: zero callers AND not an entry point AND no test reference, ranked by liveness score

## Context
Introduced in v1.3.0. A naive "zero callers = dead" rule would flag `main()`, HTTP handlers, trait/interface implementations, and test-only helpers as dead code. The design needed conservative entry-point exclusions to avoid these false positives, plus a ranking so the most concerning dead symbols (public, well-tested, central) surface first. Dead-code detection consumes the new call graph alongside `test_map`, `pagerank`, and `api_surface`.

## Options considered
- **Option A — Three-condition dead rule with entry-point heuristics; liveness = pagerank × (1 + test_file_count) × export_weight:** Dead iff zero call-graph callers AND not an entry point (main, HTTP handler, test fn, pub root export, trait impl/override) AND not referenced from a test file. Rank by liveness score descending. Pros: conservative — avoids the obvious false positives; the ranking surfaces high-impact dead code first. Cons: entry-point detection is signature/string heuristic, not semantic. Someone could prefer it because it balances precision against a useful priority ordering.
- **Option B — Pure zero-callers rule:** A reasonable alternative would have been to flag any symbol with no call-graph callers. Pros: simple. Cons: floods results with `main()`, handlers, trait methods, and test helpers. Someone could prefer it for its implementation simplicity if false positives were acceptable.

## Decision
Classify a symbol as dead only when all three hold: zero callers in the call graph; not an entry point (main; test function by name/attribute; pub export from a module root file like `mod.rs`/`lib.rs`/`index.ts`/`__init__.py`; trait impl / `@Override` / `override`; HTTP handler when the file has detected routes); and not referenced from any test file via the call graph. Rank dead symbols by `liveness_score = pagerank × (1 + test_file_count) × export_weight` (export_weight 2.0 for pub exports, 1.0 otherwise), descending. This populates the `dead_code` dimension of `HealthScore` as `10.0 * (1 - dead_count/total_symbols)`.

## Consequences
### Positive
- Conservative rules suppress the common false positives (main, handlers, trait methods, test helpers).
- Liveness ranking puts important dead code first.
- Closes the loop with `HealthScore` by filling the v1.2.0 `dead_code` placeholder.
### Negative
- Entry-point detection is heuristic (string/signature matching) and can miss framework-specific entry points.
- Trait-dispatch and reflection callers are invisible to the call graph.
### Neutral
- Exposed via `cxpak_dead_code` (default limit 50) and the LSP diagnostic that tags dead symbols `UNNECESSARY`.

## Revisit if
- The false-positive rate from missed entry points is too high.
- A framework's entry points aren't matched by the heuristics.
