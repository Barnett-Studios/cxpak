---
id: '0107'
title: Dead code is a deterministic binary classification; liveness_score is only a sort key
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.3.0 dead code detection
loop: planning
---

# ADR-0107: Dead code is a deterministic binary classification; liveness_score is only a sort key

## Context
v1.3.0 adds dead-code detection on top of the call graph. The hard problem is false positives: a symbol with zero callers might still be live — an entry point, a test, a trait impl, or a public export. The output also needs to be unambiguous: a clear dead/alive answer, not a fuzzy threshold that conflates "unimportant" with "unused."

## Options considered
- **Option A — Binary dead/alive with entry-point exclusions; `liveness_score` for sorting only:** a symbol is dead iff it has zero callers AND is not an entry point (`main`, HTTP handler from route detection, test function, trait implementation, `pub` export from the lib root) AND is not referenced in test files. `liveness_score = pagerank × (1.0 + test_file_count) × export_weight` (export_weight 2.0 for `pub` exports, 1.0 otherwise) ranks dead symbols by how concerning they are; it is never a cutoff. Pros: no false positives from entry points; a deterministic yes/no answer; sorting surfaces the most concerning dead symbols first. Cons: entry-point heuristics must be maintained per framework/language; approximate Tier 2 call edges can still mislead. (Grounded — this is the shipped design.)
- **Option B — Liveness threshold:** A reasonable alternative would have been to compute a continuous liveness score and call everything below a cutoff dead, giving one tunable knob. Someone could prefer the simplicity of a single dial. Rejected because the threshold is arbitrary and conflates "unimportant" with "unused" — and since every dead symbol has zero callers anyway, a score threshold is meaningless for the dead/alive question itself. (Reconstructed — not formally evaluated in the source.)

## Decision
Dead code is a deterministic binary classification. A symbol is dead iff:
1. it has zero callers in the call graph, AND
2. it is not an entry point — `main`, an HTTP handler from route detection, a test function, a trait implementation, or a `pub` export from the lib root, AND
3. it is not referenced in test files.

`liveness_score = pagerank × (1.0 + test_file_count) × export_weight`, where `export_weight` is `2.0` for `pub` exports and `1.0` otherwise, is metadata for **sorting** dead symbols by importance — never a threshold. A higher score means more concerning: important, has nearby tests, exported, yet never called.

## Consequences
### Positive
- An unambiguous dead/alive answer per symbol.
- Entry-point exclusions prevent the obvious false positives (`main`, handlers, tests, trait impls, public exports).
- Sorting by `liveness_score` surfaces the most concerning dead symbols first.

### Negative
- Per-framework entry-point heuristics need ongoing maintenance.
- Approximate Tier 2 call edges can produce dead-code false positives.

### Neutral
- This dimension is also the v1.3.0 `dead_code` input to the composite health score, which only becomes available once the call graph ships.

## Revisit if
- Entry-point detection misses framework patterns and produces false positives.
- Tier 2 approximate edges cause an unacceptable level of dead-code noise.
