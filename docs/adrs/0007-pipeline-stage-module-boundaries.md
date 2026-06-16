---
id: '0007'
title: Organize the system as a linear pipeline of single-responsibility modules with explicit I/O boundaries
status: ACCEPTED
date: 2026-03-05
triggered_by: Need a module decomposition that keeps stages independently testable and replaceable
loop: implementation
---

# ADR-0007: Organize the system as a linear pipeline of single-responsibility modules with explicit I/O boundaries

## Context

The v0.1.0 implementation plan codifies the architecture as a strict pipeline — Scanner → Parser → Index → Budget → Output — where each stage is a module with clear input/output boundaries and the Index is the central data structure. The plan builds and TDD-tests each module in isolation before wiring them together in the overview orchestration task. The decision is about how to decompose the system so that stages stay independently testable and replaceable as the feature set grows.

## Options considered

- **Option A — Linear pipeline of single-responsibility modules:** Scanner, parser, index, budget, git, output, and commands as separate modules, each with a defined input and output. Pros: stages are independently testable, the data flow is explicit, and new capabilities slot in as new stages. Cons: the pipeline ordering is somewhat rigid. Someone could prefer this for the clean module boundaries and the ability to unit-test each stage against a fixture input.
- **Option B — Monolithic command module:** A reasonable alternative would have been a single command module performing scan + parse + budget + render inline. Pros: fewer files. Cons: untestable in isolation, mixed concerns, and hard to extend. Someone could prefer it early on for the lower file count and the absence of inter-module plumbing.

## Decision

Decompose the system into a linear pipeline of single-responsibility modules (scanner, parser, index, budget, git, output, commands), each with clear input/output boundaries, built and unit-tested in isolation before being wired in the overview orchestrator. The Index is the shared central data structure that downstream stages consume.

## Consequences

### Positive
- Each stage is independently TDD-tested against fixture inputs.
- New capabilities slot in as new pipeline stages without disturbing existing ones.

### Negative
- Stage ordering constrains where new cross-cutting logic can live.

### Neutral
- The pipeline framing held: shipped code extended it to Scanner → Parser → Schema → Index → Conventions → Budget → Context Quality → Intelligence → Auto Context → Output, all as discrete modules.

## Revisit if
- A cross-cutting concern cannot be expressed as a discrete pipeline stage and must thread through multiple stages instead.
