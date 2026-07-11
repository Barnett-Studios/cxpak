---
id: '0189'
title: HTTP serve auth covers all data routes, not just /v1
status: ACCEPTED
date: 2026-07-09
triggered_by: Adversarial security review of the 3.0.0 release branch (auth-scope finding)
loop: implementation
---

# ADR-0189: HTTP serve auth covers all data routes, not just /v1

## Context

`cxpak serve` refuses a non-loopback bind without a token
(`validate_bind_security`, ADR-0158): the error tells the operator a
non-loopback listener "MUST be authenticated with a non-empty bearer token."
But the bearer middleware was attached only inside `build_v1_router`
(`.route_layer(...)`), so it guarded `/v1/*` alone. The legacy Intelligence
routes on the outer router — `/diff`, `/auto_context`, `/overview`,
`/search`, … — carry the same source-bearing payloads and were reachable
**unauthenticated** on a tokened non-loopback bind. The startup guard made a
security promise the request path did not keep.

This is a human-facing security-boundary decision: which routes a configured
token protects is documented behavior (README, prior text scoped it to
`/v1/*`), and widening it changes what an operator relying on the old scope
observes.

## Options considered

- **Guard every route except `/health` (chosen):** apply the bearer layer to
  all legacy data routes and keep the v1 layer, leaving only `/health` open as
  a liveness probe. Pro: honors `validate_bind_security`'s promise; a token now
  authenticates the whole listener; `check_auth(None, _) == true` keeps the
  no-token/loopback default fully open (byte-identical behavior). Con: an
  operator who deliberately relied on unauthenticated legacy routes behind a
  token must now send the token there too.
- **Document the gap instead of closing it:** state that the token only guards
  `/v1/*` and legacy routes are loopback-only. Rejected — the routes are on the
  same listener and `validate_bind_security` already promises authentication;
  documenting a source-disclosure hole as intended is worse than closing it.
- **Guard every route including `/health`:** simplest (one layer, no carve-out)
  but breaks unauthenticated liveness probes that orchestrators rely on.
  Rejected for the `/health` carve-out.

## Decision

Apply the bearer-auth `route_layer` to all legacy data routes (via a hoisted
`bearer_auth_layer`), keep the equivalent layer on `/v1/*`, and leave only
`/health` unauthenticated. Auth is attached directly to `.route()`-registered
routes (no `.merge` between the routes and their `route_layer`), so coverage is
unambiguous. With no `--token`, `check_auth` returns true for every route, so
the default local path is unchanged.

## Consequences

### Positive
- A configured token authenticates the entire listener; the source-disclosure
  path on a non-loopback tokened bind is closed.
- `validate_bind_security`'s guarantee is now truthful.
- Default no-token / loopback behavior is byte-identical.

### Negative
- Any caller that hit a legacy route without a token on a tokened server now
  gets 401 and must supply the token.

### Neutral
- `/v1/health` remains behind auth exactly as before (unchanged); only the
  legacy `/health` is the open liveness probe.

## Revisit if
- A future need arises for an unauthenticated read-only route beyond `/health`
  (then carve it out explicitly, as `/health` is).
- The HTTP surface gains per-route scopes/roles (then a single all-or-nothing
  bearer layer is replaced by a scope-aware authorizer).
