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
