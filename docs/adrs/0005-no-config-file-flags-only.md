---
id: '0005'
title: Configuration via CLI flags only, no config file
status: ACCEPTED
date: 2026-03-05
triggered_by: Deciding how users configure cxpak behavior (budget, format, output)
loop: planning
---

# ADR-0005: Configuration via CLI flags only, no config file

## Context

In v0.1.0, cxpak needs a way for users to configure its behavior — token budget, output destination, format, verbosity. The design keeps configuration entirely in CLI flags (`--tokens`, `--out`, `--format`, `--verbose`) and explicitly lists "Config file (flags only)" under Not In Scope, keeping the tool stateless with no config-discovery logic.

## Options considered

- **Option A — Flags only:** All options are passed on the command line. Pros: simple, explicit, stateless, and no config-file discovery or precedence logic to maintain. Cons: repetitive for fixed preferences, and no persistent per-project settings. Someone could prefer this for the predictability of fully command-line-determined behavior.
- **Option B — Config file + flags:** Support a `.cxpak` config with flag overrides. Pros: persistent project defaults so users don't repeat themselves. Cons: config discovery and merge/precedence complexity, and it was declared out of scope at design time. Someone could prefer this for project-level convenience — and this is effectively the direction shipped code later took.

## Decision

Configure cxpak exclusively through CLI flags with no config file, keeping the tool stateless and explicit.

## Consequences

### Positive
- No config discovery or precedence logic; behavior is fully determined by the command line.

### Negative
- The flags-only stance was later reversed. Shipped code reads `.cxpak.json` / `.cxpak/` configuration for embeddings providers (`.cxpak.json`), the plugins manifest (`.cxpak/plugins.json`), the conventions baseline (`.cxpak/conventions.json`), and more.

### Neutral
- The `--tokens` flag (with a k/m-suffix parser, `parse_token_count`) is the central knob; the design doc specified it as required, though shipped code defaults it to `50k`.

## Revisit if
- Users need persistent per-project settings (they later did, for embeddings/plugins/conventions, reversing this decision).
