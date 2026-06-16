---
id: '0061'
title: Store per-file term frequencies as a HashMap on CodebaseIndex, computed at parse time
status: ACCEPTED
date: 2026-03-17
triggered_by: TermFrequency signal needs precomputed per-file term counts
loop: planning
---

# ADR-0061: Store per-file term frequencies as a HashMap on CodebaseIndex, computed at parse time

## Context

In v0.9.0 the TermFrequency relevance signal needs fast lookup of how often query
terms appear in each file. The index must carry this data so the signal can score
candidates cheaply, without pulling a search-engine dependency into the crate.

The data must stay consistent across the incremental index lifecycle: full builds,
content-aware builds, single-file upserts, and removals.

## Options considered

- **Option A — `HashMap<String, HashMap<String, u32>>` on `CodebaseIndex`, built at parse time:**
  Per-file term counts stored in memory on the index. Identifiers are lowercased and
  split on underscore and camelCase boundaries (tokens shorter than 2 characters
  skipped). Built once during indexing and kept in sync on upsert/remove. Pros:
  lightweight, no external deps, computed once during parsing, stays consistent on
  incremental edits. Cons: memory grows with vocabulary, no IDF/ranking sophistication,
  a file's TF is fully recomputed on every change.

- **Option B — embedded full-text search engine (e.g. tantivy):** A reasonable
  alternative would have been to index content into a dedicated search library
  offering real TF-IDF/BM25 ranking. Someone could prefer it for mature, tuned ranking
  out of the box. Rejected as a heavy dependency that is overkill for a single
  lightweight scoring signal.

- **Option C — recompute term counts on demand per query:** A reasonable alternative
  would have been to tokenize file content at scoring time rather than precomputing,
  avoiding any stored state. Someone could prefer it to keep the index smaller.
  Rejected because it re-tokenizes every file on every query, which is slow.

## Decision

Add `term_frequencies: HashMap<String, HashMap<String, u32>>` to `CodebaseIndex`,
populated in `build()` and `build_with_content()`, updated in `upsert_file()`, and
cleaned in `remove_file()`. Terms are lowercased; identifiers are split on underscore
and camelCase boundaries via `split_identifier`, with sub-2-character tokens skipped.
The TermFrequency signal reads directly from this map.

## Consequences

### Positive
- Cheap, deterministic TF signal with no new dependencies.
- Stays consistent across incremental upsert/remove operations.

### Negative
- Memory overhead proportional to the codebase's vocabulary.
- A file's term frequencies are fully recomputed on every change.

### Neutral
- The identifier splitting (snake_case/camelCase) is a custom routine in
  `compute_term_frequencies`/`split_identifier`, not a tokenizer library.

## Revisit if
- The TF signal needs IDF/BM25 weighting.
- Index memory becomes a constraint on large repositories.
