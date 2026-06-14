---
id: '0101'
title: Call graph uses tree-sitter extraction for Tier 1 languages and regex for Tier 2, tagged by confidence
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.3.0 'Deep Understanding' needs a cross-file call graph
loop: planning
---

# ADR-0101: Call graph uses tree-sitter extraction for Tier 1 languages and regex for Tier 2, tagged by confidence

## Context
v1.3.0 "Deep Understanding" needed a cross-file call graph. A precise graph requires
per-language tree-sitter extraction of call expressions plus import resolution — work
that is expensive to build for all 40 languages, and Tier 2 languages have only
structural parsers. The design doc chose a hybrid: "Tier 1 languages (26): Tree-sitter
extraction of call expressions ... Match call targets against known symbols via import
resolution. Produces precise edges. Tier 2 languages (14): Regex scan ... tagged as
`confidence: Approximate`," and "Initial v1.3.0 ships with call extraction for the top
10 Tier 1 languages (Rust, TypeScript, JavaScript, Python, Java, Go, C, C++, Ruby, C#)."

This is a human decision because it trades uniform precision for shipping speed and
broad coverage, and it commits to a confidence tag so downstream features can weight
edges — neither trade-off is something the code can resolve. Shipped in
`src/intelligence/call_graph.rs`, with the graph stored on `CodebaseIndex`.

## Options considered
- **Option A — hybrid: tree-sitter + import resolution for Tier 1 (Exact), regex for
  Tier 2 (Approximate):** Tier 1 (26 languages) extract call expressions and resolve
  callees via the import graph, edges tagged `Exact`; Tier 2 (14 languages) regex-scan
  bodies for known symbol names, tagged `Approximate`; ship the top-10 Tier 1 languages
  first and roll out the rest in patches. Pros: precise edges where possible, broad
  coverage everywhere via regex; the confidence tag lets consumers weight edges;
  incremental per-language rollout reduces shipping risk. Cons: regex edges have a
  higher false-positive rate; early v1.3.0 lacks call extraction for some Tier 1
  languages. Someone could prefer this for shipping coverage without waiting on every
  language. (Chosen.)
- **Option B — tree-sitter extraction for all languages before shipping:** wait until
  call extraction exists for every supported language. A reasonable alternative would
  have been to hold for uniform precision. Pros: uniform precision, no confidence tag
  needed. Cons: massively delays v1.3.0; Tier 2 languages only have structural parsers
  anyway. Someone could prefer it to avoid approximate edges entirely.

## Decision
Build the call graph with a hybrid approach: Tier 1 languages (26) use tree-sitter
call-expression extraction with import resolution producing `Exact` edges; Tier 2
languages (14) use regex scanning of function bodies against known symbol names
producing `Approximate` edges. `CallEdge` carries a `CallConfidence` (`Exact` |
`Approximate`). The graph is computed after index construction and stored on
`CodebaseIndex` alongside `DependencyGraph`. v1.3.0 ships extraction for the top 10
Tier 1 languages first; remaining Tier 1 languages are added in later patches; Tier 2
uses regex from day one. Implemented in `src/intelligence/call_graph.rs`.

## Consequences
### Positive
- Precise call edges for the most-used languages.
- Universal coverage via the regex fallback.
- The confidence tag lets downstream features (dead code, predict, data flow) weight
  edge reliability.

### Negative
- Approximate edges carry false positives.
- Early v1.3.0 lacks call extraction for some Tier 1 languages until later patches.

### Neutral
- `Approximate` also covers unresolvable Tier 1 calls, not only Tier 2 regex hits.

## Revisit if
- The Tier 2 regex false-positive rate proves too high for downstream features.
- A downstream feature requires extraction for all Tier 1 languages before it is
  reliable.
