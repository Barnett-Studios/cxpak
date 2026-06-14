---
id: '0084'
title: Compute blast radius via reverse BFS with a multiplicative risk score and single-category assignment
status: ACCEPTED
date: 2026-03-22
triggered_by: Need a 'what breaks if I change this' impact tool (cxpak_blast_radius)
loop: planning
---

# ADR-0084: Compute blast radius via reverse BFS with a multiplicative risk score and single-category assignment

## Context

cxpak v0.13.0 adds `cxpak_blast_radius`, a "what breaks if I change this" impact tool. It
walks reverse edges (dependents) from changed files, assigns each affected file to exactly one
category, and scores risk multiplicatively. The logic lives in
`src/intelligence/blast_radius.rs`.

Categories are assigned by priority: `test_files` → `direct_dependents` → `schema_dependents`
→ `transitive_dependents`. Risk combines hop decay, per-edge-type weight, file PageRank, and a
1.2× penalty for untested files, clamped to `[0,1]`, with high/medium thresholds at 0.7/0.3.

## Options considered

- **Option A — reverse BFS + multiplicative risk + single category:**
  BFS over `dependents()`, with `risk = hop_decay × edge_weight × pagerank × test_penalty`
  clamped to `[0,1]`, and each file placed in exactly one category (highest priority wins). This
  answers the actual question (what depends on this), gives an intuitive combined risk, and keeps
  the output clean for LLM reasoning. The cost is that single-category assignment hides files
  that are, for example, both a test and a schema dependent. Chosen.

- **Option B — forward BFS:** walk the dependencies of the changed files. Someone could prefer
  this to show what the change relies on. Rejected: that is the wrong question — change impact is
  a reverse-graph walk.

- **Option C — multi-category membership:** let a file appear in every applicable category.
  Someone could prefer this for a more complete view. Rejected: it is messier for LLM reasoning
  than one category per file.

## Decision

Implement `compute_blast_radius()` as a reverse BFS over the typed graph that tracks hop count
and places each affected file into exactly one of `test_files` / `direct_dependents` /
`schema_dependents` / `transitive_dependents` (highest priority wins). Compute risk as
`hop_decay × edge_weight (1.0/0.8/0.6/0.5 by edge type) × file_pagerank × (1.2 if untested else 1.0)`,
clamped to `[0,1]`, with thresholds high ≥ 0.7 and medium ≥ 0.3. The risk function shipped as
`compute_blast_impact()` (the design doc proposed the name `compute_risk`, but it was renamed to
disambiguate from `risk::compute_risk_ranking()`, which answers a different question).

## Consequences

### Positive
- Directly answers the change-impact question via a reverse walk.
- Per-edge-type weights let schema/migration edges carry less risk than imports.
- Single-category assignment keeps results clean for the LLM.

### Negative
- The 1.2× untested penalty is a coarse heuristic.
- Single-category assignment loses dual-role nuance.

### Neutral
- When multiple edges connect two files, the highest-risk edge wins.
- The risk function is named `compute_blast_impact` as shipped, not `compute_risk`.
- Exposed both as MCP `cxpak_blast_radius` and `POST /blast_radius`.

## Revisit if
- Single-category assignment proves to hide important relationships.
- The risk thresholds mis-classify in practice.
