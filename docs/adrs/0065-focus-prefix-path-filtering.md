---
id: '0065'
title: Add focus param as a starts_with prefix filter on all MCP tools
status: ACCEPTED
date: 2026-03-18
triggered_by: Users needed to scope tool results to a subdirectory
loop: planning
---

# ADR-0065: Add focus param as a starts_with prefix filter on all MCP tools

## Context

In v0.10.0, every MCP tool gains an optional `focus` param scoping results to a path
prefix, implemented via a single `matches_focus(path, focus)` utility using
`path.starts_with`. A trailing slash is tolerated, and `focus` on the root `""` is
equivalent to no focus.

The design also specified special semantics for two tools — `trace` listing
out-of-scope dependencies in an `out_of_scope_deps` array, and `pack_context` flagging
included out-of-scope dependencies as `included_as: out_of_scope_dependency` — but these
were not implemented (see Decision).

## Options considered

- **Option A — prefix match via `path.starts_with`:** A single `matches_focus` utility
  applied as an early filter in each handler. Pros: simple, predictable, covers all use
  cases, and handles a trailing slash either way. Cons: no glob/regex flexibility.

- **Option B — glob or regex path matching:** A reasonable alternative would have been
  to allow patterns like `src/**/*.rs`. Someone could prefer it for more expressive
  scoping. Rejected because it is more complex and the design favors predictability.

## Decision

Add `focus` to every MCP tool, implemented as `path.starts_with` via a shared
`matches_focus` utility (`src/commands/serve.rs:47`) applied as an early filter. The
design called for `trace` to surface out-of-scope deps in an `out_of_scope_deps` array
and for `pack_context` to flag them as `included_as: out_of_scope_dependency`, but
neither was implemented: the shipped `trace` handler only echoes `focus` back without
traversing or listing deps, and `pack_context`'s `included_as` emits only `selected` or
`dependency`.

## Consequences

### Positive
- Uniform scoping across all tools with one utility.

### Negative
- Prefix-only matching cannot express extension or mid-path globs.
- The trace and pack_context out-of-scope handling specified in the design was not
  implemented; `focus` is a plain include/exclude filter in those tools.

### Neutral
- `focus` on the root `""` is equivalent to no focus.

## Revisit if
- Users need glob/extension-based scoping.
- A tool needs out-of-scope semantics (e.g. surfacing rather than dropping skipped deps),
  as the original design intended for trace and pack_context.
