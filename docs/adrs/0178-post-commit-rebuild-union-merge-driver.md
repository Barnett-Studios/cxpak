---
id: '0178'
title: Post-commit auto-rebuild + union-merge driver for a committable graph artifact
status: ACCEPTED
date: 2026-06-30
triggered_by: cxpak 3.0.0 Phase B Task B3 (git-integration features)
loop: implementation
---

# ADR-0178: Post-commit auto-rebuild + union-merge driver for a committable graph artifact

## Context

Phase B gave cxpak a persisted dependency graph (B1 graph-query, B2 Cypher/GraphML
export). Two workflow gaps remained:

1. A committed graph artifact goes stale the moment someone commits code without
   re-running cxpak. We want it refreshed automatically after each commit —
   **without ever risking the user's git workflow** (a post-commit hook that
   errors or hangs is worse than a stale artifact).
2. If a team commits the artifact, two branches that each regenerated it produce
   different-but-both-correct files, so a merge conflicts on a *derived* file —
   pure noise a human should never resolve by hand.

Both require a human decision because they trade correctness granularity against
safety and determinism, and because they wire cxpak into a foreign repo's git
plumbing (hooks, config, attributes) where a wrong default is destructive.

A hard boundary: this is the cxpak **product** feature that installs into a USER
repo. It must never touch *this* repository's own dev hooks (`.git/cxpak-hooks/*`,
repo-local `core.hooksPath`) or git config.

## Options considered

- **Canonical artifact format — line-oriented sorted edge list (chosen):** one
  edge per line `<from>\t<to>\t<edge_type>\t<confidence>`, sorted + deduped,
  derived from the `BTreeMap`/`BTreeSet`-backed `DependencyGraph`. Pro: union-merge
  is a trivially well-defined, conflict-free set operation; byte-deterministic.
  Con: not a graph-interchange format (a stakeholder wanting tool interop might
  prefer reusing B2's GraphML/Cypher). Rejected interop because B2's exports are
  *not* line-oriented per edge (nested XML / multi-line Cypher statements), so a
  line-set union over them is undefined — they would conflict exactly like the
  raw graph does.
- **Post-commit incrementality — persisted parse cache + full graph build (chosen):**
  the standalone post-commit process has no hot in-memory index, so cross-process
  incrementality comes from the already-persisted parse cache (`parse_with_cache`
  re-parses only the files the commit touched); the graph is then built by the
  same `build_dependency_graph` the whole pipeline uses. Pro: reuses existing
  machinery, no second index serialization, correct by construction. Con: it does
  a full graph assembly rather than an in-memory edge-delta. We accept this
  because the edge-delta machinery (`rebuild_graph_delta`) is the serve/watch
  *hot-loop* optimization and requires a live prior index; we instead **assert the
  invariant** that an edge-delta-updated graph serializes byte-identically to a
  full rebuild (`post_commit_incremental_equals_full`), so the committed artifact
  is path-independent.
- **Merge semantics — union of ours ∪ theirs, ancestor ignored (chosen):** vs. a
  true 3-way merge that honours deletions relative to the base. Pro: union is
  commutative, deterministic, and never fails to resolve; it keeps every edge
  either branch knew about. Con: a deletion on one side is not propagated — a
  removed edge can linger. We accept this because the artifact is *derived and
  self-correcting*: the post-commit hook fires on the merge commit and regenerates
  it exactly, so any lingering edge is gone after the next commit. A 3-way merge
  would add base-parsing complexity for a property the regeneration already
  guarantees.
- **Post-commit failure handling — best-effort, always exit 0 (chosen):** vs.
  surfacing rebuild errors as a non-zero exit. A non-zero post-commit exit
  confuses some tooling and erodes trust in the hook; a stale artifact is strictly
  recoverable. Rejected the strict variant for the post-commit path (kept it for
  the merge driver, which must fail loudly so git falls back to a real conflict
  rather than silently dropping edges).

## Decision

Add a default-feature `cxpak hook` command with three subcommands —
`install`, `post-commit`, `merge-driver` — operating on a canonical, committable
line-oriented edge artifact at `.cxpak/graph.edges`.

- **`post-commit`** regenerates the artifact best-effort and **always exits 0**;
  skippable via `CXPAK_NO_HOOK`. Incrementality is the persisted parse cache; the
  artifact is byte-identical to what the edge-delta hot path would produce
  (asserted).
- **`merge-driver`** writes `union_merge(ours, theirs)` (sorted, deduped, no
  conflict markers) back to git's `%A` path; commutative and deterministic.
- **`install`** wires both into the **target** repo only, idempotently: appends a
  fenced managed block to `.git/hooks/post-commit` (preserving any user hook),
  sets `merge.cxpak-union.{name,driver}` in the repo-local git config, and adds
  `.cxpak/graph.edges merge=cxpak-union` to `.gitattributes`. It writes nothing
  global and never touches this repo's dev hooks.

Plugin shell wrappers (`plugin/lib/cxpak-post-commit`, `plugin/lib/cxpak-merge-driver`)
resolve the binary via `ensure-cxpak` then exec the subcommand — the post-commit
wrapper swallows resolution failures (exit 0), the merge-driver wrapper fails loud.

## Consequences

### Positive
- Committed graph artifact stays fresh automatically; team merges of it are
  conflict-free and deterministic.
- Zero new dependencies; reuses `Scanner`, `parse_with_cache`, `build_with_content`,
  `build_dependency_graph`, and git2 (LOCAL only).
- Post-commit can never break a commit (best-effort, exit 0, env opt-out).

### Negative
- A one-sided edge deletion can linger in the union until the next regeneration
  (acceptable for a self-correcting derived file).
- Post-commit does a full graph assembly, not an in-memory delta (bounded by the
  parse cache; the delta path stays the serve/watch optimization).

### Neutral
- The artifact format is cxpak-internal, not a graph-interchange standard; tool
  interop continues to go through B2's Cypher/GraphML export.

## Revisit if

- The artifact grows large enough that a full graph assembly per commit is
  measurably slow — then persist a resumable index and drive `rebuild_graph_delta`
  cross-process.
- A use case needs deletion-accurate merges before the next regeneration — then
  move to a base-aware 3-way merge.
- A consumer needs the committed artifact in an interchange format — then make the
  canonical artifact a line-oriented projection of the GraphML/Cypher export.
