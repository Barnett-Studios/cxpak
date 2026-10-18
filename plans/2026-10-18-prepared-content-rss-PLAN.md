---
title: Prepared-content RSS — plan
status: DRAFT
created: 2026-10-18
updated: 2026-10-18
spec: specs/2026-10-18-prepared-content-rss-SPEC.md
adrs: 0215
---

# Plan

Constraints: no public API change; byte-identical outputs; each task RED first. Run tests via the test-runner.

## T1 — Phase-attributed RSS (measurement, no product change)
Sample RSS at 50 ms (`ps -o rss= -p`) during `scripts/mcp-ready.py --lifecycle 20` and tag phases from
`BuildTimings` / log timestamps (build per-file, fold, derived load, assemble, mining, republish, saves).
Output: per-phase peak for U and C. Decides T2's lever. Stop and report if no phase exceeds 480 MB-equivalent
headroom (then ADR-0215 Option D).

## T2 — RED: transient duplicate is not retained
Pick the lever the T1 data names. Candidate RED tests (in `src/commands/warm_start_tests.rs`):
1. `Prepared.fresh` built only for entries that are written: test that on a full artifact-store hit
   `build_index_pending` returns a `PendingStore` with zero `changed` entries and that no `ArtifactEntry`
   is constructed (counter via the existing `BuildReport.artifact_hits == N`, `changed.is_empty()`).
2. Build transients released before mining: test that `PendingStore` handed to the background phase
   does not own `artifact_files`/`Prepared` (structural: type no longer has the field; compile-level).
3. Convention mining borrows `IndexedFile.content` without cloning (assert via `Arc::strong_count`/ptr equality
   of content between the published index and the miner input).
Watch the chosen test fail for the right reason.

## T3 — GREEN
Minimal change in `src/commands/serve.rs` (and `src/index/mod.rs` crate-private items only). Confirm
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, determinism tests green;
`cargo public-api`/semver diff shows no change for `cache`, `core_graph`, `index`.

## T4 — Measurement protocol
- `git clone /Users/lb/Projects/barnett/scriptorium /tmp/frz/scriptorium` (if absent); prime a warm cache.
- Binaries: released `/opt/homebrew/bin/cxpak` (3.5.1), the worktree release build, 3.4.0 if available.
- `/usr/bin/time -l <bin> serve --mcp /tmp/frz/scriptorium < <(sleep 25)` driven by
  `scripts/mcp-ready.py --lifecycle 20`; record `peak memory footprint` and `maximum resident set size`,
  and `index_ready_s`.
- Scenarios: U (warm unchanged), C (untimed priming run, then `git commit --allow-empty`, then measured run).
- n=5 interleaved per binary, each preceded by a priming run; record `uptime` load average with each series.
- Report medians with [min-max]; compare to gate in spec section 4.

## T5 — Docs
Update `benches/BASELINES.md` with the new table; set ADR-0215 ACCEPTED or record the unmet gate and
Option D / follow-up ADR for Option B.
