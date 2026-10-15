---
title: cxpak warm start Phase 2 — conventions off the Ready path, call graph from cached parse results
status: DRAFT
created: 2026-10-15
updated: 2026-10-15
target: cxpak 3.6.0 (MINOR — no public API change)
adrs: 0212
spec: specs/2026-10-15-warm-start-SPEC.md (Phase 1, §8 phasing)
---

# cxpak warm start Phase 2 — Spec

## 1. Problem (measured)

Phase 1 (ADR-0210, released as 3.5.0) made an *unchanged-repo* warm start on the scriptorium clone
(4,384 files) fall from 14.26 s to 1.85 s (`benches/BASELINES.md`, "MCP index-ready on real repos").
It did not touch the Phase 2 gate it defined: a **post-commit** warm start — every per-file artifact
hits, the derived cache misses because `HEAD` moved (ADR-0167 fingerprint) — which the same table
measures at **11.7–11.9 s** (median 11.79 s, n = 5), against the 6.0 s gate (spec Phase 1 §8). The
gate fired. This is the common case in a factory repo: a commit just landed, and the next session's
`cxpak serve --mcp` pays the full derived-build cost once more.

**Profile** (issue #173, same clone, `sample(1)`, ms):

| Work | ms |
|---|---|
| `build_convention_profile` | **5866**, of which git-history mining (`extract_git_health`'s revwalk + per-commit `diff_tree_to_tree`/`diff.foreach`) is 3849 |
| `finish_build` | 3610 |
| ↳ `build_call_graph` | 1523, of which the per-file tree-sitter **re-parse** (`extract_calls_by_symbol`) is 1464 |
| ↳ `build_dependency_graph` | 607 |
| `detect_cross_lang` | 162 |
| store load | ~80 |
| derived load | ~20 |

Conventions and the call graph together are 7.3 s of the 11.8 s — the two items this spec addresses.

## 2. Goal

A post-commit warm start — artifacts hit, derived cache misses — publishes MCP `Ready` without
waiting for conventions to be freshly mined, and without the call graph re-parsing files tree-sitter
already parsed for this build.

## 3. Non-goals

- Phase 3, the shared index server (ADR-0211).
- A wall-clock-driven expiry of git-history mining within a session (the existing 60 s TTL on
  `extract_git_health` governs that already and is unchanged).
- Changing `detect_cross_lang_edges`, `build_dependency_graph`, or the watcher's in-session
  incremental path (ADR-0200/0204/0205).
- A public API change, or a `CACHE_VERSION` bump. `ARTIFACT_STORE_VERSION` does bump (1 → 2).

## 4. Hard constraints (a violation is a defect)

| ID | Constraint |
|---|---|
| C1 Parity | Once background work settles, the published index equals a cold build on every Phase 1 C1 field, plus `conventions` and `call_graph` explicitly. |
| C2 Honest window | What a tool call sees in the window between `Ready` and the background republish is documented, not silently stale: a `ConventionsStatus` discriminator (`Fresh` / `StalePending { base_commit }`) is exposed on the index so a caller can tell. |
| C3 Fail-closed | A missing, corrupt, or version/grammar-mismatched artifact-store call-site entry, or a `load_for_delta` miss, gives a full recompute — never an indexing error. |
| C4 No API break | `build_call_graph`'s existing public signature is unchanged (ADR-0210 C7 precedent extends here). `ArtifactEntry`, `ArtifactStore` stay `pub(crate)`. |
| C5 Determinism | The republished conventions, once settled, are bit-for-bit what a cold build of the same `HEAD` produces. The call graph built from cached call sites is identical to one built from a fresh parse. |
| C6 Coverage | ≥ 90 % on new and changed code (tarpaulin CI), matching Phase 1 C8. |

## 5. Design

See ADR-0212 for the decision and rejected options. Summary of the chosen shape:

1. **Conventions stop blocking `Ready`.** On a derived miss, `Ready` publishes with conventions
   loaded via `DerivedCache::load_for_delta` (ignores fingerprint, validates grammar/version) from
   the *previous* on-disk derived cache — i.e. last commit's conventions — tagged
   `StalePending { base_commit }`. A background phase (reusing the existing snapshot-then-swap
   enrichment pattern at `spawn_mcp_index_build`, used today for embeddings) then runs the full
   `build_convention_profile`, and republishes `Ready` with `Fresh` conventions and the now-complete
   `DerivedCache` write. No prior derived cache (first start in the repo, or `load_for_delta` fails
   grammar/version) means conventions are mined inline before `Ready`, unchanged from today — a
   repo's first post-upgrade start pays once, as Phase 1's "Negative" already documents for artifacts.
2. **Call graph stops re-parsing.** `ArtifactEntry` gains
   `call_sites_by_symbol: Option<BTreeMap<String, Vec<String>>>` (`ARTIFACT_STORE_VERSION` 1 → 2,
   fail-closed on mismatch per C3, so a 3.5.0 store is a full miss once under 3.6.0 — same cost
   shape as the Phase 1 upgrade note). A new `pub(crate) fn build_call_graph_cached(index, cache:
   &HashMap<String, BTreeMap<String, Vec<String>>>) -> (CallGraph, HashMap<String,
   BTreeMap<String, Vec<String>>>)` consults the cache by `(relative_path, content_sha256)` instead
   of calling `extract_calls_by_symbol` for a hit, and returns the freshly computed map for misses so
   the caller writes it back into the artifact store. The existing public `build_call_graph` becomes
   a thin wrapper calling the cached variant with an empty cache (C4) — behaviour-identical to today
   for every existing caller (CLI, tests, the watcher's incremental path).

## 6. Acceptance

| # | Check | How |
|---|---|---|
| B1 | Post-commit warm start, scriptorium clone: median `index_ready_s` ≤ 6.0 s, n ≥ 5 | `scripts/mcp-ready.py` on `/tmp/frz/scriptorium` after `git commit --allow-empty` before each run |
| B2 | Parity: once the background republish lands, `conventions` and `call_graph` equal a cold build's | test: `warm_start_tests.rs` |
| B3 | Honest window: between `Ready` and republish, `ConventionsStatus::StalePending { base_commit }` is exposed and `base_commit` equals the derived cache's prior commit | test |
| B4 | First-ever start (no prior derived cache): conventions are mined inline, `ConventionsStatus::Fresh` from the first `Ready` | test |
| B5 | Call-graph cache hit: a sentinel entry in `call_sites_by_symbol` for an unchanged file is exposed in the built `CallGraph`, and a thread-local parse counter (test-only, mirrors Phase 1's T1.10) reads 0 for that file | test |
| B6 | Call-graph cache miss: a changed file's call sites are recomputed and written back to the store | test |
| B7 | `build_call_graph`'s public signature and behaviour are unchanged for every existing caller | existing call_graph.rs tests stay green unmodified |
| B8 | C3: a v1 artifact store, or a `call_sites_by_symbol: None` entry, is a per-file miss, not an error | test |
| B9 | C3: `load_for_delta` returning `None` (no prior cache, grammar mismatch) falls back to inline mining | test |
| B10 | Determinism: the republished `ConventionProfile` is identical across two independent cold builds of the same `HEAD` | test |
| B11 | Gates: `cargo fmt --check`, `clippy --all-targets --all-features -D warnings`, `cargo test --all-features`, `scripts/feature-matrix.sh`, tarpaulin ≥ 90 % | CI |

## 7. Risks

| Risk | Mitigation |
|---|---|
| A tool call reads `StalePending` conventions and acts on last commit's naming/testing/health signals | C2 makes the staleness explicit on the index; conventions are advisory telemetry (CLAUDE.md layer 4), never a control input, so a one-commit-old read is a known, bounded cost, not a correctness defect |
| The background republish races a second commit landing mid-recompute | The republish recomputes against the `HEAD` captured when the background phase started; if `HEAD` moved again, the *next* `Ready` publish (triggered by the watcher or the next start) is still correct — same tolerance Phase 1 already accepts for the artifact store's "concurrent writer" risk |
| `call_sites_by_symbol` grows the artifact store further (Phase 1 already flagged term-frequency growth as a Negative) | Measured in the same `BuildReport` store-load timing Phase 1 added; revisit via binary encoding if store load exceeds 25 % of warm `index_ready_s` (ADR-0210 "Revisit if", unchanged threshold) |
| `extract_calls_by_symbol`'s cache key must track language, not just path+hash, since a file's detected language can change across a rename | Cache key is `(relative_path, content_sha256, language)`; a language mismatch is a miss |
