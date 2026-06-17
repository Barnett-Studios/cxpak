---
id: '0121'
title: Three-level semantic zoom with lazy level-3 (top-20 by PageRank for static export)
status: ACCEPTED
date: 2026-04-01
triggered_by: Architecture Explorer view payload sizing (Task 8)
loop: implementation
---

# ADR-0121: Three-level semantic zoom with lazy level-3 (top-20 by PageRank for static export)

## Context

In v2.0.0, the Architecture Explorer view offers three-level semantic zoom: module graph (level 1), per-module file graphs (level 2), and per-file symbol graphs (level 3). The view ships as a self-contained HTML payload with inlined D3.js. Embedding fully-computed symbol-level layouts for every file would explode the HTML file size on large repos. A strategy was needed to bound the payload while keeping the module and file levels always available offline.

## Options considered

- **Option A — lazy level-3, top-20 by PageRank:** Always embed level 1 (module graph) and level 2 (per-module file graphs); embed level 3 (per-file symbol layouts) only for the top-20 files by descending PageRank. The design doc additionally specified an on-demand fallback for the remaining files (loading spinner that POSTs to `/v1/visual` when served, or an "open in interactive mode" message under `file://`). Pros: bounded HTML regardless of repo size, important files keep offline symbol detail, graceful degradation. Cons: files outside the top-20 need a server or interactive mode for symbol detail; the on-demand half was specified but not shipped. This is the chosen approach.
- **Option B — embed all three levels for every file:** A reasonable alternative would have been to precompute and inline symbol layouts for every file. Pros: fully offline at every zoom level. Cons: payload size blows up on large repos, defeating the self-contained-HTML goal. Someone could prefer it for small repos where size is a non-issue.

## Decision

`ArchitectureExplorerData` always embeds level1 (module graph) and level2 (per-module file layouts); level3 (per-file symbol layouts) is embedded only for the top-20 files by descending PageRank (`src/visual/render.rs`, `build_architecture_explorer_data`, ranked and `.take(20)`). In the browser, files without a precomputed level3 are simply not navigable to symbol level: the view controller's `onNodeClick` only descends into level 3 when an entry exists, otherwise it falls back to rendering level1.

NOTE: the design doc specified a loading-spinner + `POST /v1/visual` + "open in interactive mode" fallback for non-top-20 files, but this was NOT implemented in the shipped code. No `spinner`, `/v1/visual` fetch, `XMLHttpRequest`, or interactive-mode message exists in the visual JS; the shipped behavior is a silent fallback to the module-level view.

## Consequences

### Positive
- HTML payload stays bounded regardless of repo size.
- Most-important files (top-20 by PageRank) retain offline symbol detail.

### Negative
- Symbol detail for non-top-20 files is unavailable offline; the explorer silently falls back to the module-level view rather than offering an on-demand fetch.

### Neutral
- Breadcrumb navigation structure is embedded in the JSON payload (`BreadcrumbEntry`, rooted at "Repository").
- The on-demand-POST delivery mechanism was proposed in the design doc but not realized in shipped behavior; consumers should not expect it.

## Revisit if
- The top-20 cutoff proves too small for typical navigation.
- A better on-demand level-3 delivery mechanism is added (the spinner/`POST /v1/visual` path from the design doc remains unimplemented and is itself a candidate for this trigger).
