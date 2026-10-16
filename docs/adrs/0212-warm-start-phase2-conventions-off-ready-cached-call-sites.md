---
id: '0212'
title: Warm start Phase 2 — conventions publish stale-then-republish off the Ready path; call graph from cached per-file call sites
status: ACCEPTED
date: 2026-10-15
triggered_by: specs/2026-10-15-warm-start-phase2-SPEC.md (cxpak#173; Phase 2 gate fired at 11.79 s median vs 6.0 s)
loop: planning
---

# ADR-0212: Warm start Phase 2 — conventions off the Ready path; call graph from cached call sites

## Context

ADR-0210 (Phase 1, 3.5.0) made an *unchanged* warm start fast by content-validating per-file
artifacts and skipping the derived passes on a derived-cache hit. It explicitly did not address a
**post-commit** warm start, where every per-file artifact still hits but `HEAD` moved, so the
derived fingerprint misses and `finish_build` runs in full. Measured on the scriptorium clone:
`build_convention_profile` 5.87 s (3.85 s of it is git-history mining — `extract_git_health`'s
revwalk + per-commit `diff_tree_to_tree`), `build_call_graph` 1.52 s (1.46 s is tree-sitter
re-parsing every file `finish_build` already parsed once, earlier in the same build, to produce
`file.parse_result`). The 6.0 s Phase 2 gate (spec Phase 1 §8) fired at 11.79 s median.

This is a human decision because the fix means either accepting a previous commit's conventions for
a window (a staleness trade visible to every MCP tool call) or taking on a second mined-history
cache with its own rebase/force-push correctness trap (ADR-0179 names exactly this trap for the
dependency-graph edge-delta: "a cache that is two commits behind, on another branch, post-rebase, or
whose base is unknown... silently produces a wrong [result]"). No code-level heuristic resolves that
trade; it has to be chosen and recorded.

## Options considered

### Conventions

- **Option A — incremental history mining cached by base commit.** Persist every mined commit
  record (changed files, timestamp, bugfix/revert flags) keyed by commit oid; on a new `HEAD`, walk
  only `revwalk.hide(old_head); revwalk.push_head()` (new commits only), merge into the cached
  record set, then recompute churn/bugfix/revert aggregates from the full in-window record cache —
  no further `diff_tree_to_tree` calls for commits already mined. *Pros:* conventions are always
  fresh at `Ready`; no visible staleness window. *Cons:* needs a new persisted cache format (none
  exists today at per-commit granularity — `DerivedCache` holds only the aggregated
  `ConventionProfile`); must detect a rewritten history (the `old_head` is no longer an ancestor of
  the new `HEAD`) and fail closed to a full remine, exactly the trap ADR-0179 called out for the
  dependency-graph delta; the 30/180-day windows are wall-clock-relative, so every build still has to
  re-walk the *aggregate* recompute over the full in-window record set even though no new
  `diff_tree_to_tree` runs — correct, but a second subsystem with its own fail-closed logic to
  review and keep honest alongside ADR-0179's existing one.
- **Option B — publish `Ready` with the previous snapshot's conventions, recompute in the
  background, republish (chosen).** `DerivedCache::load_for_delta` already exists for exactly this
  shape of read — it validates grammar/version and **not** fingerprint, built for ADR-0179's
  edge-delta. Reusing it for conventions costs no new cache format. The background phase reuses the
  snapshot-then-swap pattern `spawn_mcp_index_build` already runs for embeddings enrichment
  (`src/commands/serve.rs:2593`, "Extension point... a second background phase will enrich the same
  cell... without touching the handshake path"). *Pros:* no new persisted format, no rebase/rewrite
  detection to get right, reuses two already-reviewed primitives. *Cons:* a tool call in the window
  between `Ready` and the republish reads one-commit-old conventions; must be made visible (spec
  C2), not silently stale.
- **Option C — mine conventions off the hot path entirely, triggered by the post-commit git hook**
  (`cxpak hook post-commit`, which ADR-0179 says already re-mines conventions to keep the shared
  derived cache warm). *Pros:* by the time `serve --mcp` starts, the hook may have already primed a
  fresh derived cache, making this a non-issue for a repo with the hook installed. *Cons:* **not
  universal** — ADR-0210's Context already names "some harnesses kill an MCP server that is slow to
  start," and some sessions start `cxpak serve --mcp` in a repo or worktree that never ran the hook
  (a fresh clone, a harness without `conductr`'s `commit-msg`/`post-commit` wiring, cxpak used
  standalone). Rejected as the sole fix because it is contingent on an external, optional
  integration cxpak cannot assume; it may still help in practice and is unaffected by this decision.

### Call graph

- **Option D — status quo**, re-parse every file in `build_call_graph` via `extract_calls_by_symbol`.
  Rejected: this is the exact duplicate-work pattern Phase 1 fixed for the per-file index build (a
  file is parsed once for `file.parse_result`, then again here).
- **Option E — cache per-file call sites in the artifact store, keyed by content hash (chosen).**
  `ArtifactEntry` gains `call_sites_by_symbol: Option<BTreeMap<String, Vec<String>>>`, bumping
  `ARTIFACT_STORE_VERSION` 1 → 2 (fail-closed on mismatch, spec C3 — a pre-upgrade store becomes a
  full per-file miss once, matching the Phase 1 upgrade note). A new `pub(crate)
  build_call_graph_cached` consults it; the existing public `build_call_graph` wraps it with an
  empty cache, so no public signature changes (spec C4). *Pros:* no API break, same cache
  infrastructure Phase 1 already built and reviewed, parses each file exactly once per commit.
  *Cons:* one more field growing `artifacts.json` (Phase 1 already flagged term-frequency growth as
  a Negative; this is the same shape of cost, measured the same way).
- **Option F — cache the call graph itself (post-build), keyed by the derived fingerprint.** This is
  what the *derived* cache already does on an unchanged-repo hit (Phase 1, finish_build
  `precomputed`); it does not help here because the fingerprint, by construction, misses on every
  post-commit start — the whole point of this spec is the fingerprint-miss case. Rejected: solves a
  problem Phase 1 already solved, not this one.

## Decision

**B (conventions) + E (call graph).** `Ready` publishes with `DerivedCache::load_for_delta`'s
conventions **and `co_changes`** (one derived unit, persisted and restored together) tagged
`ConventionsStatus::StalePending { base_commit }` when a prior derived cache exists; a background phase mines fresh conventions and republishes `Fresh`. The Ready-time build path **never saves a
`DerivedCache` while conventions are `StalePending`**: only the background phase, after fresh
mining, saves the complete `DerivedCache` under the current fingerprint (an interrupted start must
not leave stale conventions on disk under this commit's fingerprint). Only the MCP start path
defers; all other build callers mine inline. The call graph is built
from `call_sites_by_symbol` cached per file in the artifact store (version 2), with the public
`build_call_graph` unchanged and a new `pub(crate)` cached variant underneath it.

## Consequences

### Positive
- The post-commit warm start no longer blocks `Ready` on git-history mining or a tree-sitter
  re-parse; both are either skipped (cache hit) or deferred (background republish).
- No new persisted cache format, no rebase/force-push detection code path to maintain alongside
  ADR-0179's.
- `ArtifactStore`'s existing content-validation (SHA-256, C3 fail-closed) extends to call sites with
  no new validity model to design.

### Negative
- A caller that reads conventions in the `StalePending` window sees the previous commit's
  naming/testing/health signals. Bounded to advisory telemetry (CLAUDE.md layer 4: verification is
  never control), but a real, user-visible trade, not a free lunch.
- `artifacts.json` grows again (call sites alongside term frequencies); store-load time is already
  instrumented in `BuildReport` and will be watched against the same 25 % threshold Phase 1 set.
- The Phase 2 fix does not help a repo whose derived cache has never been written (first-ever
  start): conventions mine inline there, same as today, so B4-class repos get no improvement from
  this spec — only subsequent post-commit starts do.
- Two background-enrichment phases now exist on the same readiness cell (embeddings, and now
  conventions); they must be sequenced or made independent so one's panic/failure cannot swallow the
  other's publish. (Design detail for the plan, not re-litigated here.)

### Neutral
- `detect_cross_lang_edges`'s sort order and `serde_json`'s `float_roundtrip` (Phase 1, ADR-0210)
  are unaffected.
- The watcher's in-session incremental path is unchanged; this spec only concerns process-start
  `Ready` publication.

## Revisit if

- A background republish measurably races a second rapid commit often enough in practice that the
  one-commit staleness window matters for a real consumer (not just a theoretical tool call) —
  reopen toward Option A's incremental mining.
- `call_sites_by_symbol` pushes store-load past the 25 % threshold ADR-0210 already set: move to a
  binary/streaming encoding for the whole artifact store, not just this field.
- `cxpak hook post-commit` becomes a universal precondition for `serve --mcp` starts (e.g. cxpak
  starts refusing to run without it): Option C's rejection no longer holds, and conventions mining
  could move fully off this path.
