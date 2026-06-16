---
id: '0082'
title: Extract API surface as signatures-only plus regex-based HTTP route detection for 12 frameworks
status: ACCEPTED
date: 2026-03-22
triggered_by: Need a high-signal, compact context view of a codebase's external contracts (cxpak_api_surface)
loop: planning
---

# ADR-0082: Extract API surface as signatures-only plus regex-based HTTP route detection for 12 frameworks

## Context

cxpak v0.13.0 adds `cxpak_api_surface`, which gives an LLM a compact, high-signal view of a
codebase's external contracts. The API surface comprises public symbol signatures (no bodies),
HTTP routes detected by one regex per framework across 12 frameworks with auto-detection, and
gRPC services / GraphQL types pulled from existing parsers.

Bodies are excluded based on ICSE 2026 research finding that signatures are the highest-value
context and that bodies add noise. The logic lives in `src/intelligence/api_surface.rs`
(`extract_api_surface`, `detect_routes`).

## Options considered

- **Option A — signatures-only plus 12 regex route patterns, auto-detect:**
  emit public symbols as signature + doc comment (no bodies), sorted by PageRank; run one regex
  per framework against file content with file-glob pre-filters (e.g. Django/Rails/Phoenix), and
  report whichever frameworks match. This is compact and high-signal, each framework costs only
  one regex, and it is zero-config. The cost is that regex route detection can miss or overmatch,
  and signatures omit implementation detail. Chosen.

- **Option B — include symbol bodies:** pack the full bodies of public symbols. Someone could
  prefer this for more implementation detail. Rejected: ICSE 2026 research finds API information
  is the highest-value context and bodies add noise.

- **Option C — fewer frameworks / a configured framework:** support a smaller set or require
  configuration to pick the framework. Someone could prefer this for less code. Rejected: each
  framework is a single regex, so the incremental cost of supporting all 12 is minimal, and
  requiring configuration breaks the zero-config goal.

## Decision

Implement `extract_api_surface()` returning public symbols (signatures plus doc comments,
sorted by PageRank, no bodies); HTTP routes via `detect_routes()` with one regex per framework
for 12 frameworks (auto-detected, with file-glob pre-filters for Django/Rails/Phoenix); plus
gRPC services and GraphQL types from existing parsed symbols. All output is token-budgeted via
the v0.11.0 degradation machinery.

## Consequences

### Positive
- Compact, high-signal context aligned with the research.
- 12 frameworks at minimal cost and with no configuration.
- Highest-PageRank files survive budget cuts first.

### Negative
- Regex route detection is approximate; the Echo pattern needed widening (and a Go
  import-guard pre-filter) to avoid single-character false positives.
- Markdown-escaped pipes in the spec's route patterns must be de-escaped by the implementer.

### Neutral
- Exposed as MCP `cxpak_api_surface` and `GET /api_surface`. (The design doc intended a POST
  route, but the shipped HTTP endpoint registers `get(api_surface_handler)`.)

## Revisit if
- A framework's routes are systematically missed by its regex.
- Signature-only context proves insufficient for some class of tasks.
