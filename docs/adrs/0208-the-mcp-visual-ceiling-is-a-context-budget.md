---
id: '0208'
title: The MCP `visual` ceiling is a context budget, not a transport limit
status: ACCEPTED
date: 2026-09-03
triggered_by: cxpak#62 — 340 KB dashboard returned inline from a three-file repository
loop: implementation
supersedes: '0135'
---

# ADR-0208: The MCP `visual` ceiling is a context budget, not a transport limit

## Context

Supersedes [ADR-0135](0135-mcp-html-inline-1mb-spill-to-disk.md), which shipped the
spill-to-disk mechanism this keeps and the threshold this replaces.

0135 opens by naming the right hazard:

> Returning large self-contained HTML inline through the MCP tool channel can blow
> **context/transport** limits.

Then it picks a number for the second word. `MCP_INLINE_LIMIT = 1_048_576` is a transport
figure; a context window is three orders of magnitude smaller in the units that matter.

cxpak#62 measured the consequence on a **three-file** repository:

| op | bytes |
|---|---|
| `drift` | 57 |
| `health` | 154 |
| `security_surface` | 447 |
| `risks` | 744 |
| `architecture` | 1,009 |
| `onboard` | 1,102 |
| `conventions` | 3,916 |
| **`visual`** | **340,432** |

~90% of that payload is an inlined d3 bundle. It is 32× under 0135's ceiling, so the guard
was working exactly as designed and the caller lost most of a context window anyway.

0135 also lists this under **Neutral**:

> The spill check is gated strictly on `format == "html"`.

It is not neutral. Seven other formats are reachable through the same op, and `graphml`,
`json` and `cypher` over a large graph are not small. None of them was bounded at all.

**Its revisit conditions could not fire.** They are *"MCP transport limits change"* and
*"inline vs path handling confuses tool consumers"*. The failure that happened is neither: a
payload well **under** the threshold exhausting a context window is invisible to both. A
guard whose trigger conditions cannot describe its own failure mode is the shape ADR-0060 is
about, one layer out.

## Decision

**The ceiling is measured in tokens, against a caller-supplied budget, for every format.**

- `MAX_MCP_VISUAL_TOKENS` = `MAX_MCP_CONVENTIONS_TOKENS` (5,000). Not a new number: `visual`
  is one entry in a menu of eight ops, and an agent picking it off that list should not pay
  two orders of magnitude more than its neighbours cost.
- `tokens` is honoured, parsed the same way `conventions` parses it. A caller that wants the
  whole artifact inline says so. Before this there was no such lever on this op.
- The spill applies to **all** formats, and the file gets the extension of what was written —
  a mermaid graph saved as `.html` is a file nothing will open.
- The spill result is JSON — `{path, bytes, tokens, token_budget, format, note}` — because
  the caller has to decide what to do next and a prose sentence makes it guess.

**Spill, not truncate.** Every format here is structured, and half a structured document is
not a smaller document, it is a broken one. `conventions` truncates because its payload
degrades gracefully; this one writes the whole artifact and returns where it went. That also
matches what the CLI's `--out` says the command is for.

## Consequences

### Positive
- The op costs what the caller allowed, in the units the caller is actually spending.
- Seven previously unbounded formats are bounded.
- The default is shared with a sibling op, so drift between them is one assertion away
  (`the_visual_budget_matches_the_other_capped_op`).

### Negative
- A caller who previously got 340 KB inline now gets a path. That is the point, but it is a
  behaviour change on a published MCP surface for anyone who was relying on it.
- The token count is computed over the whole rendered payload, so a large dashboard is still
  fully rendered before being written. This bounds what the caller *receives*, not what the
  server *builds*.

### Neutral
- 0135's spill mechanism, its `.cxpak/visual/` location and its path-escape check are all
  kept unchanged. What changed is the number, the units, and the set of formats it covers.

## Revisit if
- A caller needs an artifact **not** written to the repo — the destination is still
  hardcoded to `.cxpak/visual/` and there is no `out` parameter (0135's unshipped
  `mcp_inline_limit_bytes` configurability has an echo here).
- Per-op input schemas land (cxpak#45). `tokens` is honoured but not advertised, because
  `additionalProperties: true` and per-op parameter documentation is that issue's subject,
  not this one's.
- The rendering cost itself, rather than the return payload, becomes the constraint.
