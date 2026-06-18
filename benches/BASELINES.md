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
