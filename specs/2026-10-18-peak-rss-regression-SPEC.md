---
title: cxpak 3.5.1 — peak RSS regression on warm start, and the stale ConventionsStatus doc claim
status: DRAFT
created: 2026-10-18
updated: 2026-10-18
target: cxpak 3.5.1 (PATCH — no public API change)
adrs: 0213
spec: specs/2026-10-15-warm-start-SPEC.md, specs/2026-10-15-warm-start-phase2-SPEC.md
---

# cxpak 3.5.1 — Spec

## 1. Problem (measured, full lifecycle)

`benches/BASELINES.md` measured 3.5.0 vs 3.4.0 up to "MCP index ready" only. A validation pass
(ps RSS @100 ms, `scripts/mcp-ready.py`, kept alive past Ready) found that gate blind to a **second,
higher** RSS peak in 3.5.0's background phase (conventions/co-change mining deferred past Ready,
`src/commands/serve.rs:2795-2824`). The script's original `--lifecycle`-less mode exits at Ready and
cannot see it.

`scripts/mcp-ready.py --lifecycle 20` on `/tmp/frz/scriptorium` (4,384 files), two scenarios, n=2
(small-n triage; the gate in §4 requires n ≥ 5 before this fix ships):

- **U** — warm, unchanged repo (derived cache hit, conventions not stale).
- **C** — warm, right after an empty commit (derived cache hit on content, but HEAD moved → base is
  `StalePending`; triggers the ADR-0212 deferred conventions remine + `republish_conventions`).

| Scenario | Binary | build_peak_mb | post_ready_peak_mb | **peak_mb (full lifecycle)** | index_ready_s |
|---|---|---|---|---|---|
| U | 3.4.0 | 229–237 | 169 | **229–237** | 19.7–22.7 |
| U | 3.5.1 (pre-fix) | 303–307 | 220–231 | **303–307** | 1.4–3.9 |
| C | 3.4.0 | 239–243 | 135–140 | **239–243** | 30.5–34.1 |
| C | 3.5.1 (pre-fix) | 307–327 | **374–382** | **374–382** | 5.2–7.5 |

3.4.0 mines conventions synchronously inside the build (no deferred phase, hence no second peak and
a much larger `index_ready_s`). 3.5.1-pre-fix is faster to Ready but:
- Scenario U: dominant peak is still the build phase (matches §2's original analysis), +31–34%
  over 3.4.0.
- Scenario C: the **post-ready** peak (374–382 MB, at t≈16–18s) exceeds the build peak and is the
  **overall** peak, +56–60% over 3.4.0 — a larger regression than U, and on the gate's own terms
  (§4) this is the binding number, not the build-phase one §2 originally diagnosed.

## 2. Where each peak comes from (code-path analysis)

**Build-phase peak** (both scenarios, dominant in U): `build_index` (`src/commands/serve.rs:440-560`)
runs `files.par_iter().zip(stored.into_par_iter()).map(..).collect()` into
`prepared: Vec<Option<Prepared>>` — an O(repo) resident set (every file's raw content plus its
parsed artifacts, held for all files simultaneously) before folding into the index and store. 3.5.0
(ADR-0210, ADR-0212) added two per-entry maps (`term_frequencies`, `call_sites_by_symbol`) to the
artifact store that 3.4.0 did not carry — part of this peak's measured delta over 3.4.0.

**Post-ready peak** (scenario C, dominant overall): `republish_conventions`
(`src/commands/serve.rs:2854-2932`), reached only when a `StalePending` base deferred conventions
mining. Line `let mut next = (**cur).clone();` (2909) deep-clones the **entire** `CodebaseIndex` —
`term_frequencies: HashMap<String, HashMap<String,u32>>`, `graph: DependencyGraph`,
`call_graph: CallGraph`, `pagerank: HashMap<String,f64>`, `conventions`, `co_changes` — none of
which, except `files: Vec<Arc<IndexedFile>>` and (behind `embeddings`) `embedding_index`, are
`Arc`-wrapped (`src/core_graph/index.rs:27-58`). `embedding_index` was already wrapped in `Arc` for
exactly this reason (ADR-0205 / issue #47: avoid an O(repo) deep copy on every `CodebaseIndex::clone`
the MCP freshness watcher performs). The old, still-published `Arc<CodebaseIndex>` (`cur`, held by
any in-flight reader) stays alive while `next` is built, so the resident set briefly holds both the
old and the new deep copy of every un-Arc'd heavy field — the measured second peak. This happens on
every `StalePending` republish (scenario C), once per attempt (up to 3, `ATTEMPTS` at line 2872) if
the compare-and-swap loses the race to a concurrent watcher edit.

This is the dominant, measured driver of the full-lifecycle regression — larger than the build-phase
peak §2's prior analysis targeted, and present only when conventions are stale (scenario C), which a
`--lifecycle`-less gate at Ready cannot see because `republish_conventions` runs after Ready
publishes (serve.rs:2792 publishes `next`, 2808 republishes afterward).

## 3. Doc claim (`src/core_graph/conventions.rs:78-83`)

`ConventionsStatus`'s doc comment claims the stale-conventions window (ADR-0212) is "visible here
instead of silently stale." Grepping every reference to `conventions_status`/`ConventionsStatus`
outside its own definition and tests finds it read internally (serve.rs:2802, 2888-2904) but never
serialized into any MCP tool response. No tool output exposes `StalePending` today — the doc's claim
is false as written. Fix: reword the comment (out of scope: surfacing it in a tool's
`structuredContent`, new API surface with its own backward-compat question).

## 4. Goal / acceptance

- **Full-lifecycle** peak RSS (`scripts/mcp-ready.py --lifecycle N`, `peak_mb` — the max of
  `build_peak_mb` and `post_ready_peak_mb`, not just the value at Ready) on the scriptorium clone,
  scenarios U and C, each ≤ 3.4.0's full-lifecycle `peak_mb` × 1.10 (n ≥ 5, median, runs of 3.4.0
  and the fix interleaved). `N` must be long enough to observe the post-ready phase settle (empirically
  ≥ 20 s on scriptorium; re-measure the margin if it doesn't hold at the chosen fix).
- `index_ready_s` no worse than 3.5.0's measured figure + 10% for the same scenario (does not need to
  improve — the fix targets RSS, not latency).
- Every existing `warm_start_tests.rs` assertion stays green (parity with 3.5.0 behavior otherwise).
- `ConventionsStatus`'s doc comment no longer claims externally-visible status without a tool surface
  exposing it.
- No public API change anywhere: `core_graph::CodebaseIndex`'s field set *and* field types stay
  exactly as published in 3.5.0; no `cxpak::cache` change; no `CACHE_VERSION`/`ARTIFACT_STORE_VERSION`
  bump. (An earlier revision of ADR-0213 proposed `Arc`-wrapping some `CodebaseIndex` fields — a
  semver-visible type change on a `pub` struct in a `pub mod` — and was rejected on review for
  exactly that reason, same rule applied to PR #175. The current ADR-0213 fixes both scenarios
  without touching any field's type.)

## 5. Non-goals

- A binary/streaming store encoding (ADR-0210's own "Revisit if" already covers that trigger
  separately — store *load time*, not RSS).
- Surfacing `ConventionsStatus` in a tool response (new API surface, needs its own decision).
- The shared index-server work (ADR-0211/0214; see the separate 2026-10-18 triad).
- A narrower non-pub sharing mechanism for `conventions`/`co_changes` (ADR-0213's "Revisit if") —
  pulled in only if the chosen fix's own measurement shows `Arc::get_mut` rarely succeeding under
  realistic concurrent tool traffic.

## 6. Approach

See ADR-0213 for the chosen design (two independent, no-API-change fixes):

- **Scenario C** (post-ready peak, dominant overall): `republish_conventions`'s clone-and-mutate
  (serve.rs:2909) runs unconditionally today. The fix stops the background-build closure from
  holding a persistent extra clone of the published index (`ready_base`, serve.rs:2785) across the
  republish window, then has `republish_conventions` attempt `Arc::get_mut` on the cell's own `Arc`
  under the write lock — mutating `conventions`/`co_changes` in place with zero clone when no
  concurrent MCP/HTTP tool call holds a snapshot (`snapshot_ready_index`, serve.rs:3263) — and falls
  back to today's deep-clone-and-swap only when `get_mut` fails. No field of `CodebaseIndex` changes
  type.
- **Scenario U** (build-phase peak): bounded-batch chunking of `build_index`'s whole-repo
  `par_iter().collect()` (the first ADR-0213 revision's Option A, unchanged) — caps the resident
  `Prepared` set at O(batch) instead of O(repo).

## 7. Risks

- Scenario C's fix is **contention-dependent**: `Arc::get_mut` only succeeds when no concurrent
  tool call holds a snapshot at that instant. §4's measurement must include a concurrent-tool-traffic
  run, not only an idle-server run, before claiming the gate is met (ADR-0213 Consequences).
- Dropping the `ready_base` pre-capture changes the embeddings-phase trigger condition (today gated
  on that clone's presence) — must be re-derived from a non-owning check or the embeddings phase
  silently stops running (ADR-0213 Negative consequences).
- A chunk boundary for `build_index` that is too small loses rayon's parallel efficiency — chunk
  size needs measuring, not guessing.
- The small-n (n=2) numbers in §1 are triage evidence from the pre-fix binary, not the gate
  measurement; §4's n ≥ 5 interleaved run, on both the idle and concurrent-traffic workloads, is
  required before claiming the gate is met.
