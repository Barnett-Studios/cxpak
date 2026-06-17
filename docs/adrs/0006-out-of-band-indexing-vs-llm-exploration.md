---
id: '0006'
title: Index the codebase out-of-band and hand the LLM a token-budgeted briefing instead of letting it explore
status: ACCEPTED
date: 2026-03-05
triggered_by: Need to reduce the 50-100k tokens an LLM burns orienting itself in an unfamiliar codebase before doing useful work
loop: planning
---

# ADR-0006: Index the codebase out-of-band and hand the LLM a token-budgeted briefing instead of letting it explore

## Context

This is the foundational v0.1.0 premise. An LLM dropped into an unfamiliar codebase spends most of its token budget on orientation rather than the task: each file read burns 50-100k tokens navigating, it makes greedy local read decisions with no global view, and it behaves nondeterministically run-to-run with results that don't transfer across models. The design reframes this as a problem solvable by moving indexing entirely outside the token economy — "cxpak inverts this by performing indexing *outside* the token economy" — so that "the LLM gets a briefing packet instead of a flashlight in a dark room."

## Options considered

- **Option A — LLM-driven exploration (status quo):** Let the model read and navigate files itself, spending tokens on orientation. Pros: no external tooling, and it works inside any agent loop. Cons: 50-100k tokens burned navigating, no global view, nondeterministic, and not reusable across models. Someone could prefer this because it requires nothing beyond the model itself.
- **Option B — Out-of-band CPU indexing producing a budgeted bundle:** A separate Rust CLI parses the repo and emits a token-budgeted context bundle the LLM consumes once. Pros: zero navigation tokens, deterministic output, works offline, reusable across models, and a fast first response. Cons: requires building and maintaining a separate indexing tool. Someone could prefer this because the indexing cost moves to cheap CPU time and off the token budget.

## Decision

Build cxpak as a standalone Rust CLI that performs indexing "outside the token economy" and produces a token-budgeted context bundle. The LLM receives a briefing packet — zero navigation tokens, deterministic, offline, reusable across models — rather than exploring the codebase with a flashlight.

## Consequences

### Positive
- Navigation token cost drops to zero; the full budget is available for the task.
- Output is deterministic and reusable across models.
- Works offline.

### Negative
- Indexing quality is now cxpak's responsibility, not the model's — a bad bundle silently degrades downstream task quality.

### Neutral
- Establishes the entire premise the project is built on; every later feature (overview, trace, MCP, LSP) inherits the "index out-of-band, budget the output" framing.

## Revisit if
- Models gain cheap enough long-context navigation that out-of-band indexing stops paying for itself.
- Determinism conflicts with embedding-based scoring added later (the v2.0 embeddings signal).
