---
id: '0169'
title: Publish a versioned context-format schema over the existing JSON output, not a new format
status: ACCEPTED
date: 2026-06-14
triggered_by: v2.3.0 W2 — the roadmap's "context format standard" was never built; cxpak already emits JSON
loop: planning
---

# ADR-0169: Versioned context-format schema over the existing JSON output

## Context

cxpak already renders structured output as markdown, JSON, and XML (`output/`), and v2.0.0 established semver stability for the MCP API (tool names, params, response structures stable across 2.x). The roadmap's "context format interchange standard" (old Priority 9) was never built. The temptation is to design a new interchange format; the existing JSON output already *is* the de-facto format and is API-stable.

## Options considered

- **Option A — document + version the existing JSON (chosen):** add a top-level `format_version` field to the JSON output, publish a JSON Schema for it in `docs/`, and add `cxpak --emit-schema` (or `cxpak schema`) to print it. Pros: elevates what already exists and is already stable; consumers get a machine-checkable contract; zero new serialization. Cons: commits us to schema evolution discipline (already implied by the v2.x API promise). Chosen because it satisfies the interchange goal by formalizing reality.
- **Option B — design a new interchange format:** a fresh schema distinct from the current output. Pros: a clean slate unconstrained by current shapes. Cons: duplicates the existing JSON, splits the surface, and directly violates the release's extend-don't-add constraint. Someone could prefer it only if the current JSON were unfit — it is not.
- **Option C — do nothing:** Pros: no work. Cons: no published contract, no ecosystem adoption path. Someone could prefer it if the standard play is deprioritized.

## Decision

Option A. Version and publish the existing JSON output as the interchange schema; add `format_version` and a schema-emit command. No new format.

## Consequences

### Positive
- A machine-checkable, versioned contract that other tools can target.
- No new serialization code; the existing renderer gains one field.

### Negative
- We now owe schema-evolution discipline: additive changes bump the minor schema version, breaking changes bump major with a documented migration.

### Neutral
- `format_version` is additive; existing consumers ignore unknown fields.

## Revisit if
- The JSON shape needs a breaking change — bump `format_version` major and ship a migration note; do not silently reshape.
- An external standard emerges that the ecosystem converges on — map to it rather than maintain a parallel one.
