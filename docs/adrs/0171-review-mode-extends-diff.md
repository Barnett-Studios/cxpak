---
id: '0171'
title: Review mode as a --review extension of cxpak diff, composing existing intelligence + expected-but-absent detection
status: ACCEPTED
date: 2026-06-16
triggered_by: v2.3.0 W3 — a review-context entry point, without duplicating diff's git/budget machinery
loop: planning
---

# ADR-0171: Review mode as a `--review` extension of `cxpak diff`

## Context

`commands::diff::run` (diff.rs) already does the hard parts of change-scoped context: it extracts the git diff (`extract_changes`), walks dependents with an **ad-hoc `graph.dependents` traversal** (1-hop, or full BFS with `--all`, commands/diff.rs:289-299), builds a convention **profile** (`build_convention_profile`, commands/diff.rs:245), attaches co-change (commands/diff.rs:246), and renders budgeted context **signatures** (`render_context_signatures`). It does **not** call `compute_blast_radius`, `predict`, `conventions::verify_changes`, or `build_security_surface` — yet all of those exist and `compute_blast_radius`/`predict` already take `changed_files: &[&str]`, and `verify_changes` (verify.rs:163) already verifies only changed lines (via `get_changed_lines`, verify.rs:52, which populates `ChangedFile.added_lines` and is already line-scoped — no line-scoping work is needed). So a review bundle is a composition of existing, tested functions over the change set diff already computes.

Two facts shape the design. **(1) cxpak uniquely knows what usually changes together.** The index mines a co-change graph (`CoChangeEdge{file_a, file_b, count, recency_weight}`, populated on the diff path at diff.rs:244) and builds a source→test map. Reviewers' highest-value catch is the *sin of omission* — the file that should have changed but didn't — and we have exactly the data to surface it. **(2) the diff path's index is lean.** `diff::run` builds via `CodebaseIndex::build_with_content`, which leaves `pagerank` and `test_map` **empty** (only `conventions`/`co_changes` are set afterward) — so the review bundle must compute pagerank and the test map locally, or blast-radius risk collapses to ~0 and impacted tests come back empty.

## Options considered

- **Option A — `--review` flag on `cxpak diff`, composition + omission detection (chosen):** when set, after `extract_changes` yields the changed files, compose `compute_blast_radius` (categorized dependents + risk, **replacing the ad-hoc dependents walk**), `predict` (impacted tests by confidence), `verify_changes` (convention violations on changed lines), a focus-scoped `build_security_surface` filtered to the changed set, **and a new pure `detect_omissions(changed, co_changes, test_map)`** that flags expected-but-absent changes — co-changed-but-untouched files (ranked by `count × recency_weight`, thresholded) and changed source files whose high-confidence test wasn't touched. The bundle computes `pagerank` + `test_map` locally (the index is lean — see Context). Render a risk-ordered review section, **led by the omissions**. Mirror as `review: bool` on `cxpak_diff`. Pros: reuses diff's git + budgeting + rendering machinery and four tested intelligence functions; omission detection is a pure function (unit-testable with synthetic inputs); default behavior unchanged. Cons: diff output grows under the flag; `build_security_surface` is whole-index with an optional focus, so the review filters its findings to changed paths; `--review` recomputes pagerank/test_map per invocation (O(repo), acceptable for a CLI command). Chosen as composition plus the one genuinely novel signal cxpak alone can produce.
- **Option B — a new `cxpak review` command:** Pros: a clean dedicated surface. Cons: would re-implement diff's git-diff extraction, budgeting, and rendering — duplication that directly violates the release's extend-don't-add constraint. Someone could prefer it for conceptual separation; not worth the duplication.
- **Option C — leave diff as-is, document manual composition:** Pros: no code. Cons: users must call several tools and stitch results — poor UX for the headline "AI-edit safety" use case. Rejected.

## Decision

Option A. Add `--review` to `cxpak diff` (and `review: bool` to `cxpak_diff`) composing the four existing functions over the changed set, plus a pure `detect_omissions`, output ordered by risk and led by expected-but-absent changes. No new command, no new analysis beyond the omission detector (which is pure aggregation over the existing co-change graph and test map).

## Consequences

### Positive
- One review entry point reusing tested intelligence and diff's existing machinery.
- Directly serves the "know the blast radius of a change before merging" narrative.
- **Catches sins of omission** — the file/test that should have changed but didn't — using the co-change graph cxpak already mines. This is the senior-reviewer catch most tools cannot make, and the strongest single differentiator of `--review`.

### Negative
- Default `diff` output must stay unchanged — the composition is gated behind `--review`.
- `build_security_surface` operates on the whole index with an optional focus prefix; the review runs it focus-scoped and filters findings to the changed set, which is coarser than a true per-file delta.
- `--review` computes pagerank + test_map locally each run (the diff-path index omits them), so it is O(repo), not O(change). Fine for a CLI command; noted so a future watch/serve integration caches them instead.

### Neutral
- `compute_blast_radius` supersedes the ad-hoc `graph.dependents` walk inside review mode; the plain walk remains for non-review `diff`.
- Omission strength is reported as `count × recency_weight`, not a ratio — the co-change miner tracks pair counts only, no per-file denominator.
- `detect_omissions` keeps full recall (every qualifying pairing); the **render layer** caps the co-change list to the strongest few (top 7 by weight) and summarizes the rest as "…and N more", while always rendering missing high-confidence tests. Manual QA showed an uncapped list (20+ entries for a single changed file) buries the high-value catch — the cap is a presentation choice to keep signal above noise, not a recall change.

## Revisit if
- Review needs cross-PR or multi-commit ranges — extend `extract_changes`'s range handling rather than fork a command.
- The ad-hoc dependents walk in plain `diff` turns out to have no remaining consumers — collapse it onto `compute_blast_radius` everywhere.
- **(Deferred)** A confidence *ratio* for omissions ("changed together in 23 of 25 commits") proves more useful than `count × recency_weight` — add per-file change totals to `mine_co_changes` to provide the denominator.
- `--review`'s per-run pagerank/test_map recompute becomes a latency problem — share a warm index via the W1 derived cache / serve path.
