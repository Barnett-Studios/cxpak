---
id: '0037'
title: Plugin lives in-repo and is versioned/released lockstep with the CLI
status: ACCEPTED
date: 2026-03-11
triggered_by: CLI flag changes could silently break a separately-distributed plugin
loop: planning
---

# ADR-0037: Plugin lives in-repo and is versioned/released lockstep with the CLI

## Context

The Claude Code plugin invokes the cxpak CLI with specific flags — `overview --tokens --format`, `diff --git-ref`, `trace --all`. If the plugin were distributed separately from the CLI, a CLI flag change in one release could silently break a plugin pinned to an older shape, with no compile-time link between the two.

The v0.4.0 design co-locates the plugin in the cxpak repo's `plugin/` directory and ships it in the same release as the CLI, so any CLI flag change updates the plugin in the same commit. Users install by pointing Claude Code at the plugin directory.

## Options considered

- **Option A — In-repo, lockstep-versioned plugin:** `plugin/` directory inside the cxpak repo, released with the CLI; flag changes patched in the same commit. Pro: plugin and CLI flags never drift, and there is a single release process. Con: the plugin cannot version independently of the CLI. Someone could prefer this because it eliminates an entire class of version-skew bugs without extra coordination. (Considered and chosen.)
- **Option B — Separate plugin repo / marketplace package:** A reasonable alternative would have been to distribute the plugin independently in its own repository or marketplace package. Pro: independent release cadence — the plugin could ship fixes without a CLI release. Con: reintroduces flag-drift risk between plugin and CLI versions, which is exactly the failure this decision avoids. Someone could prefer it if the plugin developed a release tempo materially different from the CLI's.

## Decision

Keep the plugin in the cxpak repo's `plugin/` directory, versioned and released alongside the CLI. When a CLI flag changes, update the plugin in the same commit. Installation is by adding the plugin directory to Claude Code's plugin configuration.

The plugin version is held in lockstep with the crate across `plugin/.claude-plugin/plugin.json` and `.claude-plugin/marketplace.json` (both 2.2.1 at time of inspection, matching the crate version).

## Consequences

### Positive
- No flag drift between plugin and CLI.
- Single source of truth and a single release process.

### Negative
- The plugin cannot be versioned independently of the CLI.

### Neutral
- The shipped repo carries a `.claude-plugin/marketplace.json` listing and a `plugin/.claude-plugin/plugin.json`, both kept in lockstep with the crate version (2.2.1 at time of inspection).

### Enforcement (added after #91)

Lockstep was prose until `v3.1.4` shipped with `Cargo.toml` at `3.1.4` and every other
declaration at `3.1.3`, so `ensure-cxpak` rejected the binary its own release published — the MCP
server, four commands, two skills and both git hook wrappers with it. The cost of Option A was
paid and its benefit was not received.

`tests/version_lockstep.rs` now asserts it. Two things worth recording here rather than only in
the test:

- **There are three declarations, not the two this ADR names.** `plugin/lib/ensure-cxpak`'s
  `REQUIRED_VERSION` postdates this decision (ADR-0033 made it the single binary resolver) and is
  the one that actually refuses to run. It is the site the ADR's own file list would have let you
  miss.
- **The list is enumerated from this document, not discovered by scanning.** A check that finds
  its own denominator can only confirm that the files it happened to find agree with each other.
  So a fourth declaration means editing that list — here and in the test.

## Revisit if
- An independent plugin release cadence becomes necessary (e.g., plugin-only fixes that should not wait on a CLI release).
