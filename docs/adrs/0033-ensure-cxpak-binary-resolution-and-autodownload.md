---
id: '0033'
title: Shared ensure-cxpak bash script resolves the binary: PATH, then cached install, then auto-download
status: ACCEPTED
date: 2026-03-11
triggered_by: Plugin needs the cxpak binary present, but users may not have installed it
loop: planning
---

# ADR-0033: Shared ensure-cxpak bash script resolves the binary: PATH, then cached install, then auto-download

## Context

Every skill and command in the cxpak v0.4.0 Claude Code plugin needs a cxpak binary. Rather than require manual installation, a single bash resolver (`lib/ensure-cxpak`) is the foundation all entry points call.

The design specifies a three-step resolution: check `PATH`; check a previously-downloaded cache at `~/.claude/plugins/cxpak/bin/cxpak`; otherwise detect OS + arch via `uname` and download. The implementation plan codifies this with a `--dry-run` flag for testability and trap-based tmpdir cleanup.

## Options considered

- **Option A — three-step resolver script: PATH -> cache -> download (chosen):** One bash script returns a binary path, downloading if neither PATH nor cache has it. Pros: a single shared dependency, zero-friction first use, testable via dry-run. Cons: couples the plugin to a network download path and exposes a bash portability surface. Someone could prefer this for the zero-setup first run.
- **Option B — require manual install as a prerequisite:** A reasonable alternative would have been documenting `brew`/`cargo install` and failing if cxpak is not on `PATH`. Simpler, with no download logic, but friction on first use. One could prefer it to avoid owning a download path.
- **Option C — bundle the binary in the plugin:** A reasonable alternative would have been shipping platform binaries inside the plugin directory to avoid any download. Rejected because it bloats the repo and creates a multiplatform packaging burden; someone might prefer it for fully offline installs.

## Decision

Implement `lib/ensure-cxpak` as the single binary resolver shared by all skills/commands, resolving in order: cxpak on `PATH`, then a cached install at the install dir, then a platform-detected download. Add a `--dry-run` mode that prints the resolved target/URL without downloading, used heavily by the BATS tests. `PATH` always wins over the cached copy.

## Consequences

### Positive
- Zero-friction first use.
- One place to change binary resolution.
- Dry-run makes platform detection unit-testable without network.

### Negative
- The original GitHub-tarball download path was later replaced; the shipped `ensure-cxpak` now installs via Homebrew/cargo and pins `REQUIRED_VERSION` rather than downloading raw release tarballs.

### Neutral
- The install cache lives under `~/.claude/plugins/cxpak/bin` (overridable via `CXPAK_INSTALL_DIR`).

## Revisit if
- The distribution channel changes.
- Cached-vs-PATH precedence causes stale-version bugs.
