---
id: '0124'
title: Cross-file call graph with Exact/Approximate confidence and tree-sitter + regex two-tier extraction
status: ACCEPTED
date: 2026-04-01
triggered_by: "v1.3.0 'Deep Understanding': building a cross-file call graph as the foundation for dead-code detection"
loop: implementation
---

# ADR-0124: Cross-file call graph with Exact/Approximate confidence and tree-sitter + regex two-tier extraction

## Context

In v1.3.0 ("Deep Understanding"), cxpak supports 42 languages but only some have tree-sitter call-expression extraction. Resolving a call to a specific exporting file requires import information that is not always available. The design needed to express resolution uncertainty and to cover languages without usable tree-sitter call-site queries — as the foundation for dead-code detection.

## Options considered

- **Option A — two confidence levels + tree-sitter for top-10 Tier-1 langs, regex fallback elsewhere:** Tree-sitter extracts call sites for Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#; all other languages use regex against known symbol names. `Exact` = import-resolved to a specific exporting file; `Approximate` = name exported elsewhere but import not provable. Pros: honest about resolution uncertainty, covers all 42 languages via the regex fallback, fully deterministic. Cons: Approximate edges can be wrong; the regex fallback is coarse. This is the chosen approach.
- **Option B — single edge type, drop unresolvable calls:** A reasonable alternative would have been to only emit edges that can be import-resolved. Pros: no false edges. Cons: loses signal for dynamic/duck-typed languages and for unimported calls. Someone could prefer it to keep the graph fully trustworthy.
- **Option C — full type-resolution / LSP-grade call resolution:** A reasonable alternative would have been to build a real semantic resolver per language. Pros: most accurate. Cons: enormous per-language effort, out of scope for a structural tool. Someone could prefer it if accuracy were the dominant requirement.

## Decision

Define `CallGraph { edges: Vec<CallEdge>, unresolved: Vec<UnresolvedCall> }` stored on `CodebaseIndex` (`src/intelligence/call_graph.rs`, `src/index/mod.rs`). Each `CallEdge` carries `CallConfidence::Exact` (tree-sitter call site import-resolved to a specific exporting file) or `::Approximate` (symbol name exported elsewhere but import not provable). Tree-sitter extraction covers the top-10 Tier-1 languages (Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#); all others use the regex fallback (`extract_regex_calls` per the design doc; shipped as `extract_regex_calls_from_function`) matching known symbol names. Self-calls are skipped. The graph is rebuilt in `CodebaseIndex::build` and on `rebuild_graph()`.

## Consequences

### Positive
- A single graph spans all 42 languages via the regex fallback.
- Confidence levels let downstream consumers (dead-code, data-flow) weight edges by trust.
- Rebuilt incrementally alongside the dependency graph.

### Negative
- Approximate edges may be incorrect; the regex fallback misses method calls and aliases.
- Self-calls and stdlib names are heuristically skipped.

### Neutral
- Stored on `CodebaseIndex`; consumed by `dead_code.rs` (v1.3.0) and `data_flow.rs` (v1.5.0), which maps `Approximate` calls to `Speculative` flow confidence.

## Revisit if
- The Approximate-edge false-positive rate proves too high for downstream features.
- A language needs precise resolution that regex / tree-sitter call-site matching cannot provide.
