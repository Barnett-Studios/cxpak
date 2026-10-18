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
- No public API change to `cxpak::cache`; no `CACHE_VERSION`/`ARTIFACT_STORE_VERSION` bump. (The
  chosen fix in ADR-0213 does change `core_graph::CodebaseIndex`'s field *types* — same field names,
  `Arc`-wrapped — for fields not already externally read through a stable accessor; §6/ADR-0213
  records why this is judged in-scope for a patch and what, if anything, needs an accessor to stay
  source-compatible for any out-of-crate reader.)

## 5. Non-goals

- A binary/streaming store encoding (ADR-0210's own "Revisit if" already covers that trigger
  separately — store *load time*, not RSS).
- Surfacing `ConventionsStatus` in a tool response (new API surface, needs its own decision).
- The shared index-server work (ADR-0211/0214; see the separate 2026-10-18 triad).
- Eliminating the build-phase peak beyond what the ADR-0213 fix yields as a side effect — if §4's
  gate is met without also chunking `build_index` (bounded-batch-fold), that chunking stays a
  follow-up, not a requirement of this patch (§1 shows it is not the binding peak in scenario C).

## 6. Approach

See ADR-0213 for the chosen design. Summary: `republish_conventions`'s clone-and-mutate
(serve.rs:2909) is O(repo) because most of `CodebaseIndex`'s heavy fields are plain (not `Arc`);
wrap them so a struct-level `.clone()` is a handful of refcount bumps, and replace `conventions`/
`co_changes` by assigning a new `Arc` rather than cloning the struct that holds them. Bounded-batch
chunking of `build_index`'s collect (the original plan's Option A) is retained only if §4's gate is
not met by the `Arc`-sharing fix alone, measured — not assumed necessary up front.

## 7. Risks

- `Arc`-wrapping fields read directly (not through an accessor) outside this crate is a
  source-compat break; ADR-0213 records the audit of call sites and the mitigation (accessor
  methods / `Deref`) if any exist.
- A chunk boundary for `build_index` (if still needed after measurement) that is too small loses
  rayon's parallel efficiency — chunk size would need measuring, not guessing.
- The small-n (n=2) numbers in §1 are triage evidence, not the gate measurement; §4's n ≥ 5
  interleaved run is required before claiming the gate is met.
