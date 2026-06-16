---
id: '0090'
title: Map tests to sources via naming conventions plus import analysis, not content matching
status: ACCEPTED
date: 2026-03-22
triggered_by: Need source→test mapping to auto-include tests and classify test files in blast radius
loop: planning
---

# ADR-0090: Map tests to sources via naming conventions plus import analysis

## Context

v0.13.0 needs a source→test mapping so `auto_context` can auto-include the tests for selected files and so blast-radius analysis can classify test files. Test mapping combines language-specific naming-convention matching (6 explicit language patterns plus a catch-all covering all 42 languages) with import analysis: a test file that imports a source maps to it. Confidence is recorded as `NameMatch`, `ImportMatch`, or `Both`. Content analysis on function names is rejected as too noisy.

## Options considered

- **Option A — Naming conventions + import analysis with confidence levels:** Generate candidate test paths per language and check existence; additionally map test files to the source modules they import; record confidence as `NameMatch`/`ImportMatch`/`Both`. Pros: two independent signals, avoids false links, cached at build time. Cons: the naming table must track per-language conventions, and the catch-all is approximate. This was the chosen option.
- **Option B — Content / function-name matching:** Link tests to sources by overlapping referenced symbol names. Pros: catches non-conventional layouts that naming/import miss. Cons: content matching on function names creates too many false links. This was the explicitly rejected alternative.

## Decision

Implement `build_test_map()` merging `find_test_files_by_name()` (6 explicit language patterns — Rust, Python, Java, TypeScript/JavaScript, Go, Ruby — plus a strip-prefix/extension catch-all for all 42 languages) and `find_test_files_by_imports()` (test files mapped to the source modules they import), with `TestConfidence` of `NameMatch`/`ImportMatch`/`Both` (upgraded to `Both` when both signals agree), cached on `CodebaseIndex.test_map`.

## Consequences

### Positive
- Two orthogonal signals raise confidence when both agree.
- Avoids the false-link explosion of content matching.
- The catch-all extends mapping beyond the 6 documented language patterns.

### Negative
- Naming patterns must track per-language conventions.
- Import resolution reuses the dependency-graph candidate logic and inherits its limits.

### Neutral
- Feeds `pack_context` `include_tests` (default true) and the blast-radius `test_files` category.

## Revisit if
- Conventional naming/import matching misses a common project layout.
- Confidence levels need finer gradation.
