---
id: '0179'
title: Edge-delta post-commit rebuild via base-SHA-validated derived cache
status: ACCEPTED
date: 2026-07-02
triggered_by: cxpak 3.0.0 Phase B Task B3d (scale / live-update speed)
loop: implementation
---

# ADR-0179: Edge-delta post-commit rebuild via base-SHA-validated derived cache

## Context

ADR-0178 shipped `cxpak hook post-commit`, which regenerates the committable
graph artifact (`.cxpak/graph.edges`) after each commit via a **full** graph
rebuild (parse-cache accelerated). The 3.0.0 plan mandates an **edge-delta**
rebuild so the post-commit cost is proportional to the change, not the repo
size — matching the warm-PageRank edge-delta path the in-process watcher already
uses (`serve::process_watcher_changes`, ADR-0165/0166).

The danger is exclusively **correctness**, not speed. An edge-delta applies a
commit's changed/removed file set onto a *prior* graph. It is correct **only if
the prior graph reflects exactly the tree state before this commit** — i.e. the
prior was built at `parent(HEAD)`. Applying a delta onto any other base (a cache
that is two commits behind, on another branch, post-rebase, or whose base is
unknown) silently produces a **wrong** graph that is then committed as the
canonical artifact. Because the artifact is a durable, committed, merge-driver-
consumed file, a silent corruption is far worse than a slow rebuild. This forces
a human decision about *how much validation* gates the delta, and *what to fall
back to* — the crux of the task.

A hard boundary carried over from ADR-0178: this is the cxpak **product** hook
that installs into a USER repo; it must never touch this repository's own dev
hooks/config, and every failure path must stay best-effort (exit 0).

## Options considered

- **Base-SHA-validated edge-delta with full-rebuild fallback (chosen):** persist
  the git HEAD SHA the derived cache was built at (`base_commit`) and apply the
  delta **only if** `base_commit == parent(HEAD)`; in every other case
  (missing/`None`/mismatched base, >1 commit behind, grammar/version bump, load
  error) fall back to a full rebuild. Reuses the parity-tested
  `rebuild_graph_delta` + `compute_pagerank_seeded` machinery verbatim, so the
  result is bit-identical to a full rebuild. Pro: the artifact is provably always
  byte-identical to a full rebuild; delta is a pure speed optimization that can
  never change output. Con: a discarded cold graph/PageRank in the delta branch
  (see below) and post-commit now mines conventions/co-changes to keep the shared
  cache a valid warm hit.
- **Trust the parse cache / mtime, no base SHA:** delta whenever a prior cache
  exists. Pro: simplest, fastest. Con: **rejected** — a cache written by an
  earlier `overview` two commits ago would be silently deltaed onto, corrupting
  the artifact. This is exactly the failure the task exists to prevent.
- **Reconstruct the parent(HEAD) tree from git and delta forward:** read every
  blob at `parent(HEAD)`, rebuild that index, apply the commit diff. Pro: a
  "textbook" delta with `apply_incremental_update` doing real work. Con:
  **rejected** — reading all parent blobs is O(repo), no faster than a full
  build, and adds a second index construction path to keep correct.
- **Post-commit-owned sidecar cache** (`{base_commit, graph}` only): a separate
  file just for the delta base. Pro: keeps the shared `DerivedCache` untouched.
  Con: **rejected** — a second cache to keep coherent, and it forfeits the free
  warm-cache-for-`overview` win that falls out of writing the shared cache. The
  brief permitted this only if reusing `DerivedCache` were "genuinely awkward";
  it is not — one `Option<String>` field suffices.

## Decision

Extend the shared `DerivedCache` (ADR-0167) with `base_commit: Option<String>`
(`#[serde(default)]`), bump `CACHE_VERSION` 5→6 (old caches fail-closed →
rebuilt once), and add a non-fingerprint-gated `load_for_delta` loader. In
`hook::build_artifact`, apply the edge-delta onto the cached prior graph **only
when** `base_commit == parent(HEAD)`; otherwise the freshly built full-tree
graph stands. Either path persists a fully-valid `DerivedCache` stamped
`base_commit = HEAD`. This supersedes ADR-0178's post-commit-incrementality
decision — ADR-0178's full rebuild is now the documented **fallback**, not the
default.

### Enforcement points (the invariant, in code)

- `DerivedCache.base_commit` — the git SHA the cache's content was built at;
  stamped at **every** write site (`serve::build_index_with_workspace` on a
  cache miss, and `hook::persist_derived_cache`), so an interleaved
  `overview`/`serve` between commits refreshes the base to the current HEAD (a
  still-valid base for the next commit's delta) rather than resetting it.
- `DerivedCache::load_for_delta` — validates `version` + `grammar_hash` but
  **not** `fingerprint` (the post-commit tree's fingerprint necessarily differs
  from the base's); base-SHA equality is the safety gate instead.
- `hook::build_artifact` — the single `match` on
  `(parent(HEAD), load_for_delta())` that yields `RebuildKind::Delta` only on an
  exact base match and `RebuildKind::Full` in every other case. `RebuildKind` is
  returned so tests assert which path ran (a silent full is a test failure).

### The "delta never changes output" guarantee

`rebuild_graph_delta` is bit-identical to a full `rebuild_graph` (ADR-0166) and
falls back internally to a full rebuild on any structural (add/remove) or schema
change; `compute_pagerank_seeded` reaches the same stationary distribution as a
cold start (tests/parity.rs). The delta branch resets `index.graph` to the
validated prior base and drives it forward, so the serialized artifact equals a
full rebuild of the HEAD tree in **all** cases — asserted directly by the
`*_equals_full` tests.

## Consequences

### Positive
- The committed artifact is always byte-identical to a full rebuild; delta is a
  provable speed-only optimization guarded by an exact base-SHA check.
- The corruption modes (stale/None/mismatched base, grammar/version drift) all
  fail closed to a full rebuild — the safest possible default.
- Post-commit now warms the full shared `DerivedCache` (graph + PageRank +
  conventions + co-changes) stamped at HEAD, so the developer's next `overview`
  / `auto_context` is a warm hit, and the next commit's delta has a valid base.
- Reuses the existing, parity-tested edge-delta + warm-PageRank machinery
  verbatim — no second, hand-rolled reconstruction path to keep correct.

### Negative
- In the delta branch the initial full graph/PageRank computed by
  `build_with_content` is discarded and recomputed via delta — wasted CPU
  bounded by one graph build. Removing it would require a deferred-derivation
  build variant that skips graph/PageRank inside `build_with_content`; that
  touches core index construction and was out of scope for this correctness-
  focused, single-commit change. Noted for a future optimization.
- Post-commit now mines conventions/co-changes (git history) to keep the shared
  cache a valid fingerprint hit. This is work the next `overview` would pay
  anyway (a HEAD move invalidates its fingerprint), so it is net-neutral across
  the workflow — merely front-loaded into the (best-effort, non-fatal) hook.
- The `CACHE_VERSION` bump invalidates all local derived caches once (expected,
  fail-closed).

### Neutral
- `base_commit` stores the full oid (hex), compared against
  `parent(HEAD).id()`; both are full oids, so the comparison never depends on
  abbreviation length.

## Revisit if

- Post-commit latency on large repos becomes a complaint: add the deferred-
  derivation build variant so the delta branch stops discarding a cold
  graph/PageRank, or drop the conventions/co-changes mining from the hook and
  accept a fingerprint miss on the next `overview`.
- `DerivedCache` gains a field whose validity depends on more than the HEAD SHA
  (e.g. per-branch or per-worktree state): the single `base_commit` anchor would
  no longer be sufficient to certify a delta base.
- A measured case shows `rebuild_graph_delta` diverging from a full rebuild:
  the "delta never changes output" guarantee — and this decision — would need to
  be reopened (the parity tests are the tripwire).
