---
id: '0213'
title: Bounded-batch fold replaces whole-repo collect in build_index, to cap peak RSS
status: PROPOSED
date: 2026-10-18
triggered_by: benches/BASELINES.md (3.5.0 max RSS 316-396 MB vs 3.4.0 273-280 MB, scriptorium warm start); spec A3 not met
loop: planning
---

# ADR-0213: Bounded-batch fold replaces whole-repo collect in `build_index`, to cap peak RSS

## Context

`build_index` (`src/commands/serve.rs:440-560`) reads every file's content and builds its artifacts
in one `files.par_iter().zip(..).map(..).collect()` into `prepared: Vec<Option<Prepared>>` — an
O(repo) resident set (whole-tree source text + parsed artifacts, specs/2026-10-18-peak-rss-regression-SPEC.md
§2) held until the collect finishes, only then folded into the index and the artifact store. 3.5.0
(ADR-0210, ADR-0212) added two per-entry maps (`term_frequencies`, `call_sites_by_symbol`) to the
artifact store that 3.4.0 did not carry, which is the measured RSS delta's dominant likely source
(code-path analysis; not allocator-profiled — see spec §2's caveat). The fix must not regress
`index_ready_s` beyond +10% vs 3.5.0, must not bump `CACHE_VERSION`/`ARTIFACT_STORE_VERSION`, and
must not change any public struct.

## Options considered

- **Option A — bounded-batch fold.** Chunk `files`/`stored` into fixed-size batches, run each
  batch through today's `par_iter().map(..).collect()` closure, then fold that batch's results into
  the running `BuildReport`/`changed`/`scanned`/index accumulators before starting the next batch.
  *Pros:* smallest change, touches one function, no on-disk format change, caps the resident `Vec`
  to O(batch) instead of O(repo), rayon parallelism preserved within each batch. *Cons:* chunk size
  is a new tuning knob (too small loses parallel efficiency; too large doesn't help); the fold step
  must exactly preserve `merge_store_entries`' existing ordering/pruning semantics, so it needs a RED
  test pinning that before the change lands.
- **Option B — binary/streaming store encoding** (the ADR-0210 "Revisit if" trigger, bincode already
  behind the `embeddings` feature). *Pros:* shrinks the *store's own* in-memory size, and its on-disk
  size. *Cons:* ADR-0210's own revisit trigger is store *load time* exceeding 25% of index-ready
  time, not RSS — the measured peak (spec §2) is dominated by the whole-repo `prepared` Vec, which a
  smaller store encoding does not bound; would still need Option A's chunking for the real fix and
  adds scope (a new encoding format, migration from `artifacts.json`) a patch release shouldn't carry.
- **Option C — drop `content` from `Prepared` once consumed**, re-reading from disk later wherever
  `CodebaseIndex` needs raw text (e.g. search snippets). *Pros:* removes the single largest per-file
  payload from the resident set entirely. *Cons:* changes `CodebaseIndex`'s contract for every reader
  of file content (unknown blast radius without a fuller audit than this patch's budget allows);
  double-reads add latency variance keyed to disk speed, not CPU, for every later content access.
- **Option D — do nothing, document the regression.** *Pros:* zero risk, zero engineering cost.
  *Cons:* spec A3 stays unmet and the regression compounds as repos grow; the measured ratio
  (up to +45%) is a real cost on memory-constrained dev machines (ADR-0211 already measures 7
  concurrent servers at ~2.4 GB).

## Decision

**Option A.** Bound `build_index`'s parallel collect to fixed-size batches (initial guess:
`files.len() / num_cpus().max(1)`, clamped to a sane floor/ceiling — tuned empirically against the
1.10× RSS gate and the +10% time gate, not fixed by this ADR) and fold each batch's `Prepared`
results into the running accumulators (`report`, `changed`, `scanned`, `content_shas`,
`artifact_files`, `body_shas`) before the next batch starts. Options B and C stay available if A
alone cannot clear the gate (B's trigger condition already exists in ADR-0210; C needs a separate
decision given its wider blast radius).

## Consequences

### Positive
- Peak RSS bounded by batch size, not repo size — the fix scales to repos much larger than
  scriptorium without a further regression.
- No on-disk format change, no public API change, no `*_VERSION` bump: ships as a patch (3.5.1).
- `merge_store_entries` and the hit/miss/reuse counters are unit-testable in isolation from the
  batching, so the RED test pins current semantics before the refactor touches them.

### Negative
- A new tuning parameter (batch size) that this ADR does not fix a value for — the plan must
  measure it, and a future repo-shape change could need re-tuning.
- Slightly more code in `build_index` (a fold loop around the existing closure) — contained to one
  function, no new module.

### Neutral
- Rayon's within-batch parallelism means wall-clock impact is expected to be small if the batch size
  is chosen well above `num_cpus()`; this is a prediction the plan's benchmark must confirm, not an
  assumed outcome.

## Revisit if

- The 1.10× RSS gate is not met at any batch size that also holds the +10% time gate — fall through
  to Option C (drop `content` from `Prepared`) as a follow-up ADR.
- A future repo is large enough that even one batch's `Prepared` set is itself a RSS problem —
  revisit with per-file streaming (Option C) rather than a smaller fixed batch.
- `ArtifactStore`'s own load time (not RSS) crosses ADR-0210's existing 25%-of-index-ready trigger —
  that is Option B's separate, already-recorded trigger, unaffected by this decision.
