---
id: '0203'
title: Self-package import resolution (use <own-crate>::) — deferred, limitation documented
status: ACCEPTED
date: 2026-07-16
triggered_by: issue #20 (secondary — src/main.rs reports exists:false)
loop: planning
---

# ADR-0203: Self-package import resolution — deferred

## Context

Issue #20 flags (as "secondary") that cxpak's own `src/main.rs` reports
`exists:false` from `graph node`. The cause: the dependency graph
stores only edge-participating nodes (`contains_node`, core_graph/graph.rs:240),
and cxpak's `main.rs` imports only `clap::` (external) and `cxpak::cli` /
`cxpak::commands` — via the **package name `cxpak::`**. The Rust import resolver
(`src/index/graph.rs`) resolves `crate::`/`super::`/`self::` relative paths to
local files but treats a `<package-name>::` path as an external crate, so
`main.rs` gets zero resolved edges and is not a node. On a repo whose `main.rs`
uses `use crate::foo` the same path resolves fine (a tiny repro's `main.rs`:
exists:true, out:1). So this is a **narrow self-package (binary→library
cross-crate) resolution gap**, not a graph-model bug.

Human decision: whether to change edge extraction — a high-blast-radius core —
inside a patch release, for a cosmetic gain on entry-point files.

## Options considered

- **Option A — Resolve `use <own-crate>::path` to the local crate root in 3.1.1:**
  makes `main.rs` a node. But import resolution feeds the dependency graph, which
  feeds PageRank, blast radius, health, api_surface and the SPA — every consumer.
  New edges shift PageRank on essentially every repo, and the determinism golden
  fixture (`spa_golden.html`) plus the recall-regression gate (ADR-0172) would
  both need re-baselining. High risk for a bugfix patch; a maintainer optimizing
  for correctness-of-entry-points could still want it.
- **Option B — Document the limitation, defer the fix (chosen):** record that
  `use <package-name>::` from a binary crate is not resolved to local files
  (only `crate::`/`super::`/`self::` are), keep 3.1.1 focused on the usability
  fixes (ADR-0202), and carry the resolution work as its own change with its own
  golden/recall re-baseline and tests. Honest and low-risk.
- **Option C — Special-case only the crate-root entry files (main.rs/lib.rs):**
  narrower than A but still perturbs the graph and still needs re-baselining, for
  even less generality. Worst of both.

## Decision

Option B. Ship ADR-0202's usability fixes in 3.1.1; do **not** change import
resolution. Document the `use <package-name>::` limitation (in the graph docs /
`graph nodes` help note). The `nodes` enumerate op (ADR-0202) already removes the
practical sting — a consumer lists real ids instead of guessing `main.rs`.

## Consequences

### Positive
- 3.1.1 stays a low-risk patch; the golden fixture and recall gate are untouched.
- The gap is documented, not silent; `graph nodes` makes it a non-issue in
  practice.

### Negative
- `graph node --id src/main.rs` still reports `exists:false` on cxpak-like repos
  until the deferred change lands — surprising for entry-point files.

### Neutral
- No code change in 3.1.1 for this item; docs-only.

## Revisit if
- A general Cargo package-name → crate-root resolution is implemented (with its
  own golden + recall re-baseline and tests) — then this decision is superseded.
- Entry-point node coverage becomes load-bearing for a downstream feature
  (e.g. onboarding order, dead-code entry rules ADR-0131) rather than cosmetic.
