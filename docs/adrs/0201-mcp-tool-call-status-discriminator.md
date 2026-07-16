---
id: '0201'
title: Machine-readable MCP tool-call status discriminator (retryable vs terminal)
status: ACCEPTED
date: 2026-07-16
triggered_by: issue #19 (no machine-readable discriminator between retryable indexing and terminal errors)
loop: planning
---

# ADR-0201: Machine-readable MCP tool-call status discriminator

## Context

On the MCP surface a `tools/call` that arrives before the background index is
ready returns a human-readable message with **no machine-readable discriminator**
distinguishing (a) retryable "indexing in progress", (b) terminal "indexing
failed — restart", and (c) terminal parameter/capability errors. A client can
only tell "retry soon" from "give up" by substring-matching the prose — a wording
change silently breaks retry logic (issue #19 documents a client pinning the exact
string in a unit test for exactly this reason).

REPRO.md corrects the issue's cited root cause and it was independently
re-verified: Building/Failed do **not** flow through `mcp_tool_error`
(serve.rs:4213). They flow through `mcp_tool_result` (serve.rs:2575) and carry
**no `isError`** — so they are indistinguishable not only from each other but from
a genuine success result, except by prose. Parameter/capability errors *do* set
`isError:true` (`mcp_tool_error`, serve.rs:4214). The current
`snapshot_ready_index` collapses both not-ready states into a bare
`Result<_, String>` (serve.rs:2427), discarding retryable/terminal at the type
level before the caller can act on it.

Crucially, `mcp_tool_error` is a **shared** helper for many terminal conditions —
param validation *and* capability-execution failures, invalid regex, IO/render
errors (serve.rs:2828/2839/3127/3475/3576/4143/4178). So a discriminator cannot be
attached at that helper without mislabelling a disk-write failure as
"invalid_params". The design must add a positive signal only where one is actually
missing.

Human decision: it picks the on-the-wire contract machine clients will depend on,
and whether to change an existing envelope (Failed → `isError`).

## Options considered

- **Option A — Stable status strings in `text`, documented:** cheapest; clients
  still parse prose. Rejected — prose parsing is exactly the fragility the issue is
  about.
- **Option B — Tag every result/error with `structuredContent{status,retryable}`,
  including at the shared `mcp_tool_error` helper:** uniform, but tagging the shared
  helper labels IO/render/capability failures as `invalid_params` — a factual lie,
  and `retryable:false` isn't even universally right (a transient IO error could be
  retryable). Rejected: it bakes a wrong contract into the exhaustive table test.
- **Option C — Positive machine signal only where it's missing (chosen):** the
  only state that is "neither an error nor a real answer" is **Building** — it
  currently has no `isError` and no marker, so a machine client cannot detect it.
  Give *Building* a positive marker: `mcp_tool_result` +
  `structuredContent{"status":"indexing","retryable":true}`. Make *Failed* terminal
  the standard way: `mcp_tool_error` (`isError:true`) +
  `structuredContent{"status":"failed","retryable":false}`. Leave generic
  param/capability/IO errors exactly as they are (`isError:true`, no
  `structuredContent`) — already terminal via the standard flag, and not
  mislabelled. Success stays a bare `mcp_tool_result` with **no** `structuredContent`.
  Client rule becomes a clean three-way branch with no prose: `retryable === true`
  → retry; else `isError === true` → terminal, don't retry; else → success.

`structuredContent` (a first-class MCP `CallToolResult` field) is chosen over
`_meta` (out-of-band/experimental) as the "the client should read this" channel;
the cost of being wrong is low (additive field).

## Decision

Option C. Change `snapshot_ready_index` to `Result<Arc<CodebaseIndex>, NotReady>`
with `NotReady { Indexing, Failed(String) }` (typed, with a `message()` accessor
reproducing today's human prose **byte-for-byte**). At the tool-call site
(serve.rs:2575): `Indexing` → non-error result + `{status:"indexing",
retryable:true}`; `Failed` → `isError:true` + `{status:"failed",retryable:false}`
(an intentional, ADR'd envelope change — Failed is a terminal error state).
Param/capability/IO errors are untouched. Success is untouched (no
`structuredContent`). The readiness gate runs before param validation, so during
Building even a malformed call reports `retryable:true`; this is bounded and
self-correcting — once `Ready`, the same call reaches validation and returns the
terminal `isError:true`, so a conformant client stops retrying (documented + tested,
not left implicit).

## Consequences

### Positive
- A client distinguishes retry / restart / fix-your-call via a three-way branch on
  `retryable`/`isError`, never on prose.
- No mislabelling: generic terminal errors keep their honest `isError:true` and
  gain no false `invalid_params` tag.
- Typed `NotReady` makes the distinction unloseable at the call site; `message()`
  keeps the Failed prose byte-identical.

### Negative
- One intentional envelope change: Failed now sets `isError:true` (it previously
  had none). No existing test asserts its absence (verified:
  tests/mcp_nonblocking_startup.rs:166-181 checks `error==null`+text only), but a
  client relying on Failed-as-normal-result must adapt. Documented in the PR.
- During Building, a permanently-malformed call is reported retryable until the
  build finishes; bounded by index build time, then terminal. Documented + tested.

### Neutral
- No new dependency; JSON shape only. No effect on CLI/HTTP/LSP (MCP-specific).

## Revisit if
- The MCP spec deprecates/relocates `structuredContent` — move the marker.
- Clients need more states (e.g. a distinct "enriching" while `ReadyEnriched`
  builds) — extend `status`; `retryable` stays the primary branch key.
- Param validation is hoisted before the readiness gate (a larger dispatch change)
  — then Building would no longer mask param errors and the "self-correcting" note
  is moot.
