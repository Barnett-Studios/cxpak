---
id: '0059'
title: Extract relevance scoring into a standalone src/relevance/ core module
status: ACCEPTED
date: 2026-03-17
triggered_by: Need for task-aware file ranking reusable across MCP, CLI, and HTTP surfaces
loop: planning
---

# ADR-0059: Extract relevance scoring into a standalone src/relevance/ core module

## Context

v0.9.0 introduces task-aware context bundling. The relevance scoring logic could have been embedded directly in the MCP `serve` handler, but it needed to be reusable and testable in isolation.

## Options considered

- **Option A — Standalone `src/relevance/` module with `RelevanceScorer` trait:** A dedicated module exposing a `RelevanceScorer` trait plus a `MultiSignalScorer` implementation, with `signals.rs` and `seed.rs` submodules. Pros: reusable across MCP/CLI/HTTP, testable in isolation, and the trait allows swapping in an embedding-based scorer later. Cons: more module boilerplate than inlining in the handler.

- **Option B — Inline scoring inside the `serve.rs` MCP handler:** Compute relevance directly in the tool-call handler. A reasonable alternative would have been this, since it is less ceremony for a single consumer. Cons: not reusable, couples scoring to the MCP transport, and is harder to unit test. (Not formally evaluated; reconstructed here.)

## Decision

Implement relevance as a first-class core module `src/relevance/` with a `RelevanceScorer` trait and a `MultiSignalScorer` concrete implementation, split into `mod.rs` (trait + scorer), `signals.rs` (signal impls), and `seed.rs` (seed selection). The trait boundary was explicitly chosen to allow later swapping in an embedding-based implementation.

## Consequences

### Positive
- Module reused across MCP and HTTP surfaces (both via `serve.rs`); CLI reuse was deferred as future work and has not shipped.
- The trait abstraction enabled the later embedding-similarity signal to slot in without rewriting consumers.

### Negative
- Extra module/trait indirection for what started as a single MCP consumer.

### Neutral
- The scorer is instantiated per tool call in `serve.rs`.

## Revisit if
- A second scorer implementation never materializes, making the trait a dead abstraction.
- Scoring needs per-request state the trait signature cannot carry.
