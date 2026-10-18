---
id: '0213'
title: Arc-share CodebaseIndex's heavy fields so republish_conventions stops deep-cloning the whole index
status: PROPOSED
date: 2026-10-18
triggered_by: full-lifecycle measurement (specs/2026-10-18-peak-rss-regression-SPEC.md §1) showing
  the post-ready peak (374-382 MB, scenario C) exceeds the build-phase peak this ADR originally
  targeted (307-327 MB) and is the binding number vs 3.4.0 (239-243 MB, +56-60%)
loop: planning
supersedes-decision-of: '0213' (same ADR number; prior decision was bounded-batch-fold, scoped
  only to build_index's collect — see "Prior decision" below)
---

# ADR-0213: Arc-share `CodebaseIndex`'s heavy fields so `republish_conventions` stops deep-cloning the whole index

## Prior decision (superseded by this revision)

The original ADR-0213 chose bounded-batch chunking of `build_index`'s `files.par_iter().collect()`
(Option A below), reasoning from `benches/BASELINES.md`'s Ready-only measurement. A validation pass
instrumented `scripts/mcp-ready.py` to keep sampling RSS past Ready and found a **second, higher**
peak in the deferred-conventions background phase that the Ready-only gate cannot see, and that
occurs on a different code path (`republish_conventions`, not `build_index`). Bounded-batch-fold
does not touch that path at all. This revision keeps the number 0213 (the triggering problem is the
same regression) but changes the decision to target the now-measured dominant peak.

## Context

`republish_conventions` (`src/commands/serve.rs:2854-2932`) runs when a `StalePending` base defers
conventions mining past Ready (ADR-0212). Its retry loop clones the *entire* `CodebaseIndex` to
apply freshly-mined conventions/co-changes:

```rust
let mut next = (**cur).clone();       // serve.rs:2909
mined.clone().apply_to(&mut next);
```

`CodebaseIndex` (`src/core_graph/index.rs:27-58`) has one field already `Arc`-wrapped for exactly
this reason — `embedding_index: Option<Arc<EmbeddingIndex>>` (ADR-0205 / issue #47, comment at
lines 48-54: "cloning a `CodebaseIndex`... bumps a refcount instead of deep-copying"). Every other
heavy field is plain: `term_frequencies: HashMap<String, HashMap<String,u32>>`, `graph:
DependencyGraph`, `call_graph: CallGraph`, `pagerank: HashMap<String,f64>`, `conventions:
ConventionProfile`, `co_changes: Vec<CoChangeEdge>`. `files: Vec<Arc<IndexedFile>>` is already
cheap to clone (per-file `Arc`, not per-struct). So `(**cur).clone()` deep-copies every un-Arc'd
field while the old `Arc<CodebaseIndex>` (`cur`) is still alive and may still be read by an
in-flight caller — both copies resident at once is the measured peak (spec §2).

The fix must not regress `index_ready_s` beyond +10% vs 3.5.0 (per scenario), must not bump
`CACHE_VERSION`/`ARTIFACT_STORE_VERSION`, and should keep `cxpak::cache`'s public surface untouched.
It necessarily changes `core_graph::CodebaseIndex`'s field *types* (same names, `Arc`-wrapped) —
in-crate call sites (221 non-test field reads outside `index.rs`/`serve.rs`, by grep) keep compiling
unchanged through `Arc`'s auto-deref for field/method access; a site that *constructs* a
`CodebaseIndex` literal or moves a field out needs `Arc::new(..)`/`(*arc).clone()` at the
construction site, a mechanical, locally-contained change. Any out-of-crate reader of these fields
(none identified inside this repo — `core_graph` is `pub`, so this is a source-compat risk for
`conductr-core` or another external consumer, not verified here) is this ADR's open risk, not a
closed one; the plan's manifest includes a grep-based audit node before the field types change.

## Options considered

- **Option A — bounded-batch fold** (the prior decision). Chunks `build_index`'s collect to cap the
  *build-phase* resident set. *Pros:* smallest change, one function, no on-disk format change.
  *Cons:* does not touch `republish_conventions` at all — scenario C's measured dominant peak
  (374-382 MB) is untouched; only helps scenario U, which was never the larger regression once
  measured full-lifecycle (§1: U is +31-34%, C is +56-60%).
- **Option B — `Arc`-share `CodebaseIndex`'s heavy fields** (chosen). Wrap `term_frequencies`,
  `graph`, `call_graph`, `pagerank`, `conventions`, `co_changes` in `Arc` (same pattern as
  `embedding_index`, ADR-0205). `republish_conventions` then builds `next` by cloning the `Arc`
  pointers for every field it does not touch (refcount bump, not deep copy) and replacing
  `conventions`/`co_changes` with a freshly-allocated `Arc` — no full-struct deep copy, so the old
  and new "generations" of the index never both hold two full copies of the untouched fields.
  *Pros:* targets the measured dominant peak directly; follows an existing, reviewed precedent in
  the same struct; the MCP freshness watcher's own per-edit `CodebaseIndex::clone()` (the reason
  `embedding_index` was already `Arc`-wrapped) gets the same benefit for free. *Cons:* touches
  `CodebaseIndex`'s field types (mitigation: §Context's auto-deref argument + an audit node); a
  mutator that needs to *mutate in place* a field now behind `Arc` must `Arc::make_mut` or replace
  the `Arc` wholesale — `ConventionProfile::apply_to` (serve.rs:2910) is exactly such a mutator and
  needs its call site adjusted to build a fresh value and swap the `Arc`, not mutate through it.
- **Option C — drop `content` from `Prepared` once consumed**, re-reading from disk later. Same
  trade-off the prior ADR recorded: large, with unknown blast radius, and does not address the
  post-ready peak either (it is a `build_index`-phase fix, same limitation as Option A).
- **Option D — do nothing, document the regression.** Spec A3/§4 stays unmet; +56-60% on the
  larger, measured peak is worse than the number the prior ADR revision was written against.

## Decision

**Option B.** `Arc`-wrap `CodebaseIndex`'s `term_frequencies`, `graph`, `call_graph`, `pagerank`,
`conventions`, `co_changes`. Change `republish_conventions`'s clone-and-mutate (serve.rs:2909-2911)
to construct `next` from `cur`'s `Arc` fields directly (cheap) and compute the mined
conventions/co-changes into fresh values, assigned as new `Arc`s — no field of `next` ever holds a
freshly deep-copied value except the two actually changing. Keep Option A (bounded-batch-fold)
available as a follow-up, gated on whether §4's full-lifecycle measurement still fails on scenario U
after Option B ships — not applied speculatively.

## Consequences

### Positive
- Targets the measured dominant peak (scenario C, the binding full-lifecycle number) directly,
  rather than a peak that the full-lifecycle measurement shows is not the larger regression.
- Follows the repo's own precedent (ADR-0205) rather than introducing a new sharing strategy.
- Also shrinks the MCP freshness watcher's own existing per-edit-batch clone cost (any caller of
  `CodebaseIndex::clone()` benefits, not just `republish_conventions`).
- No on-disk format change, no `*_VERSION` bump: still ships as a patch (3.5.1).

### Negative
- Touches more of `CodebaseIndex` than Option A (six fields vs. zero) — larger diff, more call
  sites to mechanically adjust for the `Arc` wrapper (construction/move-out sites; reads are
  unaffected by auto-deref).
- `ConventionProfile::apply_to` and any other in-place mutator of a now-`Arc`-wrapped field must be
  changed from "mutate through the reference" to "build fresh, swap the `Arc`" — a real semantic
  change at each such call site, not just a type annotation.
- The build-phase peak (scenario U, the prior ADR's target) is not addressed by this decision; if
  §4's gate still fails there, Option A is the recorded fallback, not a silent gap.

### Neutral
- `files: Vec<Arc<IndexedFile>>` already uses this pattern at the per-file level; this decision
  extends it to the struct's own per-field level. No new sharing *mechanism* is introduced.

## Revisit if

- The 1.10× full-lifecycle RSS gate is not met on scenario U after Option B ships — pull in Option A
  (bounded-batch-fold) as a follow-up, measured against the gate, not guessed.
- An external (out-of-crate) consumer is found to construct or move fields out of `CodebaseIndex`
  directly — add accessor methods or a constructor function before the field-type change ships,
  rather than after a break is reported.
- `republish_conventions`'s retry loop (`ATTEMPTS = 3`) still shows a transient double-peak even
  after Option B, because the two *replaced* fields (`conventions`, `co_changes`) are themselves
  large enough to matter at `Arc::new` time — profile with `dhat`/jemalloc `stats.resident` rather
  than assuming they are small.
