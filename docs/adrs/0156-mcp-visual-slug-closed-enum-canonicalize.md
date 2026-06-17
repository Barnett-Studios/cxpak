---
id: '0156'
title: MCP visual_type slug validated against a closed enum returning &'static str, plus canonicalize-and-verify on the output path
status: ACCEPTED
date: 2026-04-17
triggered_by: MCP cxpak_visual type param is interpolated into a written file path
loop: planning
---

# ADR-0156: MCP visual_type slug validated against a closed enum returning &'static str, plus canonicalize-and-verify on the output path

## Context

Designed for cxpak v2.1.0. The MCP `cxpak_visual` tool interpolates its `type` parameter into
`format!("cxpak-{}.html", slug)` and writes the file under `.cxpak/visual/` when output exceeds the
1 MiB inline limit. As user-controlled input, an unvalidated slug enables path traversal
(`../etc/passwd`) or symlink escape.

## Options considered

- **Option A — Closed-enum validator returning `&'static str` plus a canonicalize prefix assert:**
  `validate_visual_type_slug` matches the 7 known slugs and returns a `&'static str`, so no
  user-controlled bytes reach the path; after `join`, canonicalize the result and assert it starts
  with the canonicalized `.cxpak/visual/` prefix. Pros: no user bytes in the path, belt-and-suspenders
  against symlinked base paths. Cons: adds a small enum plus a canonicalize step. Chosen.
- **Option B — Sanitize the slug string (strip dangerous chars):** A reasonable alternative would
  have been to filter or escape the user slug before `format!`. Pros: no enum needed. Cons: sanitizers
  are error-prone and user-controlled bytes still reach the path. Someone could prefer it as less
  code, but it keeps the attacker's input in the filesystem path.

## Decision

Validate the MCP `visual_type` slug against a closed enum
(`dashboard`/`architecture`/`risk`/`flow`/`timeline`/`diff`/`all`) via `validate_visual_type_slug`,
which returns a `&'static str` so no user-controlled bytes reach the path
(`src/commands/serve.rs:539`). Additionally, at the write site (inside the >1 MiB inline-limit
branch), canonicalize the `.cxpak/visual` directory and the joined file's parent, and reject with a
path-escape error if the canonicalized file path does not start with the canonicalized directory,
defending against symlinked base paths. Malicious slugs (`../etc/passwd`, `/etc/passwd`,
`dashboard/../foo`, NUL) error before any file write
(`tests/mcp_slug_validation.rs`).

## Consequences

### Positive
- Path traversal and symlink escape are both blocked before writing.
- Hardens the already-shipped behavior of writing visual output to file when it exceeds 1 MiB.

### Negative
- New slug values require updating the enum.

### Neutral
- Shipped `serve.rs` confirms `validate_visual_type_slug` returning `&'static str` and its use at the
  write site.

## Revisit if
- A new visual type is added (extend the enum).
