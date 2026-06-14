---
id: '0057'
title: Wire MCP server into the Claude Code plugin via .mcp.json + ensure-cxpak-serve wrapper
status: ACCEPTED
date: 2026-03-17
triggered_by: MCP tools must be available natively inside the Claude Code plugin
loop: planning
---

# ADR-0057: Wire MCP server into the Claude Code plugin via .mcp.json + ensure-cxpak-serve wrapper

## Context

As of v0.9.0, cxpak already has an MCP server (`serve --mcp`). To expose it natively inside the Claude Code plugin, the plugin needs an MCP config that launches the binary, and that binary must be resolved or installed reliably on the user's machine. The plugin already ships an `ensure-cxpak` script that resolves the binary via PATH, then a cached install, then an auto-download.

## Options considered

- **Option A — `ensure-cxpak-serve` wrapper invoked from `.mcp.json` over stdio:** `.mcp.json` declares a stdio server whose command is `${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak-serve`, a bash wrapper that resolves the binary via the existing `ensure-cxpak` (PATH → cached install → auto-download) then `exec`s `cxpak serve --mcp`. Pros: reuses the existing auto-download/binary-resolution logic, `exec` keeps stdio flowing directly, it is portable, and it coexists with the existing slash commands and skills. Cons: adds a shell indirection layer and relies on bash availability.

- **Option B — Hardcode the cxpak path in `.mcp.json`:** Point `.mcp.json` directly at a cxpak binary path. A reasonable alternative would have been this, since it needs no wrapper script. Cons: not portable, no auto-install, and it breaks if the binary is not on the expected path. (Not formally evaluated; reconstructed here.)

- **Option C — Replace the slash commands/skills with MCP tools:** Drop the existing `/overview`, `/trace`, `/diff` commands and skills in favor of MCP tools only. A reasonable alternative would have been a single code path to maintain. Cons: removes a path some users prefer and is a breaking change. (Not formally evaluated; the design doc only affirmatively decides to keep the slash commands and mentions MCP-preference migration as a future, post-v0.9.0 enhancement.)

## Decision

Add `plugin/.mcp.json` declaring a stdio MCP server whose command is `${CLAUDE_PLUGIN_ROOT}/lib/ensure-cxpak-serve`. That wrapper resolves the binary via the existing `ensure-cxpak` script then `exec`s `cxpak serve --mcp`, so stdio flows directly between Claude Code and cxpak. The existing slash commands and skills are kept as a parallel path with no added maintenance burden, letting users choose.

## Consequences

### Positive
- Reuses the battle-tested binary resolution and auto-download logic.
- `exec` keeps stdio direct between Claude Code and cxpak.
- MCP tools coexist with the existing commands and skills; no breaking change.

### Negative
- Adds a bash wrapper indirection that depends on a POSIX shell.

### Neutral
- The tool inventory becomes 6 MCP tools alongside the existing slash commands.

## Revisit if
- Maintaining both the MCP and slash-command paths becomes a burden.
- A non-bash environment needs to launch the server.
