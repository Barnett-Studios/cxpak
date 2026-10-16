---
title: cxpak warm start Phase 2 — Implementation Plan
status: DRAFT
created: 2026-10-15
updated: 2026-10-15
spec: specs/2026-10-15-warm-start-phase2-SPEC.md
adrs: 0212
complexity: Medium (one new store field, one background enrichment phase, one cached-variant function)
risk: Medium (staleness-window correctness; two background phases on one readiness cell)
---

# Plan: cxpak warm start Phase 2

## Executive summary

Stop the post-commit warm start from blocking `Ready` on git-history mining and a tree-sitter
re-parse. Two changes:
- conventions publish via `DerivedCache::load_for_delta` (stale, tagged) at `Ready`, then recompute
  and republish in the background, reusing the embeddings-enrichment pattern already in
  `spawn_mcp_index_build`;
- the call graph is built from `call_sites_by_symbol`, a new field in the Phase 1 artifact store
  (`ARTIFACT_STORE_VERSION` 1 → 2), with the public `build_call_graph` wrapping an unchanged-behaviour
  cached variant.

No public API change, no `CACHE_VERSION` bump. One PR (`perf/warm-start-p2`), released as 3.6.0.

## Gates

| Gate | Rule |
|---|---|
| TDD | All red tests committed in one commit before any implementation commit. Each is red for its stated reason. |
| Spec constraints | C1–C6 (spec §4). |
| No new deps | Everything needed (`git2`, `serde_json`, `sha2`) is already in the tree. |
| Lint and tests | `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`; `cargo test --all-features`; `bash scripts/feature-matrix.sh` |
| Coverage | ≥ 90 % (tarpaulin CI). |
| Docs with code | ADR-0212 INDEX row, `benches/BASELINES.md` Phase 2 section, README note — same PR. |

## Current state (verified on `perf/warm-start-p2` @ worktree HEAD, post-3.5.0)

| File | Today | Change |
|---|---|---|
| `src/conventions/git_health.rs:15-56` | `extract_git_health` always walks history inline | unchanged in itself — called later (background), not fixed |
| `src/conventions/mod.rs:30-44` | `build_convention_profile` always runs before `Ready` on a derived miss | caller in serve.rs changes, not this fn |
| `src/cache/mod.rs:409-503` | `DerivedCache`, `load`, `load_for_delta` | no change — reused as-is |
| `src/cache/artifacts.rs:1-60` | `ArtifactEntry` (v1): hash, mtime, size, language, parse_result, token_count, term_frequencies | T2: new `call_sites_by_symbol` field, version 2 |
| `src/intelligence/call_graph.rs:745` | `pub fn build_call_graph(index)` re-parses every file via `extract_calls_by_symbol` | T3: wraps new cached variant |
| `src/commands/serve.rs:2596-2630` | `spawn_mcp_index_build` — one background enrichment phase (embeddings) after `Ready` | T4: a second phase (conventions), sequenced after/independent of embeddings |
| `src/commands/serve.rs:319-428` (`build_index_pending`/`build_index_with_workspace_reported`) | derived miss runs `build_convention_profile` inline, builds `BuildReport` | T4: on a derived miss with a loadable prior `DerivedCache`, defer conventions; otherwise unchanged |
| new `ConventionsStatus` (core_graph::conventions or index/mod.rs) | — | T1 |

## Phase 1 — tests first (one commit, all red)

Extends `warm_repo()` (`src/commands/warm_start_tests.rs`) with a second commit helper
(`commit_all` already exists) so a test can build at commit 1, then commit again and rebuild.

| Test | File | Asserts | Red today because |
|---|---|---|---|
| P2.1 `conventions_status_fresh_on_first_build` | warm_start_tests.rs | first-ever build (no prior derived cache): `index.conventions_status == Fresh` | `ConventionsStatus` does not exist (compile red) |
| P2.2 `conventions_status_stale_pending_after_commit` | warm_start_tests.rs | build at commit 1 (derived cache written), commit again, rebuild: immediately after `Ready`, `conventions_status == StalePending { base_commit: <commit 1 sha> }` and `index.conventions` equals commit 1's | same |
| P2.3 `conventions_republish_matches_cold` | warm_start_tests.rs | after the background phase joins (test calls the background fn directly, synchronously, rather than via the thread spawn), `conventions_status == Fresh` and `index.conventions` equals a fresh cold build at commit 2 | same |
| P2.4 `conventions_inline_when_no_prior_cache` | warm_start_tests.rs | derived cache absent (fresh repo) and a derived miss (e.g. grammar bump simulated): conventions are `Fresh` at `Ready`, no background phase runs | same |
| P2.5 `conventions_load_for_delta_grammar_mismatch_falls_back_inline` | warm_start_tests.rs | a `derived.json` with a mismatched `grammar_hash`: `load_for_delta` returns `None`, conventions mine inline, `Fresh` at `Ready` | same |
| P2.6 `artifact_store_v2_rejects_v1` | cache/artifacts.rs | a v1-shaped `artifacts.json` loads as empty (every field `None`) under `ARTIFACT_STORE_VERSION = 2` | version is still 1 |
| P2.7 `call_sites_cache_hit_skips_reparse` | warm_start_tests.rs | unchanged `src/a.rs` across two builds: a `#[cfg(test)]` thread-local parse counter in `extract_calls_by_symbol` reads 0 for `src/a.rs` on the second build's `build_call_graph_cached` call; `CallGraph` output is identical to the first build's | `build_call_graph_cached` does not exist (compile red) |
| P2.8 `call_sites_cache_miss_on_content_change` | warm_start_tests.rs | `src/a.rs` edited (new fn body): the counter reads ≥ 1 for `src/a.rs`; the stored `call_sites_by_symbol` entry for `src/a.rs` is overwritten with the new call sites | same |
| P2.9 `call_sites_cache_miss_on_language_change` | warm_start_tests.rs | a cached entry's `language` differs from the current build's detected language for that path: treated as a miss, not a false hit | same |
| P2.12 `stale_window_carries_prior_co_changes` | warm_start_tests.rs | in the `StalePending` window `index.co_changes` equals the prior snapshot's and is non-empty | `ConventionsStatus` does not exist (compile red) |
| P2.13 `stale_build_never_saves_derived_under_current_fingerprint` | warm_start_tests.rs | after a `StalePending` build whose background phase is not run, `DerivedCache::load(dir, current_fp)` is `None` | same |
| P2.14 `cold_builds_are_deterministic` | warm_start_tests.rs | two independent cold builds of the same `HEAD` give identical `conventions` and `co_changes` (B10) | compile red (shares the new fixture helpers) |
| P2.15 `call_sites_none_and_v1_store_are_per_file_misses` | warm_start_tests.rs | an entry with `call_sites_by_symbol: None` is recomputed and written back; a v1 store is a full miss, not an error (B8) | same |
| P2.10 `build_call_graph_wrapper_matches_cached_empty` | call_graph.rs | `build_call_graph(index)` output equals `build_call_graph_cached(index, &HashMap::new()).0` on the existing call_graph.rs fixtures (all four existing tests' indexes) | `build_call_graph_cached` does not exist |
| P2.11 `post_commit_gate_measurement` | not a unit test — recorded in BASELINES, gated by B1 | — | — |

The `ConventionsStatus` fields used by tests: `Fresh`, `StalePending { base_commit: String }`.

## Phase 2 — implementation (each task commits green for its tests)

- **T1. `ConventionsStatus`.** Add to `core_graph::conventions` (alongside `ConventionProfile`, same
  de-cycle module as Phase 1's re-exports) a `#[derive(Debug, Clone, PartialEq, Serialize,
  Deserialize)] pub enum ConventionsStatus { Fresh, StalePending { base_commit: String } }`, and a
  `conventions_status: ConventionsStatus` field on `CodebaseIndex` (default `Fresh` via `#[serde(default = "...")]` so existing serialized indexes/tests deserialize unaffected — C7-style, no break).
  Turns P2.1 green (others still red pending T4).
- **T2. Artifact store v2.** `src/cache/artifacts.rs`: `ARTIFACT_STORE_VERSION = 2`; `ArtifactEntry`
  gains `call_sites_by_symbol: Option<BTreeMap<String, Vec<String>>>`. `ArtifactStore::read`'s
  version check already fails closed to empty (Phase 1 T1 behaviour) — no new logic needed there,
  only the constant bump and the field. Turns P2.6 green.
- **T3. `build_call_graph_cached`.** `src/intelligence/call_graph.rs`: extract the body of
  `build_call_graph` into
  `pub(crate) fn build_call_graph_cached(index: &CodebaseIndex, cache: &HashMap<String, BTreeMap<String, Vec<String>>>) -> (CallGraph, HashMap<String, BTreeMap<String, Vec<String>>>)`
  keyed by `relative_path` (spec §5) — the caller (`build_index_pending`) admits an entry into the
  cache only after its `content_sha256` and `language` matched the bytes just read (the existing
  `matching` filter, so a language change is a whole-entry miss, P2.9), and writes computed call
  sites back only if the store entry's sha still matches. For each file: cache hit → reuse `call_map`, no
  `extract_calls_by_symbol` call; miss → call it, record the result for the caller to write back.
  `pub fn build_call_graph(index)` becomes `build_call_graph_cached(index, &HashMap::new()).0`.
  Turns P2.7, P2.8, P2.9, P2.10 green.
- **T4. Deferred conventions in `serve.rs`.**
  - `build_index_pending` gains a deferring sibling used only by `spawn_mcp_index_build`
    (`build_index_pending_deferred`); all other callers stay inline. On a derived-cache miss it
    attempts `DerivedCache::load_for_delta(ns_dir)`. On `Some(prior)`: use `prior.conventions` **and
    `prior.co_changes`** (one derived unit),
    `conventions_status = StalePending { base_commit: prior.base_commit.unwrap_or_default() }`,
    skip the inline `build_convention_profile` call **and do not save a `DerivedCache`** (the background
    phase saves the complete one under the current fingerprint). On `None`: today's inline mining,
    `conventions_status = Fresh`.
  - `spawn_mcp_index_build`: after publishing `Ready`, if the published index carries
    `StalePending`, spawn (or extend the existing embeddings thread-spawn site) a second phase that
    runs `build_convention_profile`, builds a fresh `DerivedCache` entry with the now-complete
    conventions, saves it, and republishes the readiness cell with `conventions_status: Fresh` and
    the updated index — same snapshot-then-swap write-lock discipline as the embeddings phase, and
    sequenced so one phase's panic (caught via the existing `catch_unwind`) cannot block the other's
    publish.
  Turns P2.2, P2.3, P2.4, P2.5 green.
- **T5. Benchmark + gate.** `scripts/mcp-ready.py` on `/tmp/frz/scriptorium`, `git commit
  --allow-empty` before each of n ≥ 5 runs, old (3.5.0) vs new binary, interleaved. Record in
  `benches/BASELINES.md` under a new "Phase 2 gate result" subsection. Turns B1 into a recorded
  number, not a claim.

## Phase 3 — docs, measurement, gate

- **T6.**
  - `docs/adrs/INDEX.md`: row for 0212.
  - README: extend the Phase 1 "Persistence" note — `call_sites_by_symbol` is part of the same
    content-validated store; a `StalePending` conventions status self-heals on the next `Ready`
    republish, no action needed.
  - `benches/BASELINES.md`: Phase 2 gate result (median, min–max, n, load average).
- **T7.** If B1 fails, say so in the PR body; do not merge on a claim (same rule Phase 1's T10 set).

## Risks

See spec §7. Additional:
- **T4 sequencing.** Two background phases (embeddings, conventions) on one `SharedReadiness` cell.
  Each must take and release the write lock independently so neither phase's duration gates the
  other's publish; a slow conventions remine must not delay an embeddings-ready flip that has
  nothing to do with conventions.
- **T3 cache key.** Keying on `(relative_path, content_sha256)` alone (dropping the language guard)
  would let a renamed-extension file with coincidentally identical bytes falsely hit under the old
  language's call/func node kinds — P2.9 exists specifically to keep this guard in place under
  refactor pressure.

## Execution manifest

Local fleet availability unchanged from Phase 1 (down, ~10 GB RAM needed on a machine with ~0.1 GB
free); `local: true` nodes fail open to the coder. Accept commands use `set -o pipefail` and require
at least one passing test.

```json
{
  "execution-manifest": [
    {"id": "conventions-status-type", "files": ["src/core_graph/conventions.rs", "src/index/mod.rs"], "change": "Add ConventionsStatus enum (Fresh / StalePending{base_commit}) and a defaulted conventions_status field on CodebaseIndex.", "accept": "set -o pipefail; cargo test --lib conventions_status_fresh_on_first_build 2>&1 | grep -E 'test result: ok\\. [1-9]'", "forbid": ["new_deps"], "local": true},
    {"id": "artifact-store-v2", "files": ["src/cache/artifacts.rs"], "change": "Bump ARTIFACT_STORE_VERSION to 2; add call_sites_by_symbol: Option<BTreeMap<String, Vec<String>>> to ArtifactEntry.", "accept": "set -o pipefail; cargo test --lib artifact_store_v2_rejects_v1 2>&1 | grep -E 'test result: ok\\. [1-9]'", "forbid": ["new_deps"], "local": true},
    {"id": "call-graph-cached", "files": ["src/intelligence/call_graph.rs"], "change": "Extract build_call_graph into build_call_graph_cached(index, cache) keyed by relative_path with caller-validated entries (sha and language checked in build_index_pending); build_call_graph wraps it with an empty cache.", "accept": "set -o pipefail; cargo test --lib call_graph:: 2>&1 | grep -E 'test result: ok\\. [1-9]'", "forbid": ["new_deps"], "local": false},
    {"id": "deferred-conventions", "files": ["src/commands/serve.rs"], "change": "On a derived miss, try DerivedCache::load_for_delta for conventions (StalePending); otherwise mine inline (Fresh). Background phase on spawn_mcp_index_build remines and republishes Fresh, sequenced independently of the embeddings phase.", "accept": "set -o pipefail; cargo test --lib warm_start_tests 2>&1 | grep -E 'test result: ok\\. [1-9]'", "local": false},
    {"id": "phase2-gate-measurement", "files": ["benches/BASELINES.md"], "change": "Record the post-commit warm-start gate measurement (scripts/mcp-ready.py, n>=5, git commit --allow-empty before each run) against the 6.0s target.", "accept": "test -f benches/BASELINES.md && grep -q 'Phase 2 gate result' benches/BASELINES.md && echo ok", "local": false}
  ]
}
```

## Success criteria

B1–B11 are met and recorded. The PR is reviewed by a non-author agent. CI is green. 3.6.0 is
released and the downstream pins are bumped, following Phase 1's rollout steps.
