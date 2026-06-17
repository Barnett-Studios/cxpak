---
id: '0162'
title: 'v1 API parameter validation: two normalizers (path vs symbol), JSON error envelope, and caps'
status: ACCEPTED
date: 2026-04-17
triggered_by: Wiring 9 v1 stub handlers to real intelligence functions that accept user-controlled params
loop: planning
---

# ADR-0162: v1 API parameter validation: two normalizers (path vs symbol), JSON error envelope, and caps

## Context
In v2.1.0 the nine previously-stubbed `POST /v1` handlers were wired to real intelligence functions that accept user input: paths, symbols, file lists, and depth. Path parameters and symbol parameters have different legal character sets — a symbol like `Vec<String>` legitimately contains `<>:()` that a path whitelist must reject. Unbounded inputs (large file lists, deep BFS) could drive graph traversal over the whole repo or exhaust work per request.

## Options considered
- **Option A — two distinct normalizers + caps + structured JSON error envelope:** `normalize_path_param` (strict whitelist: alphanumerics/`_-./`, reject standalone `..` segments, absolute paths, backslash, NUL, >1024 chars) vs `normalize_symbol_param` (permit `<>:()` etc., reject control chars and `/ \ ` `` ` `` `$ ; |`, >512 chars); `Vec` inputs capped at 100; depth capped at 10; all errors emitted as `{error, message}` JSON via a shared `v1_error` helper. Pros: symbols like `Vec<String>` accepted while paths stay safe; bounded work; consistent machine-readable errors. Cons: two code paths to keep in sync. This is the chosen option.
- **Option B — single normalizer for all params:** one whitelist for every param type. Pros: simpler. Cons: either rejects legitimate symbols (`Vec<String>`) or weakens path safety — you cannot satisfy both with one character set. Someone could prefer it for code simplicity, but it forces a bad trade-off.

## Decision
Validate `/v1` params with two normalizers (`src/commands/serve.rs`): a strict path whitelist (`alphanumerics/_-./`, reject standalone `..` segments, absolute paths, backslash, NUL, `>1024` chars) and a broader symbol allowlist (permit `<>:()` etc., reject only control chars and `/ \ ` `` ` `` `$ ; |`, `>512` chars). Empty strings normalize to `None` centrally; `Vec` inputs cap at 100; depth caps at 10 across handlers (default 3 for `predict`, 6 for `data_flow`). All 4xx/5xx responses use a shared `v1_error` helper returning `{"error": code, "message": ...}` JSON with a fixed set of error codes. Success responses are wrapped in named envelopes for forward-extensibility.

## Consequences
### Positive
- Generic-heavy symbols accepted; path traversal blocked; BFS bounded by the uniform depth cap of 10.
- Machine-readable, consistent error and success shapes across all `/v1` endpoints.
### Negative
- Two normalizers must stay aligned as new params are added.
### Neutral
- Shipped `serve.rs` confirms `v1_error`, `normalize_path_param`, and `normalize_symbol_param`.

## Revisit if
- Observed monorepo paths exceed 1024 chars or symbol signatures exceed 512.
- The `workspace` param starts being consumed by handlers (v2.2.0+).
