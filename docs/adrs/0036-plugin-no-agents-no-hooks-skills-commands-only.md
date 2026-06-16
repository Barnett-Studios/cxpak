---
id: '0036'
title: Plugin surface limited to auto-invoked skills and user-invoked commands; no agents, no hooks
status: ACCEPTED
date: 2026-03-11
triggered_by: Need to expose cxpak's indexing inside Claude Code sessions without adding heavyweight extension surfaces
loop: planning
---

# ADR-0036: Plugin surface limited to auto-invoked skills and user-invoked commands; no agents, no hooks

## Context

The cxpak v0.4.0 plugin had to make cxpak's overview/trace/diff/clean available in-session. Claude Code plugins can ship skills (auto-invoked via YAML frontmatter description matching), commands (user-invoked `/plugin:name`), agents, and hooks. As designed on 2026-03-11, the surface was deliberately restricted to skills + commands only, relying on skill description matching for auto-invocation rather than hook interception or dedicated agents.

This ADR records the no-agents/no-hooks decision as made. That invariant still holds in the shipped tree, though the surface was subsequently extended (see the neutral consequence) with an MCP server and a third skill.

## Options considered

- **Option A — skills + commands only (chosen):** Two auto-invoked skills (codebase-context, diff-context) plus the `/cxpak:*` commands, sharing one `ensure-cxpak` script. Pros: minimal surface, no background interception, leverages built-in skill auto-discovery. Cons: auto-invocation depends entirely on description-match heuristics; no guaranteed pre-prompt injection. Someone could prefer this for its small, predictable footprint.
- **Option B — add PostToolUse/PreToolUse hooks:** Considered and rejected. Hooks would inject context deterministically on every prompt or tool use, but the design explicitly rules them out as too invasive. One could prefer this for guaranteed auto-context.
- **Option C — dedicated subagents:** Considered and rejected. A context-gathering agent invoked by the orchestrator would offer encapsulation but is heavier. One could prefer it to isolate context-gathering logic.

## Decision

Ship skills, commands, and (later) an MCP server only — no agents and no hooks. Skills handle auto-invocation via their description frontmatter; `lib/ensure-cxpak` is shared by all of them. The no-hooks/no-agents invariant is enforced by structural tests asserting the absence of `hooks/` and `agents/` directories.

## Consequences

### Positive
- Smallest practical plugin surface.
- Auto-invocation handled by native skill discovery.
- Tested invariant (no `hooks/`/`agents/` dirs) prevents scope creep.

### Negative
- No guaranteed pre-prompt context injection; auto-context depends on Claude choosing to invoke the skill.

### Neutral
- Auto-context relies on description matching rather than deterministic hook injection.
- The surface was subsequently extended with an MCP server (`.mcp.json` -> `lib/ensure-cxpak-serve`) and a third `setup` skill, while the no-agents/no-hooks invariant was preserved.

## Revisit if
- Skill description matching proves unreliable for auto-invocation.
- A need arises for deterministic per-prompt context injection.
