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

## 1. Problem (measured)

`benches/BASELINES.md` ("MCP index-ready on real repos") measured 3.5.0 against 3.4.0 on the
scriptorium clone (4,384 files), warm (unchanged-repo) start, store primed:

| Metric | 3.4.0 | 3.5.0 |
|---|---|---|
| peak `phys_footprint` | 445 MB | 383 MB |
| max RSS | 273–280 MB | 316–396 MB |

`phys_footprint` improved; max RSS regressed +15–45%. Spec A3 of the ADR-0210 warm-start work was
written against peak RSS (≤ +10%) and is not met on that metric.

A single re-run this session on the same frozen clone (`/tmp/frz/scriptorium`, installed 3.5.0,
`scripts/mcp-ready.py`) confirms the order of magnitude: `index_ready_s=2.83 peak_rss_mb=385`,
consistent with the BASELINES range.

## 2. Where the peak comes from (code-path analysis, not an allocator trace)

`.cxpak/cache/artifacts.json` on this clone is 57 MB. `ArtifactStore::read`
(`src/cache/artifacts.rs:60-73`) streams the *input* bytes (`BufReader::with_capacity(1 << 16, ..)`)
but `serde_json::from_reader` still materializes the *entire output* object graph in one call: a
`BTreeMap<String, ArtifactEntry>` with one entry per file, each entry carrying
`parse_result: Option<ParseResult>`, `term_frequencies: Option<BTreeMap<String, u32>>` (ADR-0210) and
`call_sites_by_symbol: Option<BTreeMap<String, Vec<String>>>` (ADR-0212, new in 3.5.0 — 3.4.0 had
neither map). Both maps are new cost 3.4.0 never carried.

`build_index` (`src/commands/serve.rs:440-560`) then:
1. Loads the store whole (`ArtifactStore::load`, line 454) and `remove`s each file's entry into a
   `Vec<Option<ArtifactEntry>>` (`stored`, lines 457-460) — not cloned, so this step alone does not
   double the store's memory.
2. Runs `files.par_iter().zip(stored.into_par_iter()).map(..).collect()` (lines 471-554) into
   `prepared: Vec<Option<Prepared>>`. Every file's **full raw content** (`content`, read at line
   483, retained in `Prepared.content` at line 540-543) is read from disk and held **for every file
   in the repo simultaneously**, regardless of cache hit or miss — a hit still needs `content` to
   compute `raw_sha`. Each `Prepared` also carries that file's `FileArtifacts` (parse result +
   term_frequencies) and, on a Partial/Miss, a second `ArtifactEntry` (`fresh`, line 518/534) that
   duplicates some of the same term-frequency data the `artifacts` field already holds, destined for
   `changed` (line 558/577).
3. Only after the full `.collect()` does per-file data fold into `CodebaseIndex`'s own term-frequency
   and parse-result structures — a third representation of the same information.

So on a repo-wide scan, the **O(repo) resident set** at the collect boundary is: the whole source
tree's text (step 2) + that tree's parsed artifacts in `Prepared` form (step 2) + whatever the
now-partially-drained `ArtifactStore` the loop over `stored` has not yet freed. 3.5.0 added two maps
per entry (step 1) and the store was already O(repo) in 3.4.0's `cache.json` — the delta is
dominated by step 1's two new maps, not by step 2, which existed in equivalent form in 3.4.0's own
`parse_with_cache` path. Ruled out: jemalloc retention (`Cargo.toml:109-125`, decay-tuned) — decay
needs idle time to purge and this is measured mid-build, on the first and only pass, so retention
from a prior session cannot explain an in-process peak.

This analysis is from reading the code path, not from an allocator profiler (no `dhat`/`heaptrack`
run — out of this planning pass's budget). If the chosen fix's measurement (§4) does not clear gate
A, profile with `dhat` or jemalloc's `stats.resident` before trying a second design.

## 3. Doc claim (`src/core_graph/conventions.rs:78-83`)

`ConventionsStatus`'s doc comment claims the stale-conventions window (ADR-0212) is "visible here
instead of silently stale." Grepping every reference to `conventions_status`/`ConventionsStatus`
outside its own definition and tests (`src/commands/serve.rs`, `src/commands/warm_start_tests.rs`)
finds it read internally (serve.rs:2802, 2888-2904) but never serialized into any MCP tool response.
No tool output exposes `StalePending` today — the doc's claim is false as written. Fix: reword the
comment (smaller, correct change; surfacing it in a tool's `structuredContent` is a real option but
is new API surface with its own backward-compat question, out of scope for a patch release).

## 4. Goal / acceptance

- Peak RSS on the scriptorium clone (warm, store primed) ≤ 3.4.0 baseline × 1.10 (n ≥ 5, median,
  runs of 3.4.0 and the fix interleaved, `scripts/mcp-ready.py`).
- `index_ready_s` does not regress beyond +10% vs 3.5.0's measured warm figure (baseline 0.85 s per
  the Phase 2 table's "warm, unchanged" row; use the same interleaved methodology).
- Every existing `warm_start_tests.rs` assertion stays green (parity with 3.5.0 behavior otherwise).
- `ConventionsStatus`'s doc comment no longer claims externally-visible status without a tool surface
  exposing it.
- No public API change (`cxpak::cache`, `cxpak::core_graph` field sets unchanged); no `CACHE_VERSION`
  or `ARTIFACT_STORE_VERSION` bump.

## 5. Non-goals

- A binary/streaming store encoding (ADR-0210's own "Revisit if" already covers that trigger
  separately — store *load time*, not RSS. Not pulled in here unless §6's chosen fix needs it).
- Surfacing `ConventionsStatus` in a tool response (new API surface, needs its own decision).
- The shared index-server work (ADR-0211; see the separate 2026-10-18 triad).

## 6. Approach

See ADR-0213 for the chosen design. Summary: bound the parallel collect in `build_index` to
fixed-size chunks (fold each chunk's `Prepared` entries into the running index/store state before
starting the next chunk) so the O(repo) resident set at §2 step 2 becomes O(chunk). No change to
per-file logic, hit/miss semantics, or the on-disk store format.

## 7. Risks

- A chunk boundary that is too small loses rayon's parallel efficiency (more fold overhead, worse
  wall time) — chunk size needs to be measured, not guessed; start near repo_size / num_cpus and
  tune against the 1.10x gate.
- The fold step must preserve the existing dedup/ordering semantics in `merge_store_entries`
  (`src/cache/artifacts.rs:113-133`) — the plan's RED tests pin that before the chunking lands.
