---
id: '0110'
title: 'Intelligence HTTP API is versioned (/v1/), localhost-bound by default, optional bearer token, path-traversal guarded'
status: ACCEPTED
date: 2026-03-31
triggered_by: v1.6.0 exposes cxpak intelligence as an HTTP API for other tools
loop: planning
---

# ADR-0110: Intelligence HTTP API is versioned (/v1/), localhost-bound by default, optional bearer token, path-traversal guarded

## Context

Introduced in v1.6.0 ("The Platform"), which exposes cxpak intelligence over HTTP so other tools can consume it. Exposing intelligence over the network raises versioning and security concerns: schema stability across releases, network exposure, authentication, and file-path safety since several endpoints accept path parameters.

## Options considered

- **Option A — Versioned `/v1/` prefix, default `127.0.0.1`, optional `--token` bearer, no built-in TLS, path validation:** All endpoints under `/v1/` with stable JSON schemas; bind localhost by default with `--bind` for remote use; an optional `--token` flag enables timing-safe `Authorization: Bearer` validation on all endpoints; HTTPS handled by an external reverse proxy; path parameters validated against the workspace root, rejecting `..` or absolute paths that escape the repo. Pros: the versioned prefix isolates breaking changes to major bumps; secure-by-default localhost binding; auth is opt-in; the path guard prevents traversal; no in-binary TLS complexity. Cons: HTTPS needs an external proxy; bearer-only auth is coarse-grained. Someone could prefer it for shipping a small, safe-by-default surface.

- **Option B — Unversioned API, bind all interfaces, no auth:** A reasonable alternative would have been to expose endpoints at the root, listen on `0.0.0.0`, and run without a token. Pros: simplest possible thing to consume. Cons: breaking changes are unmanaged; insecure default network exposure; no auth or path safety. Someone could prefer it only for a throwaway local prototype.

## Decision

Expose the Intelligence API under a versioned `/v1/` prefix with stable JSON schemas (breaking changes require a major version bump). Default bind to `127.0.0.1`, with a `--bind` flag for non-local deployment; an optional `--token` flag enables `Authorization: Bearer` validation on all endpoints, shipped as a timing-safe (constant-time) comparison. No built-in TLS — use a reverse proxy for HTTPS. File path parameters are validated against the workspace root, rejecting paths containing `..` or absolute paths that escape the repo (canonicalization catches symlink escapes). All endpoints accept `workspace` and `focus` parameters.

## Consequences

### Positive
- The versioned prefix contains breaking changes to major releases.
- Secure-by-default localhost binding with opt-in auth.
- Path-traversal guard on file parameters.

### Negative
- HTTPS requires an external reverse proxy.
- Bearer-token auth is coarse-grained (all-or-nothing, no per-endpoint scopes).

### Neutral
- Non-local binds are gated to require a token (`validate_bind_security`), coupling network exposure to authentication.

## Revisit if
- Finer-grained auth (per-endpoint, scopes) is needed.
- Built-in TLS becomes a requirement.
