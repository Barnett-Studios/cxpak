---
id: '0052'
title: --timing flag emitting per-stage pipeline durations to stderr
status: ACCEPTED
date: 2026-03-12
triggered_by: Need measurement before optimizing, and visibility into which pipeline stage dominates
loop: planning
---

# ADR-0052: --timing flag emitting per-stage pipeline durations to stderr

## Context
Released in v0.6.0 (Workstream 2, Step 1). Before optimizing pipeline speed, the team wanted per-stage measurement and visibility into which stage dominates. The `--timing` flag wraps each pipeline stage with `std::time::Instant` and prints durations to stderr, explicitly preceding the rayon parallelism work as a measure-first gate. Confirmed shipped: `timing: bool` flags are present on Overview, Diff, and Trace in src/cli/mod.rs, with `eprintln!("cxpak [timing]: ...")` blocks in the command modules.

## Options considered
- **Option A — Instant-per-stage timing printed to stderr (chosen):** `std::time::Instant` around scan/parse/index/graph/render, printed to stderr so stdout context output stays clean for piping. Pros: cheap and dependency-free; keeps stdout pure for downstream consumers; enables the measure-then-optimize sequencing. Cons: coarse wall-clock granularity; manual instrumentation per command.
- **Option B — external profiler (perf/flamegraph):** A reasonable alternative would have been to rely on OS profilers instead of built-in timing. Pros: deep call-level detail. Cons: not user-facing; provides no per-stage repo-level signal during normal runs. Not pursued.

## Decision
Add a `--timing` flag to overview, trace, and diff that wraps each pipeline stage (scan, parse, index, graph, render) in `std::time::Instant` and prints durations to stderr. Stdout stays reserved for context output. Timing lands before the rayon work so optimization decisions are measured rather than assumed.

## Consequences
### Positive
- Per-stage visibility on real repos; drives the speed-optimization decision.
- stderr output keeps stdout clean for downstream consumers.

### Negative
- v0.6.0 shipped the flag as a dead no-op in trace and diff (only overview was wired; trace/diff used `_timing`); fixed in v0.6.1 by two "wire --timing flag into trace/diff command" commits.

### Neutral
- Format is human-readable lines, not machine-parseable.

## Revisit if
- A structured/JSON timing format is needed for automated benchmarking.
