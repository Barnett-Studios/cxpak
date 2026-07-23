---
id: '0205'
title: Watcher single-clone — one deep clone per edit batch, Arc-shared embedding matrix
status: ACCEPTED
date: 2026-07-23
triggered_by: field report (issue #47) — `cxpak serve --mcp` grew to a 57 GB physical footprint (peak 64 GB) after ~44 min of a normal editing session, climbing ~1 GB/min
loop: implementation
---

# ADR-0205: Watcher single-clone — one deep clone per edit batch, Arc-shared embedding matrix

## Context

ADR-0204 bounded the watcher's *ingestion set* and guarded the deep clone on
**spurious** wakes (all-ignored batches early-return before the clone). It did
**not** touch the **real-edit** path, where each debounced batch that carries a
genuine source change still deep-cloned the entire `CodebaseIndex` **twice**:

1. `process_watcher_changes` (`serve.rs`) — `let mut next = (*snapshot).clone()`,
   the working copy the delta rebuild mutates and swaps into the `shared` mirror.
2. `republish_watcher_index` (`serve.rs`) — `(**g).clone()` of that just-swapped
   index **a second time**, solely to produce a copy with `embedding_index = None`
   (the ADR-0200 6-signal fallback) to publish into the `readiness` cell.

`CodebaseIndex` derives `Clone` with no structural sharing (`core_graph/index.rs`),
so each clone is a full deep copy of every `IndexedFile.content` String,
`term_frequencies`, `graph`, `call_graph`, **and** the embedding matrix
(`EmbeddingIndex.vectors: Vec<f32>`). Under a continuous editing session (roughly
one batch/second) this sustained two whole-index deep copies per edit — with the
embedding matrix copied on both — driving the physical footprint into the tens of
GB. A `vmmap` field report showed 16 GB retained in `MALLOC_LARGE` (the flat
`Vec<f32>` matrices) and 44 GB in `MALLOC_SMALL` (file contents), dirty and
swapped — live retention plus allocator churn, not a bounded working set.

Two mechanisms compounded the cost:
- The **redundant second whole-index deep clone** existed only to null one field.
- The **embedding matrix was deep-copied by value** on every `CodebaseIndex`
  clone, because `embedding_index` was `Option<EmbeddingIndex>` (owned by value).

## Options considered

- **Option A — strip embeddings before the swap + Arc-share the matrix (chosen):**
  clear `embedding_index` once, inside `process_watcher_changes`, immediately
  before `Arc::new(next)`, and return that freshly-swapped `Arc` so the watcher
  publishes the *same* pointer into `readiness` — deleting the second clone
  entirely. Independently, change the field to `Option<Arc<EmbeddingIndex>>` so
  cloning a `CodebaseIndex` bumps a refcount for the matrix instead of copying
  `Vec<f32>`. The two changes ship together: stripping-before-swap alone keeps
  the ADR-0200 invariant, and the `Arc` wrap removes the matrix from the *first*
  (unavoidable) clone's cost as well as the enrichment clone's.
- **Option B — make `republish_watcher_index`'s clone cheap via the `Arc` field:**
  keep the second clone but rely on the `Arc`-wrapped matrix to make it O(1).
  Rejected: Arc-wrapping *only* the embedding field does not make the whole-index
  `(**g).clone()` cheap — `files`/`content`, `term_frequencies`, and `graph` are
  still deep-copied. It would leave the 44 GB `MALLOC_SMALL` churn fully in place.
- **Option C — in-place mutation of the swapped index (issue #47 suggestion 3):**
  mutate the `Arc`'s pointee instead of cloning. Rejected: violates the
  snapshot-then-swap reader-consistency invariant (`serve.rs`, the comment block
  above the clone) — long-running MCP tool handlers alias the pre-swap `Arc`, so
  in-place mutation is a torn-read hazard for no benefit over Option A.

## Decision

Adopt Option A.

- `embedding_index: Option<Arc<crate::embeddings::EmbeddingIndex>>`
  (`core_graph/index.rs`). Read sites auto-deref through `Arc`
  (`relevance/signals.rs`); `has_embedding_index()` is unchanged; the two
  by-value `Some(emb)` writes become `Some(Arc::new(emb))`
  (`serve.rs` `publish_ready_enriched`, plus test constructors). `CodebaseIndex`
  derives only `Debug, Clone` (no `serde`), so the wrap introduces no
  serialization change; `EmbeddingIndex` keeps its own `Serialize/Deserialize`
  and `load`/`save`, which never run through the index.
- `process_watcher_changes` now clears `embedding_index` once before the swap
  and returns `Option<Arc<CodebaseIndex>>` — `Some(new_arc)` on a real change,
  `None` on the ignored-only / no-op / poisoned paths. `spawn_mcp_watcher`
  publishes that exact `Arc` into `readiness`. `republish_watcher_index` is
  **deleted** (its clearing responsibility moved into the pre-swap step).
- **Copy-on-write file storage:** `CodebaseIndex.files: Vec<IndexedFile>` →
  `Vec<Arc<IndexedFile>>`. Cloning a `CodebaseIndex` now shares every unchanged
  file as a refcount bump; the surviving per-batch clone copies only the `Vec`
  spine plus the handful of files the delta actually touches, turning the
  residual `MALLOC_SMALL` cost from O(codebase) to O(delta). The delta path is
  copy-on-write by construction: `upsert_file` is `remove_file`
  (`swap_remove` the whole `Arc`) + `push(Arc::new(..))`, and `remove_file`
  drops an `Arc` — neither mutates an `IndexedFile` in place, so an old snapshot
  a reader still holds is never corrupted. Read sites auto-deref through
  `Arc<IndexedFile>`; nine helper signatures taking `&[IndexedFile]` became
  `&[Arc<IndexedFile>]` (bodies unchanged).

Covered by `process_watcher_changes_clears_embedding_and_publishes_single_arc`
(published index is embeddings-free **and** `Arc::ptr_eq` with the `shared`
mirror — proving a single clone), `embedding_index_clone_shares_matrix_not_copies`
(`Arc::strong_count` proves the matrix is shared, not copied — and the test
cannot compile against the pre-fix non-`Arc` field),
`process_watcher_changes_ignored_only_publishes_none`, and the memory-ceiling
proof `watcher_does_not_accumulate_index_versions_across_batches` (drives 25
real-edit batches and asserts `Arc::strong_count` on the shared/readiness Arc
returns to a fixed baseline every batch — no version accumulation). The ADR-0204
parity guards (`process_watcher_changes_delta_parity_with_full_rebuild`,
`classify_changes` tests) remain green, confirming byte-identical delta output.

## Consequences

### Positive
- **One** whole-index deep clone per real-edit batch instead of two, and that
  surviving clone is now copy-on-write: unchanged files are shared `Arc`s, so it
  copies O(delta) file contents, not O(codebase). Together this collapses the
  `MALLOC_SMALL` (file-content) growth that dominated the 44 GB report.
- The embedding matrix is never deep-copied on the watcher path: per-batch matrix
  copies drop from three (the two watcher clones plus the enrichment clone) to
  zero — the ~16 GB `MALLOC_LARGE` class is eliminated.
- The ADR-0200 6-signal-fallback invariant is preserved and now enforced at a
  single, testable point (pre-swap) rather than via a redundant clone.
- A regression test demonstrates (not merely asserts) the memory ceiling:
  `Arc::strong_count` returns to a fixed baseline after every batch.

### Negative
- The embeddings-clearing responsibility is no longer named by a dedicated
  `republish_watcher_index` function; it is a documented step inside
  `process_watcher_changes`. Auditing the invariant now means reading that step
  (called out in-line with an ADR-0200/0205 comment) rather than a standalone fn.
- `process_watcher_changes` gained a return value; the HTTP watcher
  (`serve.rs`) and LSP backend (`lsp/backend.rs`) call it as a statement and
  discard the `Option` — intentional, as they own no `readiness` cell.

### Neutral
- The per-batch clone still copies the `Vec` spine (one pointer per file) plus
  the small scalar/`String` fields of *changed* files and the non-file index
  members (`term_frequencies`, `graph`, `call_graph`). These are far smaller than
  the file contents now shared via `Arc`; if a profile ever shows them dominating,
  the same `Arc`/delta treatment applies to `term_frequencies` and `graph`.
- The `Arc::strong_count`-returns-to-baseline test demonstrates no *extra* index
  version survives a batch; it does not, by itself, prove no *external* holder
  pins an old version (see Revisit if).

## Revisit if
- **An old `Arc<CodebaseIndex>` is pinned by a slow/hung MCP tool handler**
  (`snapshot_ready_index` hands each call an `Arc::clone`). The ceiling test
  proves no version accumulates in the watcher/readiness path itself, but a
  handler that stalls could still retain a full old index for its lifetime. The
  MCP stdio loop is synchronous/sequential today, so at most one is pinned; if a
  future concurrent handler model is added, audit with `Arc::strong_count` / a
  heap profiler (`dhat`/`heaptrack`) on a long-lived session and bound concurrent
  index versions if retention is observed.
- **The per-batch `git2::Repository::discover` + gitignore matcher rebuild in
  `classify_changes` shows up in a CPU profile.** It is churn only (freed each
  call, not a memory contributor), but ADR-0204 and this ADR both note it can be
  hoisted to the watcher's lifetime. Deferred (issue #47 Fix #4).
