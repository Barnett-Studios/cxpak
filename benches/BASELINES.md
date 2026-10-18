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
(the first start after the upgrade rejects the v1 store: `artifacts 0 hit/0 partial/4384 miss`, 2.2 s
with a derived hit; the first post-commit start after that took 5.4 s with the machine at a 10+ load
average, before the 5-run series above). The unchanged-repo figure is lower than Phase 1's 1.85 s because the machine
was less loaded; it shows no regression, not a speedup. `artifacts.json` grew from 55 MB to 59.8 MB
with the call sites (2,390 of 4,384 entries have an extractor for their language).

## 3.5.1 peak RSS (in-place republish, bounded batches) — gate NOT met

`scripts/mcp-ready.py --lifecycle 20` on `/tmp/frz/scriptorium` (4,384 files), idle (no tool calls after
the first answer), n=5, binaries interleaved, each run preceded by an untimed priming run of the same
binary (C: empty commit between prime and measured run). The machine was shared and heavily loaded
(load average 5 to 60 during the series), so 3.4.0 `index_ready_s` and RSS vary widely; ranges are
shown, medians first.

Scenario U (warm, unchanged):

| Binary | peak_mb (full lifecycle) | build_peak_mb | post_ready_peak_mb | index_ready_s |
|---|---|---|---|---|
| 3.4.0 | 324 [292-367] | 324 [292-367] | 262 [213-316] | 14.68 [14.34-14.70] |
| 3.5.0 (separate series, same day) | 364 [364-365] | 364 [364-365] | 329 [328-330] | 0.73 [0.71-0.73] |
| 3.5.1 (this branch) | 374 [365-380] | 374 [365-380] | 329 [329-330] | 0.81 [0.78-0.83] |

Scenario C (empty commit before each run), two interleaved series:

| Series | Binary | peak_mb | build_peak_mb | post_ready_peak_mb | index_ready_s |
|---|---|---|---|---|---|
| 3.4.0 vs 3.5.1 | 3.4.0 | 476 [284-574] | 476 [284-574] | 229 [151-300] | 20.57 [20.49-21.56] |
| 3.4.0 vs 3.5.1 | 3.5.1 | 565 [359-587] | 380 [359-404] | 565 [322-587] | 3.26 [3.21-3.37] |
| 3.4.0, 3.5.0, 3.5.1 | 3.4.0 | 267 [236-352] | 267 [236-352] | 148 [139-155] | 23.62 [22.36-90.18] |
| 3.4.0, 3.5.0, 3.5.1 | 3.5.0 | 392 [308-457] | 329 [301-399] | 392 [229-457] | 4.06 [2.96-17.63] |
| 3.4.0, 3.5.0, 3.5.1 | 3.5.1 | 372 [317-565] | 372 [317-377] | 323 [227-565] | 3.38 [3.35-16.76] |

Republish path taken (stderr `conventions republished ...`): in place in 9 of 9 C runs that republished
(the tenth run's derived cache was not stale); the clone fallback was never taken.

Gate (full-lifecycle peak <= 3.4.0 x 1.10 in U and C; `index_ready_s` <= 3.5.0 + 10 %):
- U: **not met** (374 vs 324 x 1.10 = 356 MB). The bounded batch did not lower the build peak
  (374 vs 3.5.0's 364).
- C: **not met** (3-way series: 372 vs 267 x 1.10 = 294 MB; 2-way series: 565 vs 476 x 1.10 = 524 MB).
  The post-ready peak is lower than 3.5.0's in the 3-way series (323 vs 392) but 4 of 5 runs in the
  2-way series and 1 of 5 in the 3-way series peaked at 553-587 MB around t=9.3-9.9 s even though the
  republish was in place, so the clone is not the only post-ready contributor (conventions mining and
  its derived-cache save run in the same window). Not explained by this series.
- `index_ready_s`: U 0.81 vs 0.73 (+11 %); C 3.26-3.38 vs 2.90 (+12-16 %) against 3.5.0 in the same-day
  series; against the spec's reference figures (U 0.85, C 2.86) U passes and C is at +14-18 %.

### 3.5.1 peak RSS, round 2 (batching reverted; borrowed derived-cache save) — `/usr/bin/time -l`

The bounded-batch change in the section above was reverted (no benefit, +11-16 % ready time). This
round adds `DerivedCacheRef` (derived cache saved by reference and streamed to disk) to the in-place
republish and watcher release.

Method: `/usr/bin/time -l <bin> serve --mcp /tmp/frz/scriptorium < <(sleep N)`, alive about 20 s past
ready, n=5 interleaved, each run preceded by an untimed priming run of the same binary (C: empty commit
between prime and measured run). 3.5.0 is the v3.5.0 release binary. Load average (1 min) during the
series: 2.6-5.8. Medians with `[min-max]`, in MB. `peak memory footprint` is the gate metric.

| Scenario | Binary | peak memory footprint | maximum resident set size |
|---|---|---|---|
| U | 3.4.0 | 422.8 [388.2-424.4] | 395.3 [355.6-456.0] |
| U | 3.5.0 | 372.8 [372.5-373.6] | 385.0 [384.6-385.7] |
| U | 3.5.1 | 372.8 [372.5-373.3] | 384.9 [384.7-385.4] |
| C | 3.4.0 | 692.1 [688.9-694.9] | 458.4 [447.1-601.0] |
| C | 3.5.0 | 646.2 [621.8-649.0] | 568.0 [498.4-600.4] |
| C | 3.5.1 | 647.7 [643.5-650.4] | 569.5 [502.6-592.6] |

In place on all 5 C runs. Gate (footprint <= 3.4.0 x 1.10): **met** (U 372.8 vs 465.1; C 647.7 vs 761.3).
Resident set size alone: U met (384.9 vs 434.8); C **not met** (569.5 vs 504.2).
3.5.1 is indistinguishable from 3.5.0 on both metrics here: in this series the footprint was already
below 3.4.0's, and the in-place republish and borrowed save did not change it measurably.

### 3.5.1 max RSS re-measurement and phase attribution (ADR-0215) — `/usr/bin/time -l`

Method: as round 2, on a fresh clone of scriptorium at `/tmp/frz/scriptorium` (HEAD 48d39208), n=5
interleaved, each measured run preceded by an untimed U priming run of the same binary (C: empty commit
between prime and run). 3.4.0 is a v3.4.0 source build, 3.5.1 is `/opt/homebrew/bin/cxpak`. The machine
was shared and loaded: 1-min load average 8-38 across the series (18 logged-in users). Medians
`[min-max]`, MB.

| Scenario | Binary | peak memory footprint | maximum resident set size |
|---|---|---|---|
| U | 3.4.0 | 429.6 [335.0-429.8] | 252.9 [251.3-263.4] |
| U | 3.5.1 | 376.7 [376.5-377.4] | 314.3 [292.6-346.5] |
| C | 3.4.0 | 689.0 [220.5-694.5] | 306.8 [257.8-316.8] |
| C | 3.5.1 | 650.7 [645.6-654.8] | 350.2 [338.5-388.2] |

- Footprint (stable run to run): 3.5.1 below 3.4.0 in both scenarios.
- Max RSS did not reproduce round 2's 570 MB for C: 3.5.1 C is 350 MB here (338-388), under the fixed
  504 MB limit; 3.4.0 measured 253 / 307 MB, so 3.5.1 sits above 3.4.0 x 1.10 (U 278, C 337) on this
  metric. The 3.4.0 RSS values are 40-45 % lower than round 2's (395 / 458) on the same binary,
  so max RSS on this host depends on memory pressure (macOS compresses pages, which `ps` and
  `ru_maxrss` stop counting) and is not a stable gate metric under load.
- Phase attribution (`scripts/mcp-ready.py --lifecycle 20`, 3.5.1, scenario C, 2 runs):
  build peak 334 / 329 MB at 0.6-0.9 s; post-ready peak 339 / 254 MB. The build phase sets the
  maximum; mining and republish do not. A temporary RSS trace (not kept) showed the per-file stage
  (store load + all file contents + artifacts) at 211-303 MB and `assemble` at 164-373 MB, with
  differences of 100+ MB between identical runs.
- jemalloc decay tuning via `_RJEM_MALLOC_CONF` (`dirty_decay_ms:1000` / `:0`, `muzzy_decay_ms:0`,
  `narenas:2`) showed no reduction (C 335-352 vs 343-388 base; U 372-385 vs 329-346).

## Shared index server, N=4 concurrent sessions (ADR-0214, 2026-10-18)

Method: `/tmp/frz-srv/measure.py` starts 4 `serve --mcp /tmp/frz-srv/scriptorium` sessions (frozen
clone of `/tmp/frz/scriptorium`, HEAD ccbab5d0, 4,384 files), each polling one `cxpak_graph` call
until the index answers, waits 5 s, then sums `footprint` per process (server included). Same
release binary both ways: `CXPAK_INDEX_SERVER=off` is the 3.5.1 in-process path. Per-run totals, MB.

| Mode | Run 1 | Run 2 | Run 3 | Composition (run 3) |
|---|---|---|---|---|
| off (4 x in-process) | 2756 | 1287 | 1285 | 4 x 321 |
| on (1 server + 4 clients) | 349.5 | 348.8 | 350.1 | 332 server + 4 x ~4.5 |

Run 1 of the `off` series built the derived cache cold (690 MB each); runs 2-3 are warm-cache.
Target: total <= (one in-process footprint + 4 x thin-client footprint) x 1.2 = (322 + 18) x 1.2 =
408 MB. Measured 349-350 MB: **met**, 3.7x below the warm in-process total (1285 MB). The idle-exit
default (600 s) is reasoned, not measured.
