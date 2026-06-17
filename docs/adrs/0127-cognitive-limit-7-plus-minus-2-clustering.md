---
id: '0127'
title: Cap graph layers and onboarding phases at 9 nodes (7±2 cognitive limit)
status: ACCEPTED
date: 2026-04-01
triggered_by: Layout readability and onboarding phase sizing
loop: implementation
---

# ADR-0127: Cap graph layers and onboarding phases at 9 nodes (7±2 cognitive limit)

## Context
Introduced in v2.0.0. Dense graph layers and large reading phases overwhelm comprehension. The Sugiyama layout engine (`src/visual/layout.rs`) and the onboarding phase grouping (`src/intelligence/onboarding.rs`) both produce node/file groupings that, left unbounded, become unreadable. A single consistent cap grounded in the 7±2 cognitive-load heuristic was needed across both subsystems so that visual layers and reading phases stay within working-memory limits.

## Options considered
- **Option A — Cap at 9 (7±2) with tail clustered into a Cluster/sub-phase node:** `LayoutConfig.max_nodes_per_layer` defaults to 9; layers exceeding it group the tail into a `NodeType::Cluster` node, and `group_into_phases` splits any module group >9 files into ≤9-file sub-phases suffixed `(N/M)`. Pros: bounded visual and reading complexity grounded in a known heuristic, consistent across layout and onboarding. Cons: an arbitrary fixed threshold, and clustering hides nodes behind an extra interaction. Someone could prefer it for the predictable, uniform bound it gives both views.
- **Option B — No cap, render all nodes per layer:** A reasonable alternative would have been to show every node regardless of layer width. Pros: nothing is hidden. Cons: dense layers become unreadable and onboarding phases grow oversized. Someone could prefer it to avoid the indirection of expandable clusters and guarantee full visibility.

## Decision
`LayoutConfig.max_nodes_per_layer` defaults to 9 (7±2). `enforce_cognitive_limit` keeps `max_per_layer - 1` nodes and groups the layer tail into a `NodeType::Cluster { member_ids, .. }` node, preserving the clustered members' ids. The same cap is reused for onboarding: `group_into_phases` (with `MAX_PHASE_SIZE = 9`) splits any module group exceeding 9 files into chunks of ≤9 files, each named `<Module> (N/M)`.

## Consequences
### Positive
- Bounded per-layer and per-phase complexity in line with the 7±2 heuristic.
- Clustered tails preserve `member_ids` for later expansion, so no node information is lost.
### Negative
- The threshold is fixed at 9 and does not adapt to graph density.
- Clustered or sub-phased content requires an extra step to reach.
### Neutral
- The same 9-node cap is shared between the layout engine and onboarding phase splitting, rather than being tuned independently per view.

## Revisit if
- User feedback shows 9 is the wrong cap for either the layout or onboarding view.
- An adaptive, density-based cap is desired instead of a fixed constant.
