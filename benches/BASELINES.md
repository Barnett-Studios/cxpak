# Indexing benchmark baselines

Run: `cargo bench --bench indexing`

Numbers are machine-specific (recorded on the W1 dev machine, Darwin/arm64,
`--sample-size 10 --measurement-time 3` quick settings) and are **indicative
baselines for regression tracking**, not absolute guarantees. Re-record on your
own hardware before using as a gate. CI comparison is advisory (runner noise).

## Headline — per-edit live-update latency

The metric that decides whether `serve`/`watch`/LSP feel instant under edits:
one single-file edit → `incremental_rebuild`.

| Scale | p50 | p99 | max |
|------:|----:|----:|----:|
| 1,000 files | ~12.0 ms | ~13.0 ms | ~13.1 ms |

## Criterion benches

| Bench | Time (median) |
|---|---|
| `cold_build_1k` | ~24.5 ms |
| `incremental_edit_1k` | ~12.7 ms |

## Interpretation & known limitation

A single incremental edit at 1k files is ~2× faster than a cold build. The
edge-delta graph rebuild (ADR-0166) and warm-started PageRank (ADR-0165) are
true deltas — work proportional to the change. The remaining per-edit cost is
dominated by two steps `incremental_rebuild` still recomputes over the **whole
repo**:

- `call_graph` — `rebuild_graph_delta` rebuilds it in full (`build_call_graph`)
  to keep it consistent with the new edges.
- `test_map` — rebuilt in full.

Both are O(repo), so they cap the incremental speedup. Making them per-file
deltas is the natural next optimization (tracked for a follow-up); the exact
graph/PageRank parity guarantees (the W1 definition of done) are unaffected.

The **derived-index cache** (ADR-0167) is the other half of the "always-warm"
prize: it makes *cold startup* fast by skipping the expensive git-mined
conventions/co-changes recompute on a content-fingerprint hit, and is portable
across clones/CI.

## MCP index-ready on real repos (ADR-0210 warm start)

Wall time from spawning `cxpak serve --mcp <repo>` to its `MCP index ready` stderr line, measured with
`scripts/mcp-ready.py` on frozen clones (so HEAD — part of the derived fingerprint — cannot move).
2026-10-15, MacBook M4 Pro, 12 cores, **machine heavily loaded (load average 19–34)**; n = 5
interleaved runs per cell, median (min–max). "Warm" = second start on an unchanged repo (derived
cache hit); "cold" = `.cxpak/` removed before each run.

| Repo (clone) | Files | 3.4.0 warm | 3.5.0 warm | 3.4.0 cold | 3.5.0 cold |
|---|---|---|---|---|---|
| ai-sdlc (`~/.claude`) | 967 | 1.93 s (1.90–2.02) | **0.17 s** (0.14–0.18) | 2.81 s (2.74–3.54) | **1.67 s** (1.64–2.89) |
| scriptorium | 4384 | 14.26 s (14.04–14.32) | **1.85 s** (0.78–2.34) | 22.26 s (21.01–37.08) | **13.29 s** (13.12–15.99) |

Memory, scriptorium warm, store primed (`/usr/bin/time -l` and `footprint`, two runs each):

| Metric | 3.4.0 | 3.5.0 |
|---|---|---|
| peak `phys_footprint` | 445 MB | **383 MB** |
| steady `phys_footprint` (ready + 5 s) | 276–309 MB | 308 MB |
| max RSS | 273–280 MB | 316–396 MB |

Spec A3 was written against peak RSS (≤ +10 %) and is **not met** on that metric. On this
memory-pressured machine RSS mostly reflects how much of a process the kernel left uncompressed,
so `phys_footprint`, the figure macOS charges against memory pressure, is the better signal. On it,
peak falls 14 % and steady state is flat. The likely RSS cost is the whole per-file store held in
memory before entries move into the index. If this matters, revisit through a binary or streaming
store encoding (ADR-0210, "Revisit if").

The artifact store for scriptorium is 55 MB of JSON (`.cxpak/cache/artifacts.json`).

**Phase 2 gate (spec §8):** a post-commit warm start on the scriptorium clone (an empty commit before
each start, so per-file artifacts all hit and the derived cache misses) measured **11.7–11.9 s
(median 11.79 s, n = 5)**, against a 6.0 s gate. The gate fired. The next step is incremental derived
work (a call graph from cached parse results, and conventions after `Ready`) in its own spec.

### Phase 2 gate result (ADR-0212)

Scriptorium clone (4,384 files), release build of `perf/warm-start-p2`, `scripts/mcp-ready.py`, an
empty commit before each start (per-file artifacts and call sites all hit, derived cache misses),
primed once with a long-lived server so the background phase saved the complete derived cache and
the call sites. Load average 4.5 (1 min) at the end of the run.

| Start | n | `index_ready_s` median | min–max | peak RSS |
|---|---|---|---|---|
| post-commit, 3.5.0 (Phase 1 table above) | 5 | 11.79 s | 11.7–11.9 s | n/a |
| **post-commit, Phase 2** | 5 | **3.18 s** | 3.11–3.55 s | 357–379 MB |
| warm, unchanged (derived hit) | 3 | 0.75 s | 0.70–0.82 s | 364–376 MB |

Individual post-commit runs: 3.55, 3.26, 3.18, 3.11, 3.13 s. Gate B1 (median <= 6.0 s): **met**.
Server ready line on a post-commit start: `artifacts 4384 hit/0 partial/0 miss, derived miss, 3.20s`
(the first, cold-store start after the upgrade reads `artifacts 0 hit/.../4384 miss`, 5.4 s once the
v1 store is rejected). The unchanged-repo figure is lower than Phase 1's 1.85 s because the machine
was less loaded; it shows no regression, not a speedup. `artifacts.json` grew from 55 MB to 59.8 MB
with the call sites (2,390 of 4,384 entries have an extractor for their language).
