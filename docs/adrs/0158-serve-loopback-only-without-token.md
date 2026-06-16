---
id: '0158'
title: cxpak serve without --token binds loopback only and rejects non-loopback binds; header-only bearer auth
status: ACCEPTED
date: 2026-04-17
triggered_by: Wiring 9 v1 endpoints to real intelligence exposes data over HTTP
loop: planning
---

# ADR-0158: cxpak serve without --token binds loopback only and rejects non-loopback binds; header-only bearer auth

## Context
In v2.1.0 the nine `/v1` endpoints were wired to real codebase intelligence functions. With real data flowing over HTTP, an unauthenticated server bound to a non-loopback address would expose that data to the network. Tokens carried in URLs also leak into logs, referers, and shell history. The v2.0.0 bearer middleware (timing-safe compare) was sound but did not constrain bind address or token transport.

## Options considered
- **Option A — loopback-only default + reject non-loopback without token + header-only auth:** when `--token` is unset, bind `127.0.0.1` only and warn; reject any non-loopback bind (`0.0.0.0`, `::`, LAN, public) without a token at startup via `SocketAddr::ip().is_loopback()`; accept tokens only via `Authorization: Bearer`; keep the timing-safe compare; never log tokens. Pros: safe-by-default; no token leakage through URLs/logs (auth reads the `Authorization` header only). Cons: users wanting LAN/remote exposure must set a token. This is the chosen option.
- **Option B — bind `0.0.0.0` by default, optional token:** A reasonable alternative would have been to bind all interfaces and authenticate only when a token is supplied. Pros: convenient for remote access out of the box. Cons: unauthenticated exposure of codebase intelligence to anyone on the network. Someone could prefer it for a trusted-LAN dev convenience, but it fails safe-by-default.

## Decision
Preserve the v2.0.0 bearer middleware and add a startup bind-security check. When `--token` is unset (empty string counts as unset), bind `127.0.0.1` only and warn. Reject any non-loopback bind without a token at startup: `validate_bind_security()` (`src/commands/serve.rs`) errors when `!addr.ip().is_loopback() && effective_token.is_none()`, called before binding. Accept tokens only via the `Authorization: Bearer <token>` header — the `/v1` auth layer reads the token from that header only, so query-param (`?token=...`) and body-field auth never authenticate. Keep the constant-time (`subtle::ConstantTimeEq`, length-prefixed) comparison. The design is such that tokens are never written to logs, spans, error bodies, or CLI output.

## Consequences
### Positive
- Safe-by-default network posture; tokens cannot leak through URLs, logs, or referers because auth reads the `Authorization` header only.
### Negative
- Non-loopback serving now requires explicitly setting a token.
### Neutral
- Shipped `serve.rs` confirms the `is_loopback` + `effective_token.is_none()` startup check.

## Revisit if
- A deployment needs authenticated non-loopback serving with a different auth scheme.
