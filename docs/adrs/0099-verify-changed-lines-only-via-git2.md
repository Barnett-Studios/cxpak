---
id: '0099'
title: cxpak_verify checks only changed lines, scoped via git2 (no git CLI)
status: ACCEPTED
date: 2026-03-27
triggered_by: New cxpak_verify tool must report convention deviations without drowning the LLM in pre-existing debt
loop: planning
---

# ADR-0099: cxpak_verify checks only changed lines, scoped via git2 (no git CLI)

## Context
The v1.1.0 `cxpak_verify` tool reports convention deviations on a change. If it flagged
every existing violation in a touched file, it would overwhelm the LLM with debt it did
not introduce. The diff scope also needs a reliable source that works inside the test
suite. The design doc fixed both constraints: it "uses `git2` APIs exclusively ... no
shelling out to `git` CLI ... testable without a git installation," and it skips
pre-existing code — "only check symbols that are new or modified."

This is a human decision because it trades completeness (whole-file verification) for
signal quality (only new violations), and it commits to a dependency boundary (git2,
no CLI) for testability. The code cannot infer either trade-off.
Shipped in `src/conventions/verify.rs`, wired into the MCP server in `src/commands/serve.rs`.

## Options considered
- **Option A — diff via git2 APIs, check only added/modified lines:** use git2 diff
  APIs (tree-to-tree for a ref, or uncommitted-vs-HEAD with no ref), filter by focus
  prefix, parse added line ranges, and only check symbols that are new or fall within
  those ranges. Pros: no git CLI dependency; unit-testable without a git installation;
  only flags what the change introduced; deterministic. Cons: requires mapping diff
  line ranges to symbols; pre-existing violations near new code can be missed. Someone
  could prefer this for testability and clean, change-scoped output. (Chosen.)
- **Option B — shell out to the git CLI for the diff:** run `git diff` and parse
  stdout. The design doc explicitly rejects this. Pros: simpler diff parsing. Cons:
  requires git installed; not testable in isolation. Someone could prefer it to avoid
  learning git2's diff API.
- **Option C — re-check the whole file when any line changes:** re-verify every symbol
  in a touched file. Pros: catches violations adjacent to edits. Cons: floods the LLM
  with pre-existing debt it did not write — the exact failure mode the tool is meant to
  avoid. Someone could prefer it to catch latent issues near the edit.

## Decision
Scope the diff using git2 exclusively — uncommitted-vs-HEAD when no ref is given,
tree-to-tree when a ref is given — filter by focus prefix, parse added/modified line
ranges, and only check symbols that are new or modified within those ranges.
Pre-existing unchanged code is skipped. No shelling out to the git CLI. Implemented in
`src/conventions/verify.rs` (`get_changed_lines` + `verify_changes`) and dispatched
from `src/commands/serve.rs`.

## Consequences
### Positive
- The LLM only sees violations introduced by the current change.
- Verify is unit-testable without a git binary (tests build real repos via git2).
- Reuses git2, already a dependency.

### Negative
- Mapping diff line ranges to symbols adds parsing complexity.
- Pre-existing debt adjacent to new code is intentionally not reported.

### Neutral
- The no-ref path diffs against HEAD including both staged and unstaged changes.

## Revisit if
- Pre-existing violations near edited code prove important enough to surface.
- git2 diff parsing becomes a maintenance burden.
