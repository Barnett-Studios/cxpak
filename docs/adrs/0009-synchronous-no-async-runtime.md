---
id: '0009'
title: No async runtime — synchronous filesystem and CPU-bound pipeline
status: ACCEPTED
date: 2026-03-05
triggered_by: Deciding the concurrency model for a CLI that does file I/O and parsing
loop: planning
---

# ADR-0009: No async runtime — synchronous filesystem and CPU-bound pipeline

## Context

The v0.1.0 workload is filesystem I/O plus CPU-bound tree-sitter parsing. The design rules out an async runtime as unnecessary complexity for this kind of work — "Synchronous is simpler and sufficient" — and lists "Async runtime" explicitly under Not In Scope. The decision sets the concurrency model for the core CLI.

## Options considered

- **Option A — Synchronous, no async runtime:** Plain blocking I/O and CPU parsing, no tokio/async-std. Pros: simpler, sufficient for filesystem + CPU work, and a smaller dependency tree. Cons: no async concurrency primitives if network/server features are later added. Someone could prefer this to keep the binary and dependency surface minimal.
- **Option B — Async runtime (e.g. tokio):** Run the pipeline on an async executor. Pros: concurrency primitives available for future server modes. Cons: unnecessary complexity for I/O + CPU work. The design considered this and placed "Async runtime" under Not In Scope. Someone could prefer it to avoid a later refactor if server surfaces were anticipated.

## Decision

Run everything synchronously with no async runtime, since the work is filesystem I/O and CPU-bound parsing where synchronous is simpler and sufficient; async runtime is declared Not In Scope.

## Consequences

### Positive
- Simpler code and a smaller dependency tree for the core CLI.

### Negative
- The "no async" stance constrained later server surfaces; shipped code added an HTTP `serve` mode and an LSP/daemon (behind the `daemon` feature) that build a tokio runtime and reintroduce concurrent request handling beyond the original synchronous model.

### Neutral
- Parallelism was later added via rayon (data-parallel, not async) for parsing, preserving the spirit of this decision in the core path.

## Revisit if
- Network-facing server/LSP modes need true async concurrency (they later arrived as opt-in features behind the `daemon` flag).
