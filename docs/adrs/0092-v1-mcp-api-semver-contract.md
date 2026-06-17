---
id: '0092'
title: Freeze the MCP tool API under semver at v1.0.0; CLI/HTTP/internal structure remain unstable
status: ACCEPTED
date: 2026-03-22
triggered_by: v1.0.0 release — plugins need to depend on stable tool schemas
loop: planning
---

# ADR-0092: Freeze the MCP tool API under semver at v1.0.0; CLI/HTTP/internal structure remain unstable

## Context

Released in v1.0.0. cxpak exposes several surfaces: MCP tools (consumed by plugins and agents), a CLI, an HTTP intelligence API, and `.cxpak/` on-disk file formats. Plugins built against cxpak need a contract they can depend on across the 1.x line, but freezing every surface would ossify the CLI output and HTTP endpoints, which are positioned as convenience layers rather than the canonical contract.

The decision defines exactly what counts as a breaking change requiring a 2.0 and what may evolve freely within 1.x.

## Options considered

- **Option A — Stabilize MCP only; leave CLI/HTTP/internal free to change (chosen):** MCP tool names, required parameters, and response field names/types are frozen in 1.x; adding optional params, new tools, new response fields, and new languages is allowed. Internal Rust module structure, CLI output format, `.cxpak/` file formats, and HTTP endpoint paths/formats are explicitly outside the public API. Pros: plugins get a stable contract while internal refactors, CLI tweaks, and HTTP changes stay non-breaking. Cons: HTTP consumers get no stability guarantee. Someone could prefer this because MCP is the actual integration contract and the other surfaces are convenience.
- **Option B — Freeze everything (MCP + CLI + HTTP):** A reasonable alternative would have been to declare all public-facing surfaces stable in 1.x. Pros: maximal guarantees for every kind of consumer. Cons: ossifies CLI output and HTTP endpoints that the design treats as convenience around the MCP contract, blocking iteration on them without a major bump. Someone could prefer it if CLI/HTTP had a large external consumer base needing stability.

## Decision

Establish a v1.0.0 semver contract:

- **Stable in 1.x:** MCP tool names, required parameters, and response structure (field names and types).
- **Allowed (non-breaking) in 1.x:** new optional parameters, new tools, new response fields, and new supported languages.
- **Breaking (requires 2.0):** removing or renaming tools or required parameters, changing response field types, or removing response fields.
- **Not part of the public API:** internal Rust module structure, CLI output format, `.cxpak/` file formats, and HTTP endpoint paths/formats — all may change freely.

## Consequences

### Positive
- Plugins can depend on stable MCP tool schemas across the 1.x line.
- Internal refactors, CLI tweaks, and HTTP changes remain non-breaking.

### Negative
- HTTP API consumers have no stability guarantee.
- Future tool redesigns are gated behind a major version bump.

### Neutral
- MCP is the contract; the CLI and HTTP API are positioned as convenience layers.

## Revisit if
- The HTTP API gains enough external consumers to warrant its own stability tier.
- A needed change requires breaking a frozen MCP tool, forcing a 2.0.
