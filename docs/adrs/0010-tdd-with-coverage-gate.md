---
id: '0010'
title: Strict Red-Green-Refactor TDD with a CI coverage gate enforced per task
status: ACCEPTED
date: 2026-03-05
triggered_by: Need a quality bar that prevents untested code from landing as the codebase grows
loop: implementation
---

# ADR-0010: Strict Red-Green-Refactor TDD with a CI coverage gate enforced per task

## Context

The v0.1.0 implementation plan's TDD Enforcement Patch mandates that no implementation code is written before a failing test exists, measures coverage with cargo-tarpaulin after every task, and gates CI on a coverage threshold. The patch sets `cargo tarpaulin --fail-under 95`, and per-task steps require 100% coverage on the changed modules. The decision establishes a quality bar that keeps untested code from landing as the codebase grows.

## Options considered

- **Option A — Mandatory TDD + tarpaulin coverage gate:** A failing test precedes every implementation; tarpaulin measures coverage and CI fails under the threshold. Pros: high confidence, regressions caught early, and coverage stays high as features accumulate. Cons: slower per-feature, and a coverage percentage can incentivize trivial tests. Someone could prefer this for the regression safety net it provides on a growing codebase.
- **Option B — Tests written after implementation, no gate:** A reasonable alternative would have been adding tests opportunistically with no enforced threshold. Pros: faster initial velocity. Cons: coverage erodes and untested paths accumulate. Someone could prefer it to move faster early when the design is still churning.

## Decision

Enforce strict Red-Green-Refactor TDD (a failing test must precede every implementation) and gate CI on cargo-tarpaulin coverage. The patch sets `--fail-under 95`; per-module steps target 100% on changed code.

## Consequences

### Positive
- Every module ships with isolated tests and high coverage from day one.
- Regressions surface immediately.

### Negative
- Per-task coverage steps and serial test runs add build/CI time.

### Neutral
- Established tarpaulin as the coverage tool. The shipped project later states CI enforces 90% coverage via tarpaulin, indicating the threshold was relaxed from the patch's 95.

## Revisit if
- Coverage-percentage targets start rewarding low-value tests over meaningful ones.
