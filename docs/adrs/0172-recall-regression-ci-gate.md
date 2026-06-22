---
id: '0172'
title: CI recall-regression gate on a bounded subset; gate recall@budget, track MRR
status: ACCEPTED
date: 2026-06-22
triggered_by: cxpak 3.0.0 Phase D2.3 (benchmark gate) — D2.1 corpus + D2.2 recall harness
loop: implementation
---

# ADR-0172: CI recall-regression gate on a bounded subset; gate recall@budget, track MRR

## Context

D2.1 committed a corpus of 62 real merged PRs across 6 repos; D2.2 built a recall
harness that, for a PR, fetches the repo at its `base_sha`, indexes it, and
measures each retrieval system's `recall@{8k,32k}` and `MRR` against the PR's
base-tree changed files. The shipped product row is `cxpak (auto_context)`.

D2.3 must turn that harness into a CI **gate** so that later work — the C2
ranking changes and the D1 semantic/embedding work, both explicitly aimed at
*improving* retrieval — cannot silently *regress* the recall we already have.
Two facts force human judgment here rather than a mechanical "run everything and
require we win":

1. **A scale wall.** D2.2 measured indexing the full corpus on a CI-class
   machine and hit ~7 GB RSS / 100% CPU on `spring-projects/spring-boot`; the
   `microsoft/TypeScript` tree is similarly heavy. Running the whole corpus on a
   default GitHub runner is not viable. The gate must therefore run on a
   *subset*, and which subset is a trade-off (coverage vs runner budget) a human
   must own.
2. **cxpak wins recall but trails MRR.** On the bounded subset, cxpak
   (auto_context) leads every baseline on recall@budget (recall@8k 0.500 vs
   0.000 for ripgrep/score_all; recall@32k 0.533 vs 0.367) but trails badly on
   MRR (0.410 vs ripgrep 1.000). MRR is a *known* current weakness that D1/C2 are
   meant to lift. Gating MRR now would red-light the very branch chartered to fix
   it. Whether to gate a metric you are about to improve is a policy choice, not
   something the code can decide.

## Options considered

### Subset scope

- **Full corpus in CI:** maximal coverage and statistical power. Rejected: it
  does not fit a default runner's memory/time (the spring-boot/TypeScript wall) —
  the job would OOM or time out, making the gate flaky and effectively useless. A
  stakeholder might prefer it if a large self-hosted runner were available
  (see Revisit-if).
- **Bounded resource-safe subset (chosen):** a fixed handful of small-repo PRs
  (ripgrep / flask / express). Fits the runner; pinned by `(repo, pr)` to
  immutable merged PRs so numbers are reproducible. Cost: fewer entries → a
  coarser recall estimate, and the excluded large repos (Java/TS/Go) are not
  exercised by the gate.

### What to gate

- **Gate recall + MRR:** strongest guard. Rejected for now: cxpak trails on MRR,
  so this would fail the branch immediately on a metric D1/C2 are about to
  improve — gating against ourselves. A stakeholder who considered MRR the
  primary quality signal could legitimately prefer it.
- **Gate recall@budget only; track MRR (chosen):** hard-fail on recall
  non-regression at {8k,32k}; record and print MRR (baseline vs current, with the
  delta) but never fail on it. Locks in the metric cxpak currently leads while
  leaving the metric it trails visible and improvable. Cost: a pure-MRR
  regression that left recall untouched would not be caught by the gate (it would
  still be visible in the printed table).

### Gate strictness

- **Require >= best baseline:** "cxpak must beat everyone." Rejected as the gate
  bar: it is an *aspiration*, not a *non-regression* contract, and it couples the
  gate to baseline systems' behavior. A stakeholder might prefer it as a
  motivating target — so we print the cxpak-vs-baseline delta informationally.
- **Non-regression vs a committed baseline (chosen):** current recall must be
  `>= committed baseline - tolerance`. The subset is pinned to immutable PRs and
  the harness is deterministic, so a faithful re-run reproduces the baseline
  exactly (verified: two runs gave identical numbers, delta +0.0000). Tolerance
  is `1e-6` — a hairline to absorb last-bit float drift in the cross-entry mean
  on a different host, not a loophole; any real recall drop fails.

## Decision

Add a CI `bench` job (mirroring `test`: `needs: [fmt, clippy]`,
checkout@v6.0.2 + rust-toolchain@1.94.1 + rust-cache@v2.9.1) that:

- **Always** runs the no-network tests — `cargo test --features bench --test
  bench_recall --test bench_gate` — covering corpus integrity, the pure metric
  math, and committed-baseline integrity. These have no external dependency, so
  the gate is never vacuous.
- **Network gate:** runs the harness on a **fixed, resource-safe subset**
  (`default_subset()` = ripgrep#3420, flask#5928, express#7234, excluding the
  spring-boot/TypeScript/cli repos that hit the scale wall) and **hard-fails** if
  `cxpak (auto_context)` recall@{8k,32k} drops below the committed
  `bench/baseline.json` minus a `1e-6` tolerance. MRR is computed, recorded, and
  printed but **not** gated. The gate's pass/fail is the job's exit status.
- Runs with `GH_TOKEN: secrets.GITHUB_TOKEN` (auto-provided on this repo's
  branch/PR runs; gh uses it for an authenticated, higher-rate-limit API) and
  `CXPAK_BENCH_NET=1`. On forked-PR runs where the token is absent/limited it
  **skips gracefully** (never claiming to have run) rather than false-failing —
  but it runs with teeth on this repo's own branches and PRs.

The committed `bench/baseline.json` (format_version 1) carries the pinned subset,
the gated `cxpak (auto_context)` numbers, and every baseline system's numbers
(informational), generated by running the harness once — never hand-written.

## Consequences

### Positive
- Recall regressions in C2/D1/any change are caught automatically before merge;
  cxpak's current recall lead is locked in.
- The gate is cheap and reproducible: pinned immutable PRs + deterministic
  harness → identical numbers run-to-run (a 0.0 tolerance would even hold; the
  1e-6 is pure insurance).
- The no-network layer always runs, so the corpus, metric code, and baseline
  artifact are continuously validated even without network.
- The full comparison table (including MRR and baseline systems) prints in the CI
  log, so progress toward beating baselines on MRR stays visible.

### Negative
- A pure-MRR regression that leaves recall untouched is not hard-gated (by
  design); it is only visible in the printed table.
- The bounded subset under-samples: large Java/TypeScript/Go repos are not
  exercised by the gate, so a regression specific to those would slip past it.
- The network gate depends on GitHub being reachable and the corpus repos
  staying public; an upstream outage fails the job (deliberately — in CI a
  network failure surfaces loudly rather than passing silently).

### Neutral
- The `bench` feature stays out of the default build and the coverage job's
  feature list (it is a measurement tool, not shipped code).
- Re-baselining is a deliberate, reviewed change to both `bench/baseline.json`
  and `default_subset()` (a test asserts they agree).

## Revisit if

- **D1 persists embeddings / changes ranking** such that recall@budget shifts
  for the better — re-baseline (`CXPAK_BENCH_NET=1 CXPAK_BENCH_GEN=1 cargo test
  --features bench --test bench_gate regenerate_baseline -- --ignored`) and raise
  the locked-in floor.
- **MRR becomes competitive** (D1/C2 lift it to ~baseline) — promote MRR from
  tracked to gated.
- **A larger CI runner becomes available** (self-hosted / high-memory) — expand
  the subset toward the full corpus to include the Java/TypeScript/Go repos the
  scale wall currently excludes.
- **A pinned corpus PR is force-removed or the repo goes private** — the gate
  fails to resolve the subset; replace the entry and re-baseline.
