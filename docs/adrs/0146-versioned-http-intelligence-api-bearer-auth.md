---
id: '0146'
title: Versioned /v1/ HTTP Intelligence API with bearer-token auth and path-traversal defense
status: ACCEPTED
date: 2026-04-01
triggered_by: v1.6.0 exposing the intelligence layer over a stable, authenticated HTTP API
loop: implementation
---

# ADR-0146: Versioned /v1/ HTTP Intelligence API with bearer-token auth and path-traversal defense

## Context

Shipped in v1.6.0. Beyond the MCP/stdio surface, the team wanted a network-reachable HTTP API for the intelligence features (editors, CI, dashboards). A network surface needs versioning for stability, authentication, and defense against path traversal when callers reference workspace files.

## Options considered

- **Option A — `/v1/`-prefixed routes on the existing axum router, bearer-token auth middleware, `validate_workspace_path` guard:** All `/v1/` routes are POST (except a GET health), gated by a `from_fn_with_state` auth layer that checks a configured `--token`; the path guard rejects `..`, absolute paths, and paths escaping the workspace root. Pros: the versioned prefix gives an API stability contract; reuses the running `serve` router and hot index; auth is opt-in via `--token`; explicit traversal defense. Cons: stub handlers initially returned `available_from` placeholders until each version's data landed. This is the design the plan chose and the shipped implementation.
- **Option B — Unversioned routes reusing the existing MCP HTTP handlers:** Expose intelligence under flat paths with no version prefix. Pros: less routing code. Cons: no API stability contract for downstream consumers. A reasonable alternative would have been this if API stability were not a concern; the plan did not weigh it, instead choosing the `/v1/` namespace outright.

## Decision

Extend the existing axum router in `serve.rs` with a `/v1/` prefix mounting 12 intelligence endpoints (all POST except a GET health in the shipped version), guarded by a bearer-token auth middleware (`check_auth`: open when no token is configured, else timing-safe exact match) and a `validate_workspace_path` guard that rejects `..`, absolute paths, and paths escaping the workspace root. Add `--bind` (default `127.0.0.1`) and `--token` (default `None`) flags to `cxpak serve`. `AppState` gains `expected_token` and `workspace_root`.

## Consequences

### Positive
- Stable, versioned network API for editors, CI, and dashboards.
- Bearer auth and path-traversal defense for a network-exposed surface.
- Reuses the running `serve` router and shared index.

### Negative
- Several `/v1/` handlers shipped as `available_from` stubs until later versions filled them in.
- Default bind `127.0.0.1` plus an optional token means auth is off unless explicitly configured.

### Neutral
- Shipped `CLAUDE.md` documents 12 endpoints, all now wired to real intelligence functions, with timing-safe bearer auth and a 2MB body limit.

## Revisit if
- A `/v2/` API is needed for breaking changes.
- Auth needs to be mandatory or to use more than a single shared token.
