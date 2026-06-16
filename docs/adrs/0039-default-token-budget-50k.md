---
id: '0039'
title: Default --tokens to 50k across all commands
status: ACCEPTED
date: 2026-03-12
triggered_by: 'First-run friction: every command errored without --tokens'
loop: planning
---

# ADR-0039: Default --tokens to 50k across all commands

## Context

In v0.7.0 all commands required `--tokens`, so `cxpak overview` failed for first-time users who had not yet learned the flag. The required flag was the most common first-run friction point. A sensible default removes it while keeping explicit overrides available.

## Options considered

- **Option A — Default to 50k:** Make `--tokens` optional with `default_value = "50k"` on `overview`, `diff`, and `trace`. Pro: fits Claude's 200k context with room for conversation, is large enough to be useful for most repos, does not overwhelm smaller models, and matches the plugin's existing default. Con: a hidden default may surprise users who expected an error or a different size. Someone could prefer this because it makes the tool work with zero flags. (Considered and chosen.)
- **Option B — Keep `--tokens` required:** Leave the flag mandatory. Pro: forces an explicit budget decision every run. Con: the first-run friction that prompted this change — a common complaint. Someone could prefer it to keep budget always intentional. (Considered.)
- **Option C — Default to a different budget (e.g. 30k or 100k):** A reasonable alternative would have been a smaller or larger default. Pro: 30k is more PR-readable; 100k is more complete. Con: neither is aligned with the plugin default; 50k was chosen as the balance point. Someone could prefer 30k for tighter PR comments or 100k for fuller context.

## Decision

Make `--tokens` optional with `default_value = "50k"` on `overview`, `diff`, and `trace`, while explicit overrides like `--tokens 100k` still work. 50k was chosen to fit Claude's 200k context, be useful for most repos, not overwhelm smaller models, and match the plugin's existing default.

## Consequences

### Positive
- `cxpak overview` works with no flags.
- Consistent default with the Claude Code plugin.

### Negative
- An implicit budget may not suit very large or very small repos.

### Neutral
- MCP tools (`overview`/`diff`/`trace`) mirror the same 50k default, keeping CLI and server behavior consistent.

## Revisit if
- Model context windows shift materially.
- Telemetry shows 50k is wrong for the median repo.
