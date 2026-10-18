---
id: '0213'
title: In-place republish via Arc::get_mut and a borrowed derived-cache save, with no public API change (bounded-batch build_index tried and rejected)
status: ACCEPTED
date: 2026-10-18
triggered_by: full-lifecycle measurement (specs/2026-10-18-peak-rss-regression-SPEC.md §1); prior
  revision of this ADR (Arc-wrapping CodebaseIndex's pub fields) rejected on review — core_graph is
  `pub mod`, so changing those fields' types is a semver break, and 3.5.1 must ship with no public
  API change (same rule applied to PR #175)
loop: planning
supersedes-decision-of: '0213' (same ADR number; two prior decisions superseded — see "Prior
  decisions" below)
---

# ADR-0213: In-place republish via `Arc::get_mut` and a borrowed derived-cache save, with no public API change (bounded-batch `build_index` tried and rejected)

## Prior decisions (both superseded)

1. Bounded-batch chunking of `build_index`'s collect only — written against the Ready-only gate,
   before the post-ready peak (scenario C) was measured. Reinstated below as the scenario-U fix
   (it was never wrong, just incomplete).
2. `Arc`-wrap `CodebaseIndex`'s heavy fields (`term_frequencies`, `graph`, `call_graph`, `pagerank`,
   `conventions`, `co_changes`) — rejected: `core_graph` is `pub mod` and these are `pub` fields of
   a `pub struct`; changing their types is a semver-visible break. 3.5.1 is a patch with no public
   API change, no exception.

## Context

Scenario C's dominant, measured peak (spec §1: 374-382 MB vs 3.4.0's 239-243 MB, +56-60%) is
`republish_conventions`'s `let mut next = (**cur).clone();` (`src/commands/serve.rs:2909`) — a
deep clone of the whole `CodebaseIndex` while the old `Arc` is still alive. Decision 2 tried to make
that clone cheap by changing field types; decision 3 (below) instead tries to **avoid the clone
entirely** when safe, using `Arc::get_mut`, which mutates in place without touching any field type.

`Arc::get_mut` succeeds only when the `Arc`'s strong count is 1 — no other clone of that pointer
alive anywhere. Every place that clones an `Arc<CodebaseIndex>` out of the `SharedReadiness` cell is
a potential blocker, by grep of `src/commands/serve.rs`:
- `snapshot_ready_index` (line 3263) — every MCP/HTTP tool call snapshots the cell's `Arc` for the
  duration of that one call (O(1), short-lived by design per the `SharedIndex` doc comment at
  lines 33-43).
- The background-build closure's own `ready_base = Arc::clone(idx)` (line 2785) — held across the
  *entire* conventions-republish + store-save + embeddings-gate sequence (lines 2784-2834) **solely
  so it has a fallback value to hand to `republish_conventions`** and to gate the embeddings phase.
  This is a **self-inflicted** second strong ref that guarantees `get_mut` fails every time,
  independent of any concurrent reader.
- `enrich_ready_with_embeddings` (line 2961) takes `base: &Arc<CodebaseIndex>` — called *after*
  `republish_conventions` returns, using its return value, not a separate hold during the republish
  window itself.
- The MCP freshness watcher (`spawn_mcp_watcher`) — **does** hold a strong ref. `classify_watcher_wait`
  returns `WatcherWait::Seed(Arc::clone(idx))` (serve.rs ~3072) and the watcher stores it in its
  long-lived `shared` cell (~3161) for the whole session. With that ref alive, `Arc::get_mut` would
  fail right after Ready in practice. An earlier revision of this ADR wrongly claimed the watcher
  only writes.

So the fix has three parts: (0) the watcher must not retain a strong `Arc` of the published index between edit batches — it fetches its base from the readiness cell when a batch arrives (readiness stays the single source of truth; ADR-0200 semantics preserved; a watcher edit still lands because the batch-local base is the cell's current value and the result is published back to it); (a) stop pre-capturing `ready_base` as a persistent clone before the
republish call — fetch it from the cell only in the fallback branches that actually need one; (b)
inside `republish_conventions`, attempt `Arc::get_mut` on the cell's own `Arc` under the write lock
before falling back to today's clone-and-swap.

## Options considered

- **Option A — bounded-batch fold in `build_index`** (prior decision 1, reinstated for scenario U).
  Unchanged rationale from the original ADR text: chunks the whole-repo `par_iter().collect()` into
  fixed-size batches, folded incrementally. No field-type change anywhere — no public API risk.
  Does not touch `republish_conventions` or scenario C at all.
- **Option B — `Arc`-wrap heavy fields** (prior decision 2). Rejected: semver break (see above).
- **Option C — `Arc::get_mut` in-place mutation, clone fallback** (chosen for scenario C). Drop the
  persistent `ready_base` pre-capture; under the readiness write lock, try
  `Arc::get_mut(&mut *arc_in_cell)`; on success, mutate `ConventionProfile`/`co_changes` in place
  (no clone, no allocation of a second full index); on failure (any concurrent snapshot holds the
  Arc), fall back to today's `(**cur).clone()` path unchanged. *Pros:* no field-type change, so no
  public API risk at all; in the common case (no concurrent MCP/HTTP call mid-republish) the
  deep-clone peak disappears entirely rather than merely shrinking. *Cons:* the fallback path still
  deep-clones (same cost as today) whenever a tool call happens to be in flight at that instant —
  the fix's effectiveness is not deterministic, it is *contention-dependent*; must be measured, not
  assumed, and the plan's RED test must cover both the get_mut-succeeds and the falls-back cases.
- **Option D — do nothing.** Same cost/consequence as before: spec §4 unmet, +56-60% stands.

## Decision

**Option C plus the watcher decision. Option A was implemented, measured and rejected — see Outcome.** The watcher drops its seed and its long-lived `shared` mirror; per batch it takes its base from the readiness cell, publishes the result back, and releases its ref. Scenario C: restructure the background-build closure (serve.rs:2784-2834)
to stop holding `ready_base` as a persistent pre-capture; `republish_conventions` fetches its
fallback value from the cell itself only inside the branches that need one (mining panic, cell not
`Ready`/`ReadyEnriched`). Inside the retry loop, attempt `Arc::get_mut` on the cell's `Arc` under the
write lock first; apply the mined conventions/co-changes in place on success; fall back to the
existing clone-and-compare-and-swap on failure (`get_mut` returning `None`, i.e. `snapshot_ready_index`
or another reader holds a concurrent clone). Scenario U: bounded-batch fold in `build_index`
(Option A) was the original plan; it was rejected on measurement and reverted, so `build_index` is
unchanged from 3.5.0.

## Consequences

### Positive
- No `CodebaseIndex` field-type change anywhere — zero public API risk, satisfies the review that
  rejected decision 2.
- In the common case (idle server, no concurrent tool call at the exact republish instant), the
  scenario-C peak disappears rather than shrinking — mutation in place allocates nothing new for the
  untouched fields.
- Scenario U's fix is unchanged from the already-reviewed first revision; no new risk there.

### Negative
- Option C's benefit is contention-dependent: under sustained concurrent tool traffic,
  `snapshot_ready_index` may hold a clone at the exact moment of every republish attempt (all 3
  `ATTEMPTS`), and the fallback clone path is exactly today's cost — the fix helps the *median* case,
  not a worst-case guarantee. The plan's measurement (§4) must report both an idle-server run and a
  concurrent-tool-traffic run, not just the idle one, before claiming the gate is met.
- Restructuring the background-build closure to drop the `ready_base` pre-capture also changes the
  embeddings-phase gate condition (it currently gates on `ready_base: Option<Arc<..>>` being
  `Some`) — the plan's manifest must re-derive that gate from a cheap non-owning check (e.g. whether
  `next` published a `Ready`/`ReadyEnriched` variant) instead of the clone's presence, or the
  embeddings phase silently stops running.
- Two independent fixes in one patch release (Options A and C) touch two different functions; each
  needs its own RED test and its own measurement row, more surface than a single-fix release.

### Neutral
- `files: Vec<Arc<IndexedFile>>` and `embedding_index: Option<Arc<_>>` stay as they are — this
  decision does not touch per-field sharing, only whether the *whole-struct* clone happens at all.

## Revisit if

- Concurrent-tool-traffic measurement (§4) shows `Arc::get_mut` rarely succeeds in a realistic
  factory workload (frequent tool calls) — the contention-dependent benefit may not be enough on its
  own; reconsider a narrower, non-pub wrapper type for just `conventions`/`co_changes` (an
  internal-only newtype, not exposed through `core_graph`'s public field) as a follow-up ADR.
- The 1.10× full-lifecycle gate is not met on scenario U at any `build_index` batch size that also
  holds the +10% time gate — same fallback as the first revision: Option C from that revision
  (drop `content` from `Prepared`), a separate ADR given its wider blast radius.

## Outcome: bounded-batch fold tried and rejected (2026-10-18)

Option A was implemented and measured (`scripts/mcp-ready.py --lifecycle 20`, scriptorium, n=5,
interleaved). It did not lower the build-phase peak (scenario U: 374 MB against 3.5.0's 364 MB) and
added 11-16 % to `index_ready_s` (U 0.81 s against 0.73 s; C 3.26-3.38 s against 2.90 s). The
`artifact_files` content is resident for the whole build regardless, so chunking the transient
`Prepared` set saved nothing measurable. It was reverted; `build_index` is unchanged from 3.5.0.
Options C (in place, plus the watcher releasing its ref) stays. The measured post-ready peak in the
mining window led to a follow-up: saving the derived cache through a borrowed view
(`DerivedCacheRef`) instead of cloning graph, call graph, pagerank and conventions.

Measured result (`/usr/bin/time -l`, scriptorium, n=5 interleaved, medians, `benches/BASELINES.md`):
peak memory footprint meets the 3.4.0 × 1.10 gate in both scenarios (U 373 MB vs 423 MB; C 648 MB vs
692 MB). Max RSS meets it in scenario U (385 MB) but **not** in scenario C (570 MB against a 504 MB
limit). Against 3.5.0 the measured peak is unchanged on both metrics (U 373/385, C 646/568), so this
ADR's changes are not shown to lower the peak; the scenario-C max-RSS regression remains open.
