---
id: '0122'
title: 'Per-module architecture quality metrics: cohesion ratio, root-file boundary violations, mean+2σ god files'
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.3.0 extending v1.2.0 ModuleInfo with quantified architecture-quality metrics
loop: implementation
---

# ADR-0122: Per-module architecture quality metrics: cohesion ratio, root-file boundary violations, mean+2σ god files

## Context

In v1.3.0, `ModuleInfo` (from v1.2.0) carried `coupling` and `aggregate_pagerank`. v1.3.0 added three more quality signals: how internally connected a module is (cohesion), which imports bypass a module's public interface (boundary violations), and which files are over-central "god files". Each needed a concrete, deterministic, language-agnostic formula.

## Options considered

- **Option A — density cohesion + root-file boundary check + statistical god files:** Cohesion = `intra_edges / (file_count*(file_count-1))` (directed-edge density, 0.0 for single-file modules); a boundary violation is an import targeting a non-root file of a module, where root files are `mod.rs`/`lib.rs`/`index.ts`/`index.js`/`__init__.py`; a god file is one whose inbound edge count exceeds `mean + 2σ`, computed only when a module has at least 3 files. Pros: all three are deterministic, cheap, and language-agnostic; `BoundaryViolation.edge_type` is a typed `EdgeType`, not a string. Cons: the root-file list is hardcoded; `mean + 2σ` assumes a roughly normal inbound distribution. This is the chosen approach.
- **Option B — fixed inbound-count threshold for god files:** A reasonable alternative would have been to flag any file with more than N inbound edges. Pros: simpler to explain. Cons: not scale-invariant — a threshold tuned for a small repo over-flags a large one and vice versa. Someone could prefer it for a fixed, well-known codebase size.

## Decision

Extend `ModuleInfo` (`src/intelligence/architecture.rs`) with three fields:

- `cohesion` — intra-module directed-edge density = `intra_edges / (file_count*(file_count-1))`, returning 0.0 for single-file modules (undefined ratio).
- `boundary_violations` — typed `BoundaryViolation` entries for imports targeting a non-root file of a module, where root files are `mod.rs`, `lib.rs`, `index.ts`, `index.js`, `__init__.py`. `BoundaryViolation.edge_type` is the typed `EdgeType` enum, not a string.
- `god_files` — files whose inbound edge count exceeds `mean + 2σ`, computed only when a module has at least 3 files (`detect_god_files` returns empty otherwise).

## Consequences

### Positive
- Three deterministic, scale-aware module quality signals.
- The statistical god-file threshold adapts to repo size rather than using a fixed cutoff.
- Typed `edge_type` avoids stringly-typed coupling information.

### Negative
- Hardcoded root-file names won't recognize non-standard barrel/module-entry conventions.
- `mean + 2σ` is meaningless for very small modules (guarded by the ≥3-files rule).

### Neutral
- Surfaced via `cxpak_architecture`; feeds drift detection's `boundary_violation_count` (and `mean_cohesion`) in the v1.4.0 `ArchitectureSnapshot`.

## Revisit if
- A project uses module-root conventions not in the hardcoded list.
- The `mean + 2σ` god-file heuristic produces poor results on skewed inbound distributions.
