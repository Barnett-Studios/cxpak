---
id: '0049'
title: --since flag resolves a time expression to a git commit
status: ACCEPTED
date: 2026-03-12
triggered_by: diff only supported --git-ref; users wanted 'what changed in the last N days'
loop: planning
---

# ADR-0049: --since flag resolves a time expression to a git commit

## Context
Released in v0.7.0. `diff` supported `--git-ref` against a specific commit, but the common need is a relative time window ("what changed in the last N days"), which forced users to look up commit hashes by hand. Adding a time-expression parser that resolves a window to a commit ref lets the existing `--git-ref` diff path be reused unchanged.

The design doc (`2026-03-12-v070-performance-dx-design.md`) proposed resolving the window with an in-process `git2` revwalk. The shipped implementation diverged: `resolve_since` (src/commands/diff.rs:81-104) instead shells out to the `git` CLI, and the precedence rule was reversed relative to the doc. This record describes what shipped.

## Options considered
- **Option A — Time expression → Duration → `git log --since` CLI (shipped):** `parse_time_expression` (diff.rs:18-76) parses day/hour/week short and long forms plus `yesterday` into a `Duration` (month = 2592000s ≈ 30 days; empty, zero, or unparseable expressions error). `resolve_since` then runs `git -C <repo> log --all --format=%H --since=<N> seconds ago`, takes the oldest commit in that window, and uses its parent (`{oldest}~1`) as the diff base fed to `extract_changes`. Pros: reuses the `--git-ref` machinery; natural UX; relies on git's own `--since` date handling. Cons: shells out to the `git` binary for this one step (the rest of the diff path still uses git2 in-process); accuracy bounded by commit timestamps; window emptiness is an error rather than an empty diff.
- **Option B — in-process `git2` revwalk (design-doc proposal, not shipped):** the design doc proposed walking commits in time order with `git2` and returning the first commit at or before `now - duration`. A reasonable alternative someone could prefer: keeps all git access in-process with no `git` binary dependency for this step. Rejected in practice — the shipped code took the simpler CLI shell-out instead; this path was never implemented.

## Decision
Add a `--since` flag to `diff` that parses a time expression (day/hour/week short and long forms, plus `yesterday`) into a `Duration`. `resolve_since` shells out to `git log --all --format=%H --since=<N> seconds ago`, takes the oldest commit in that window, and uses its parent (`{oldest}~1`) as the diff base passed to `extract_changes`. Zero, empty, or unparseable expressions error; an empty `git log` result errors with "no commits found in the last {expr}".

`--git-ref` takes precedence over `--since`: `--since` is resolved only when `--git-ref` is absent (src/main.rs:110-111, `(Some(_), _) => git_ref`, `(None, Some(s)) => resolve_since(...)`). This is the opposite of the design doc's stated intent, which was reversed in the shipped code.

## Consequences
### Positive
- Relative time windows without manual commit-hash lookup.
- Reuses the existing `extract_changes` / `--git-ref` diff machinery; `--since` only produces a ref string.

### Negative
- Accuracy is bounded by commit timestamps; an empty time window errors with "no commits found in the last {expr}" rather than producing an empty diff.
- Introduces a `git` CLI shell-out for the resolution step, while the rest of the diff path uses git2 in-process.

### Neutral
- `--git-ref` takes precedence over `--since` (not the reverse).
- Month is approximated as 30 days (2592000s).

## Revisit if
- Users need calendar-accurate months or timezone-aware windows.
- A `--before` flag or an explicit time/commit range form is requested.
- The team wants the git2 in-process revwalk from the design doc to replace the CLI shell-out.
