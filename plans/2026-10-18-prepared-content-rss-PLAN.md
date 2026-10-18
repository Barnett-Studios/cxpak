---
title: Prepared-content RSS — plan
status: DONE (T1 attribution and T4 measurement executed; no T2/T3 lever pursued, see ADR-0215 Outcome)
created: 2026-10-18
updated: 2026-10-18
spec: specs/2026-10-18-prepared-content-rss-SPEC.md
adrs: 0215
---

# Plan

Constraints: no public API change; byte-identical outputs; each task RED first. Run tests via the test-runner.

## T1 — Attribution (measurement, no product change)
`/usr/bin/time -l` is the arbiter for every number. A `ps` sampler (50 ms) is used only to tag which phase
a peak falls in. Scenarios: U, C, and M = U with the derived fingerprint forced to miss (derived cache
file removed, HEAD unchanged). U vs M isolates build-time derived analysis; M vs C isolates stale-base
mining/republish effects. Confirm "republished in place" fires in every C run.

## T2 — RED tests, one per lever actually pursued
Each RED must fail on today's code for a behavioural or structural reason. Candidates:
1. `git_health` mining must not hold duplicate path lists: unit test over a temp git repo asserting the
   co-change input is built by move (a counting allocator or an API shape test is not used; instead assert
   `build_git_health` output equal to a recorded golden and that the new `commit_file_sets` borrow/move
   path is exercised) - if no behavioural RED exists, justify and protect by the existing determinism
   tests plus measurement.
2. `MinedConventions.co_changes` is moved out of `git_health.co_changes`, not cloned: test that
   `conventions.git_health.co_changes` is empty after `mine` only if the field contract allows it; else
   skip and say so.
3. libgit2 cache bound (`git2::opts::set_cache_max_size`): not unit-testable for RSS; protected by the
   conventions determinism tests plus measurement.
Watch each chosen test fail for the right reason.

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
